//! The WebDAV provider: Nextcloud, ownCloud, sabre/dav, Apache's `mod_dav`,
//! and anything else that speaks the protocol.
//!
//! SPLIT PURE FROM IMPURE, for the reason
//! `src-tauri/src/modules/sync/providers/s3/mod.rs` states in its own header
//! and that has not changed: this crate has no `[dev-dependencies]` and
//! therefore no HTTP mock, so a decision that lives only inside an async method
//! body is expensive to test. Everything that decides anything - which URL,
//! which headers, which error, which rows - is a plain function taking values
//! and returning values, and what is left is the method bodies plus a send. The
//! one sequence that is not a pure decision, the create's collection ladder, is
//! driven at the shell against a loopback listener built from the standard
//! library.
//!
//! This is a directory module: the request builders live in
//! `src-tauri/src/modules/sync/providers/webdav/build.rs`, the status mapping in
//! `src-tauri/src/modules/sync/providers/webdav/classify.rs`, the multi-status
//! scan in `src-tauri/src/modules/sync/providers/webdav/dav_xml.rs`, the date
//! reader in `src-tauri/src/modules/sync/providers/webdav/dav_time.rs`, and the
//! HTTP shell they all send through is shared in
//! `src-tauri/src/modules/sync/providers/http.rs`.
//!
//! NO CONDITIONAL `PUT`, AND ONE OPPORTUNISTIC CREATE. The trait's `cas` method
//! is left at its `false` default rather than answered from a configuration
//! field, because there is no protocol guarantee here for a user to know the
//! answer to: a WebDAV server may or may not honour a conditional write and
//! nothing in the protocol says which. The trait's own contract makes that safe
//! to express by omission - a backend that cannot honour `if_match` degrades to
//! last-write-wins - so `put` takes the argument and binds it to `_`, and no
//! request builder behind it accepts an etag at all.
//!
//! The one conditional request is the create. [`build_put_if_absent`] always
//! sends `if-none-match: *` and [`SyncProvider::put_if_absent`] reads a 412 as
//! "an object is already there", which narrows the window in which two devices
//! both mint a keyfile. It is opportunistic rather than a guarantee: a server
//! that ignores the header stores the second keyfile over the first, and the
//! device that minted first then holds a root key the stored keyfile no longer
//! opens. That residue is the trait's documented one for a backend without a
//! usable conditional write, and it is cheaper than refusing a create on every
//! server that cannot say which it is.
//!
//! A COLLECTION HAS TO EXIST BEFORE ANYTHING CAN BE STORED IN IT, which is the
//! one structural difference from the other backend. S3 has no directories, so
//! the first write of a fresh remote succeeds; a WebDAV server refuses a PUT
//! whose parent collection does not exist and answers 409. Since the very
//! first thing the layer above does with a fresh remote is mint a keyfile, that
//! 409 is not an edge case - it is the ordinary first run. So `put` creates the
//! missing ancestors, shallowest first, and retries exactly once. A 409 that
//! survives that retry is a parent that is not a collection at all, and it is
//! reported as a refusal naming the parent rather than as the retry-and-hope
//! disposition a bare conflict carries.
//!
//! REDIRECTS ARE REFUSED. The client follows none, which matters more here than
//! it does for a signed protocol: this one puts the user's password in every
//! request, in a reversible encoding, and a server that redirects a plain-HTTP
//! request to somewhere else would otherwise have that password sent to
//! whatever host it named. A refused redirect arrives as an ordinary response
//! carrying a 3xx status - the client does not error on one - so it is caught
//! by status rather than by a transport branch.
//!
//! ONE `Content-Type` IS SENT, on the listing only. The other backend sends
//! none on any verb because a present content type has to be signed there;
//! nothing here signs anything, so that reason does not carry over and the two
//! cases are decided on their own merits. A listing's request body is XML and a
//! server is entitled to refuse an unlabelled one. A stored object is a sealed
//! envelope, which has no meaningful media type and whose type nothing on the
//! sync path ever reads back, so a put sends none and lets the server pick.

use std::future::Future;
use std::pin::Pin;

use serde::Deserialize;

use super::http;
use crate::modules::sync::provider::{Entry, Object, ProviderError, SyncProvider};

mod build;
mod classify;
mod dav_time;
mod dav_xml;

/// `normalize_etag` lived at this path before the split and now lives in the
/// shared shell, so it is re-exported here rather than moved under a caller's
/// feet.
pub use super::http::normalize_etag;
pub use build::{
    ancestors, basic_auth, build_delete, build_get, build_mkcol, build_propfind, build_put,
    build_put_if_absent, object_url, Endpoint,
};
pub use classify::{
    classify, classify_delete, classify_get, classify_list, classify_mkcol, classify_put,
    classify_put_if_absent, transport_error, PutIfAbsent,
};
pub use dav_time::parse_http_date;
pub use dav_xml::parse_multistatus;

/// Asking for three properties by name rather than for all of them, so the
/// response is three elements per row instead of whatever the server chooses to
/// keep about a file.
const PROPFIND_BODY: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"utf-8\"?>",
    "<propfind xmlns=\"DAV:\"><prop>",
    "<getetag/><getlastmodified/><resourcetype/>",
    "</prop></propfind>"
);

/// Everything the provider needs, and nothing it can supply for the user.
///
/// THE SAME FOUR PROPERTIES the other backend's configuration justifies, for
/// the same reasons: no `Default` impl, so no code path can supply an endpoint
/// the user did not type; no derived `Debug`, so `password` cannot reach a log
/// line or a panic message; camelCase field names because this arrives as JSON
/// from the frontend; and no unknown fields, so a stale or misspelled key is a
/// loud error at the first call rather than a silently defaulted one.
///
/// NO `cas` FIELD, which is that last property doing work rather than an
/// omission. A frontend that sends one - because it was copied from the other
/// provider's form, say - is refused by name instead of having the key quietly
/// ignored while the user believes conditional writes are on.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebDavConfig {
    /// Typed by the user: the collection their files hang off, which on a
    /// typical server is a per-user path under a dav mount point.
    pub endpoint: String,
    pub username: String,
    pub password: String,
}

// --- the shell ------------------------------------------------------------

pub struct WebDavProvider {
    /// The endpoint as the user typed it, which is what the guard below reads.
    endpoint_url: String,
    endpoint: Endpoint,
    /// Computed once at construction, so the password is not carried around in
    /// a second, plainer form for the life of the provider.
    auth: String,
    client: reqwest::Client,
    /// Set once the endpoint has PASSED the guard, and never otherwise.
    /// Only the pass is cached, for the reason the shared shell in
    /// `src-tauri/src/modules/sync/providers/http.rs` states at `ensure_allowed`.
    endpoint_allowed: tokio::sync::OnceCell<()>,
}

impl WebDavProvider {
    pub fn new(cfg: WebDavConfig) -> Result<Self, ProviderError> {
        // Fail here rather than at the first request, so a provider that exists
        // is one that can address something.
        let endpoint = build::endpoint(&cfg)?;
        Ok(Self {
            auth: basic_auth(&cfg.username, &cfg.password),
            endpoint_url: cfg.endpoint,
            endpoint,
            client: http::build_client()?,
            endpoint_allowed: tokio::sync::OnceCell::new(),
        })
    }

    /// Walk `key`'s parent collections, shallowest first, creating each.
    ///
    /// ONE RUNG AT A TIME because a server will not create the intermediate
    /// collections of a deep request for you; the ladder is shared by `put` and
    /// `put_if_absent` so both walk it identically.
    async fn ensure_ancestors(&self, key: &str) -> Result<(), ProviderError> {
        for collection in ancestors(key) {
            let made = http::send(
                &self.client,
                build_mkcol(&self.endpoint, &self.auth, &collection),
                None,
            )
            .await?;
            classify_mkcol(made.status, &collection)?;
        }
        Ok(())
    }
}

impl SyncProvider for WebDavProvider {
    fn id(&self) -> &'static str {
        "webdav"
    }

    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Object>, ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            http::ensure_allowed(&self.endpoint_allowed, &self.endpoint_url).await?;
            let raw = http::send(
                &self.client,
                build_get(&self.endpoint, &self.auth, key),
                None,
            )
            .await?;
            if classify_get(raw.status)?.is_none() {
                return Ok(None);
            }
            // SYMMETRIC WITH `put`, which refuses the same absence. An empty
            // string is not an etag, and handing one back would have the row
            // match itself on every later pull.
            let etag = raw.etag.ok_or_else(|| {
                ProviderError::Malformed("the remote returned an object with no etag".to_string())
            })?;
            Ok(Some(Object {
                etag,
                bytes: raw.body,
            }))
        })
    }

    /// `_if_match` is taken and dropped. See [`build_put`]: this backend cannot
    /// express a condition, reports that through `cas`, and degrades
    /// to last-write-wins rather than failing a caller that passes one.
    fn put<'a>(
        &'a self,
        key: &'a str,
        bytes: Vec<u8>,
        _if_match: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            http::ensure_allowed(&self.endpoint_allowed, &self.endpoint_url).await?;
            let first = http::send(
                &self.client,
                build_put(&self.endpoint, &self.auth, key),
                Some(bytes.clone()),
            )
            .await?;
            let stored = match classify_put(first.status, key, false)? {
                Some(()) => first,
                None => {
                    // The parents are missing, which is the ordinary first write
                    // against a fresh remote.
                    self.ensure_ancestors(key).await?;
                    let again = http::send(
                        &self.client,
                        build_put(&self.endpoint, &self.auth, key),
                        Some(bytes.clone()),
                    )
                    .await?;
                    classify_put(again.status, key, true)?;
                    again
                }
            };
            stored.etag.ok_or_else(|| {
                ProviderError::Malformed(
                    "the remote stored the object but returned no etag".to_string(),
                )
            })
        })
    }

    /// A create: the object is written only when the key is empty.
    ///
    /// `Ok(None)` MEANS AN OBJECT IS ALREADY THERE and nothing changed, which
    /// the caller has to act on rather than retry: another device minted the
    /// keyfile first, and the copy now stored is the one this device has to
    /// open. No collection is created on that path - the object a collection
    /// would be created for is already sitting there.
    fn put_if_absent<'a>(
        &'a self,
        key: &'a str,
        bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            http::ensure_allowed(&self.endpoint_allowed, &self.endpoint_url).await?;
            // A stored object answers with its etag or not at all, the same
            // rule `get` and `put` apply.
            let stored = |raw: http::RawResponse| -> Result<Option<String>, ProviderError> {
                raw.etag.map(Some).ok_or_else(|| {
                    ProviderError::Malformed(
                        "the remote stored the object but returned no etag".to_string(),
                    )
                })
            };
            let first = http::send(
                &self.client,
                build_put_if_absent(&self.endpoint, &self.auth, key),
                Some(bytes.clone()),
            )
            .await?;
            match classify_put_if_absent(first.status, key, false)? {
                PutIfAbsent::Exists => Ok(None),
                PutIfAbsent::Stored => stored(first),
                PutIfAbsent::NeedsParents => {
                    // The same ladder `put` walks, then exactly one more
                    // attempt with the conditional header still set.
                    self.ensure_ancestors(key).await?;
                    let again = http::send(
                        &self.client,
                        build_put_if_absent(&self.endpoint, &self.auth, key),
                        Some(bytes.clone()),
                    )
                    .await?;
                    match classify_put_if_absent(again.status, key, true)? {
                        PutIfAbsent::Exists => Ok(None),
                        // A retried attempt cannot report `NeedsParents`: the
                        // classifier turns the second 409 into a refusal above,
                        // so the only answer it can still give is that the
                        // object was stored.
                        PutIfAbsent::Stored | PutIfAbsent::NeedsParents => stored(again),
                    }
                }
            }
        })
    }

    /// ONE REQUEST, WITH NO PAGINATION TO FOLLOW. This protocol has no
    /// continuation token, so there is no page to accumulate and no
    /// never-terminating listing to guard against.
    fn list<'a>(
        &'a self,
        prefix: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Entry>, ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            http::ensure_allowed(&self.endpoint_allowed, &self.endpoint_url).await?;
            let raw = http::send(
                &self.client,
                build_propfind(&self.endpoint, &self.auth, prefix),
                Some(PROPFIND_BODY.as_bytes().to_vec()),
            )
            .await?;
            if classify_list(raw.status)?.is_none() {
                return Ok(Vec::new());
            }
            parse_multistatus(
                &raw.body,
                &build::object_path(&self.endpoint, prefix),
                prefix,
            )
        })
    }

    fn delete<'a>(
        &'a self,
        key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            http::ensure_allowed(&self.endpoint_allowed, &self.endpoint_url).await?;
            let raw = http::send(
                &self.client,
                build_delete(&self.endpoint, &self.auth, key),
                None,
            )
            .await?;
            classify_delete(raw.status)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    use super::*;

    fn cfg() -> WebDavConfig {
        WebDavConfig {
            endpoint: "https://cloud.example/remote.php/dav/files/rendi".to_string(),
            username: "rendi".to_string(),
            password: "hunter2".to_string(),
        }
    }

    // --- the conditional create, over a loopback server -------------------

    /// One scripted answer, popped per connection.
    struct Reply {
        status: u16,
        etag: Option<&'static str>,
    }

    /// One request as the stub saw it.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct Asked {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
    }

    impl Asked {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        }
    }

    /// A throwaway HTTP server on loopback.
    ///
    /// LOOPBACK because the guard in `super::reject_metadata_ssrf` allows it,
    /// and a plain std listener on its own thread because this crate's tokio
    /// features carry no `net`. It answers each connection with the next
    /// scripted reply and records what it was asked, which is the only way the
    /// shell's request sequence - the collection ladder, and whether the retry
    /// kept its header - can be asserted without an HTTP mock.
    struct Stub {
        base: String,
        asked: Arc<Mutex<Vec<Asked>>>,
    }

    impl Stub {
        fn start(replies: Vec<Reply>) -> Stub {
            let listener = TcpListener::bind("127.0.0.1:0").expect("loopback binds");
            let addr = listener.local_addr().expect("an address");
            let base = format!("http://{addr}");
            let asked = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&asked);
            let mut queue: VecDeque<Reply> = replies.into();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let reply = queue.pop_front().unwrap_or(Reply {
                        status: 200,
                        etag: None,
                    });
                    if let Ok(mut log) = sink.lock() {
                        log.push(read_request(&mut stream));
                    }
                    let (status, phrase) = (reply.status, reason(reply.status));
                    let mut head = format!(
                        "HTTP/1.1 {status} {phrase}\r\ncontent-length: 0\r\nconnection: close\r\n"
                    );
                    if let Some(etag) = reply.etag {
                        head.push_str(&format!("etag: \"{etag}\"\r\n"));
                    }
                    head.push_str("\r\n");
                    let _ = stream.write_all(head.as_bytes());
                }
            });
            Stub { base, asked }
        }

        fn provider(&self) -> WebDavProvider {
            WebDavProvider::new(WebDavConfig {
                endpoint: self.base.clone(),
                username: "rendi".to_string(),
                password: "hunter2".to_string(),
            })
            .expect("the stub endpoint parses")
        }

        fn asked(&self) -> Vec<Asked> {
            self.asked.lock().expect("the log is not poisoned").clone()
        }
    }

    fn reason(status: u16) -> &'static str {
        match status {
            201 => "Created",
            409 => "Conflict",
            412 => "Precondition Failed",
            _ => "OK",
        }
    }

    /// One request off the wire: its head, and then the body it declared.
    ///
    /// THE BODY IS READ RATHER THAN IGNORED, because a client still writing when
    /// the connection closes reports a transport failure instead of the status
    /// this stub sent, which would make every assertion below a flake.
    fn read_request(stream: &mut TcpStream) -> Asked {
        // A bounded read so a client that never completes its request fails
        // the test instead of hanging the stub thread forever.
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
        let mut buf = Vec::new();
        let mut head_end = None;
        let mut chunk = [0u8; 1024];
        while head_end.is_none() {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
            head_end = buf.windows(4).position(|w| w == b"\r\n\r\n");
        }
        let head = String::from_utf8_lossy(&buf[..head_end.unwrap_or(buf.len())]).to_string();
        let mut lines = head.split("\r\n");
        let mut start = lines.next().unwrap_or("").split_whitespace();
        let method = start.next().unwrap_or("").to_string();
        let path = start.next().unwrap_or("").to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(": "))
            .map(|(n, v)| (n.to_ascii_lowercase(), v.to_string()))
            .collect();
        let declared: usize = headers
            .iter()
            .find(|(n, _)| n == "content-length")
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let body_read = buf.len() - head_end.map_or(0, |end| end + 4);
        let mut left = declared.saturating_sub(body_read);
        while left > 0 {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => left = left.saturating_sub(n),
            }
        }
        Asked {
            method,
            path,
            headers,
        }
    }

    #[tokio::test]
    async fn a_412_reports_the_object_exists_and_makes_no_collection() {
        let stub = Stub::start(vec![Reply {
            status: 412,
            etag: None,
        }]);
        let provider = stub.provider();
        let answer = provider
            .put_if_absent("vault/v1/keyfile", b"sealed".to_vec())
            .await
            .expect("a refused create is an answer, not a failure");
        assert_eq!(answer, None);
        let asked = stub.asked();
        assert_eq!(
            asked.len(),
            1,
            "a refused create sent more than the PUT: {asked:?}"
        );
        assert_eq!(asked[0].method, "PUT");
        assert_eq!(asked[0].path, "/vault/v1/keyfile");
        assert_eq!(asked[0].header("if-none-match"), Some("*"));
    }

    #[tokio::test]
    async fn the_mkcol_ladder_still_runs_on_409() {
        let stub = Stub::start(vec![
            Reply {
                status: 409,
                etag: None,
            },
            Reply {
                status: 201,
                etag: None,
            },
            Reply {
                status: 201,
                etag: None,
            },
            Reply {
                status: 201,
                etag: None,
            },
            Reply {
                status: 201,
                etag: Some("fresh"),
            },
        ]);
        let provider = stub.provider();
        let answer = provider
            .put_if_absent("vault/v1/obj/ab12", b"sealed".to_vec())
            .await
            .expect("the ladder discharges the conflict");
        assert_eq!(answer.as_deref(), Some("fresh"));
        let asked = stub.asked();
        let shape: Vec<(String, String)> = asked
            .iter()
            .map(|a| (a.method.clone(), a.path.clone()))
            .collect();
        // The key names three collections under the endpoint's own path, so
        // the ladder walks all three shallowest first.
        assert_eq!(
            shape,
            vec![
                ("PUT".to_string(), "/vault/v1/obj/ab12".to_string()),
                ("MKCOL".to_string(), "/vault".to_string()),
                ("MKCOL".to_string(), "/vault/v1".to_string()),
                ("MKCOL".to_string(), "/vault/v1/obj".to_string()),
                ("PUT".to_string(), "/vault/v1/obj/ab12".to_string()),
            ]
        );
        // The retry is still a create, not a plain overwrite.
        assert_eq!(
            asked[4].header("if-none-match"),
            Some("*"),
            "the retry dropped the header"
        );
    }

    // --- the guard --------------------------------------------------------

    #[tokio::test]
    async fn the_metadata_service_is_refused_before_a_socket_is_opened() {
        // Offline by construction: the first is an IP literal, so resolving it
        // opens nothing, and the second is refused by name before resolution.
        for endpoint in [
            "http://169.254.169.254/dav",
            "http://metadata.google.internal/dav",
        ] {
            let mut cfg = cfg();
            cfg.endpoint = endpoint.to_string();
            let provider = WebDavProvider::new(cfg).expect("the endpoint parses");
            let err = provider
                .get("vault/v1/keyfile")
                .await
                .expect_err("the guard must refuse");
            assert!(
                matches!(err, ProviderError::Blocked(_)),
                "{endpoint} gave {err:?}"
            );
        }
    }

    // --- what this module ships -------------------------------------------

    /// Every shipped line of the WebDAV module: each of its files up to its own
    /// test module.
    ///
    /// One assumption, stated because it would otherwise be silent AND checked
    /// because stating it is not enough: a test module is each file's last
    /// item, so splitting at the first configuration attribute leaves exactly
    /// the shipped half. A test-only helper marked anywhere above it would
    /// shrink the scanned region and every needle below would pass over almost
    /// nothing.
    ///
    /// Checked by NAMING THE LAST SHIPPED ITEM rather than by a byte floor. A
    /// floor is a guess that has to be revised whenever either half grows, and
    /// it answers "is this big enough" when the question is "does this reach
    /// the end". The trait implementation is this file's last shipped item, so
    /// a split that lands before it loses that name.
    fn shipped() -> String {
        [
            include_str!("mod.rs"),
            include_str!("build.rs"),
            include_str!("classify.rs"),
            include_str!("dav_time.rs"),
            include_str!("dav_xml.rs"),
        ]
        .iter()
        .map(|source| {
            source
                .split_once("#[cfg(test)]")
                .map_or(*source, |(shipped, _)| shipped)
        })
        .collect()
    }

    #[test]
    fn this_module_ships_no_default_host_and_no_credential() {
        // A BACKSTOP OVER A GREP-ABLE SUBSET, not the guarantee. The guarantee
        // is that the configuration has no `Default` impl, which makes a
        // defaulted endpoint unrepresentable rather than merely absent today -
        // and that is the first needle.
        //
        // Deliberately NOT checking the names of particular self-hosted
        // servers: they appear in truthful prose about what this provider
        // supports. A registrable top-level domain does not, because every
        // address in this provider's prose uses a reserved one.
        let shipped = shipped();
        assert!(
            shipped.contains("impl SyncProvider for WebDavProvider"),
            "the split landed before the last shipped item, so the scan does not reach the end"
        );
        for needle in ["impl Default", ".com", ".net", ".org", ".io"] {
            assert!(
                !shipped.contains(needle),
                "the shipped half of this module carries {needle}"
            );
        }
    }
}
