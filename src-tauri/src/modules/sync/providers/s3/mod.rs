//! The S3 provider: Amazon S3 and the S3-compatible servers people self-host.
//!
//! SPLIT PURE FROM IMPURE, and that split is the spine of this module rather
//! than a tidiness preference. This crate has no `[dev-dependencies]` and
//! therefore no HTTP mock, so a behaviour that lives in an async method body
//! cannot be tested at all. Everything that decides anything - which URL, which
//! headers, which error, which page - is a plain function taking values and
//! returning values. What is left is five method bodies that call a builder,
//! send, and hand the result to a mapper.
//!
//! This is a directory module: the request builders live in
//! `src-tauri/src/modules/sync/providers/s3/build.rs`, the response mapping in
//! `src-tauri/src/modules/sync/providers/s3/classify.rs`, the listing scan in
//! `src-tauri/src/modules/sync/providers/s3/list_xml.rs`, and the HTTP shell
//! they all send through is shared in
//! `src-tauri/src/modules/sync/providers/http.rs`.
//!
//! Four things sit on the pure side that would otherwise hide in the shell:
//!
//! - [`transport_error`], the only producer of `ProviderError::Transport`, so
//!   the error taxonomy is reachable from a test without a network.
//! - [`classify_get`] and [`classify_put`] rather than one classifier: a
//!   function returning an error can never express "a missing key is the
//!   ordinary answer", so `get`'s 404 branch would have landed in the shell,
//!   which is the exact placement this split exists to prevent.
//! - [`accumulate`], which holds the append-and-continue decision for `list`
//!   including the case a server that repeats a continuation token forever
//!   falls into.
//! - the body decode. `reqwest` is built here without its `charset` feature, so
//!   `Response::text` takes its lossy branch and a non-UTF-8 body silently
//!   becomes replacement characters instead of failing. Every body is read as
//!   bytes and stays bytes; [`parse_list`] decodes strictly, and [`classify`]
//!   decodes lossily only in the one arm that reads an error code - so a
//!   successful fetch of a sealed envelope never allocates a mangled second
//!   copy of it.
//!
//! REDIRECTS ARE REFUSED, NOT RE-GUARDED. `reqwest` follows 3xx by default,
//! and for a SIGNED request following one is worse than an SSRF risk: `host`
//! is part of the canonical request, so a redirect to another host guarantees
//! a signature rejection that reads exactly like a bad secret key - the
//! failure mode the vectors in
//! `src-tauri/src/modules/sync/providers/sigv4.rs` exist to prevent. The client
//! here is built to follow none at all, which is strictly stronger, and a
//! region redirect is surfaced as a refusal naming the status.
//!
//! NO `Content-Type` IS SENT, on any verb including `put`. A typeless object
//! defaults to a binary media type, which is what a sealed envelope is, and
//! nothing on the sync path ever reads a content type back. A PRESENT
//! `Content-Type` must be signed, so sending one would add a fourth signed
//! header and a fourth way to get the canonical request wrong, for no reader.
//! The signer handles a signed content type regardless - one of its vectors
//! carries one - so this is a decision about what is sent, not a gap in what
//! can be signed.

use std::future::Future;
use std::pin::Pin;
use std::time::SystemTime;

use serde::Deserialize;

use super::http;
use crate::modules::sync::provider::{Caps, Entry, Object, ProviderError, SyncProvider};

mod build;
mod classify;
mod list_xml;

pub use build::{build_delete, build_get, build_list, build_put, build_put_if_absent};
pub use classify::{classify, classify_get, classify_put, classify_put_if_absent, transport_error};
pub use list_xml::{accumulate, parse_list};

/// `normalize_etag` lived at this path before the split and now lives in the
/// shared shell, so it is re-exported here rather than moved under a caller's
/// feet.
pub use super::http::normalize_etag;

/// Everything the provider needs, and nothing it can supply for the user.
///
/// FOUR PROPERTIES, EACH DELIBERATE:
///
/// - NO `Default` IMPL. Every field is required at construction, so there is no
///   code path that can supply an endpoint or a bucket the user did not type.
///   That makes a shipped default unrepresentable rather than merely
///   un-grepped, which is what the source-text test below is a backstop over
///   and not a substitute for.
/// - NO DERIVED `Debug`. A reflexive one puts `secret_access_key` into any log
///   line or panic message that formats the config. The same reason `SyncKeys`
///   in `src-tauri/src/modules/sync/crypto.rs` has none.
/// - CAMELCASE FIELD NAMES AND NO UNKNOWN FIELDS. This arrives as JSON from the
///   frontend, so the wire names are camelCase, and a renamed or misspelled
///   field is a loud error at the first call rather than a silently defaulted
///   one.
/// - NO `prefix` FIELD. The object layout belongs to the sync layer above the
///   provider, which composes the key a provider receives.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct S3Config {
    /// Typed by the user. Scheme and host, optionally a port and a base path
    /// for a server behind a reverse proxy.
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    /// Whether this endpoint honours a conditional write. A user toggle, not a
    /// probe - see `Caps` in `src-tauri/src/modules/sync/provider.rs`.
    pub cas: bool,
    pub access_key_id: String,
    pub secret_access_key: String,
}

pub struct S3Provider {
    cfg: S3Config,
    client: reqwest::Client,
    /// Set once the endpoint has PASSED the SSRF guard, and never otherwise.
    ///
    /// Only the pass is cached, for the reason the shared shell in
    /// `src-tauri/src/modules/sync/providers/http.rs` states at `ensure_allowed`:
    /// the guard resolves the host, so re-running it per object would put a DNS
    /// lookup in front of every request in an inventory-sized listing, and
    /// caching a failure would report one dropped network as a security
    /// refusal for the life of the process.
    endpoint_allowed: tokio::sync::OnceCell<()>,
}

impl S3Provider {
    pub fn new(cfg: S3Config) -> Result<Self, ProviderError> {
        // Fail here rather than at the first request, so a provider that exists
        // is one that can sign.
        build::endpoint(&cfg)?;
        Ok(Self {
            cfg,
            client: http::build_client()?,
            endpoint_allowed: tokio::sync::OnceCell::new(),
        })
    }
}

impl SyncProvider for S3Provider {
    fn id(&self) -> &'static str {
        "s3"
    }

    fn capabilities(&self) -> Caps {
        Caps { cas: self.cfg.cas }
    }

    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Object>, ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            http::ensure_allowed(&self.endpoint_allowed, &self.cfg.endpoint).await?;
            let req = build_get(&self.cfg, key, SystemTime::now())?;
            let raw = http::send(&self.client, req, None).await?;
            if classify_get(raw.status, &raw.body)?.is_none() {
                return Ok(None);
            }
            // SYMMETRIC WITH `put`, which refuses the same absence. An empty
            // string is not an etag, and handing one back would have the
            // caller send `If-Match:` with nothing after it - a condition that
            // fails every conditional write, silently and forever.
            let etag = raw.etag.ok_or_else(|| {
                ProviderError::Malformed("the remote returned an object with no etag".to_string())
            })?;
            Ok(Some(Object {
                etag,
                bytes: raw.body,
            }))
        })
    }

    fn put<'a>(
        &'a self,
        key: &'a str,
        bytes: Vec<u8>,
        if_match: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<String, ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            http::ensure_allowed(&self.endpoint_allowed, &self.cfg.endpoint).await?;
            // The same predicate `build_put` applies, through the same
            // function, so the mapper is told what was actually sent rather
            // than what was asked for.
            let conditional = build::condition(&self.cfg, if_match).is_some();
            let req = build_put(&self.cfg, key, &bytes, if_match, SystemTime::now())?;
            let raw = http::send(&self.client, req, Some(bytes)).await?;
            classify_put(raw.status, &raw.body, conditional)?;
            raw.etag.ok_or_else(|| {
                ProviderError::Malformed(
                    "the remote stored the object but returned no etag".to_string(),
                )
            })
        })
    }

    fn put_if_absent<'a>(
        &'a self,
        key: &'a str,
        bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            http::ensure_allowed(&self.endpoint_allowed, &self.cfg.endpoint).await?;
            // The same predicate `build_put_if_absent` applies, so the mapper
            // is told what was actually sent rather than what was asked for.
            let conditional = build::condition(&self.cfg, Some("*")).is_some();
            let req = build_put_if_absent(&self.cfg, key, &bytes, SystemTime::now())?;
            let raw = http::send(&self.client, req, Some(bytes)).await?;
            if classify_put_if_absent(raw.status, &raw.body, conditional)?.is_none() {
                return Ok(None);
            }
            // Symmetric with `put`: a stored object with no etag is refused
            // rather than handed back as the empty string.
            let etag = raw.etag.ok_or_else(|| {
                ProviderError::Malformed(
                    "the remote stored the object but returned no etag".to_string(),
                )
            })?;
            Ok(Some(etag))
        })
    }

    fn list<'a>(
        &'a self,
        prefix: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Entry>, ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            http::ensure_allowed(&self.endpoint_allowed, &self.cfg.endpoint).await?;
            let mut entries: Vec<Entry> = Vec::new();
            let mut token: Option<String> = None;
            loop {
                let req = build_list(&self.cfg, prefix, token.as_deref(), SystemTime::now())?;
                let raw = http::send(&self.client, req, None).await?;
                if !(200..300).contains(&raw.status) {
                    // Not `classify_get`: a missing BUCKET is a real failure,
                    // where a missing object is the ordinary answer.
                    return Err(classify(raw.status, &raw.body));
                }
                let page = parse_list(&raw.body)?;
                match accumulate(&mut entries, token.as_deref(), page)? {
                    None => return Ok(entries),
                    next => token = next,
                }
            }
        })
    }

    fn delete<'a>(
        &'a self,
        key: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), ProviderError>> + Send + 'a>> {
        Box::pin(async move {
            http::ensure_allowed(&self.endpoint_allowed, &self.cfg.endpoint).await?;
            let req = build_delete(&self.cfg, key, SystemTime::now())?;
            let raw = http::send(&self.client, req, None).await?;
            if (200..300).contains(&raw.status) {
                return Ok(());
            }
            // A delete answers with no content whether or not the key was
            // there, and a server that reports the absence instead means the
            // same thing: gone. A missing BUCKET does not - that is a
            // configuration error, and swallowing it would have every delete
            // against a mistyped bucket report success. `list` distinguishes
            // exactly the same pair.
            let missing_bucket = classify::error_code(&String::from_utf8_lossy(&raw.body))
                .is_some_and(|code| code == "NoSuchBucket");
            if raw.status == 404 && !missing_bucket {
                return Ok(());
            }
            Err(classify(raw.status, &raw.body))
        })
    }
}

#[cfg(test)]
mod test_support {
    use super::*;
    use crate::modules::sync::providers::http::Request;

    /// A fixed instant, so every signature below is reproducible.
    pub(super) fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs)
    }

    /// 2015-08-30T12:36:00Z, the same instant the signing vectors use, so a
    /// failure here and a failure there are comparable by eye.
    pub(super) const NOW: u64 = 1_440_938_160;

    pub(super) fn cfg() -> S3Config {
        S3Config {
            endpoint: "https://storage.example".to_string(),
            region: "us-east-1".to_string(),
            bucket: "subclave".to_string(),
            cas: true,
            access_key_id: "test-access-key".to_string(),
            secret_access_key: "test-secret".to_string(),
        }
    }

    pub(super) fn header<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
        req.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub(super) fn names(req: &Request) -> Vec<&str> {
        req.headers.iter().map(|(n, _)| n.as_str()).collect()
    }

    pub(super) fn all(cfg: &S3Config) -> Vec<Request> {
        vec![
            build_get(cfg, "v1/obj/abc", at(NOW)).unwrap(),
            build_put(cfg, "v1/obj/abc", b"sealed", None, at(NOW)).unwrap(),
            build_delete(cfg, "v1/obj/abc", at(NOW)).unwrap(),
            build_list(cfg, "v1/obj/", None, at(NOW)).unwrap(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_metadata_endpoints_are_refused_before_any_request_is_built() {
        // Both answered without DNS: the first is an IP literal, the second is
        // refused by name. `#[tokio::test]` rather than a plain one because the
        // guard is async and routes through a blocking task even for a literal.
        for endpoint in [
            "http://169.254.169.254/latest/meta-data/",
            "http://metadata.google.internal/",
        ] {
            let mut cfg = test_support::cfg();
            cfg.endpoint = endpoint.to_string();
            let provider = S3Provider::new(cfg).expect("the url itself is well formed");
            let err = provider.get("v1/keyfile").await.unwrap_err();
            assert!(
                matches!(err, ProviderError::Blocked(_)),
                "{endpoint} gave {err:?}"
            );
        }
    }

    /// Every shipped line of the S3 module: each of its files up to its own
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
    ///
    /// Reading one's own source is a new pattern in this tree - the existing
    /// uses embed shell scripts.
    fn shipped() -> String {
        [
            include_str!("mod.rs"),
            include_str!("build.rs"),
            include_str!("classify.rs"),
            include_str!("list_xml.rs"),
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
    fn this_module_ships_no_vendor_endpoint_and_no_credential() {
        // A BACKSTOP OVER A GREP-ABLE SUBSET, not the guarantee. The guarantee
        // is that `S3Config` has no `Default` impl, which makes a defaulted
        // endpoint unrepresentable rather than merely absent today.
        //
        // Deliberately NOT checking the names of particular self-hosted servers
        // or a loopback address: both appear in truthful prose about what this
        // provider supports.
        let shipped = shipped();
        assert!(
            shipped.contains("impl SyncProvider for S3Provider"),
            "the split landed before the last shipped item, so the scan does not reach the end"
        );
        for needle in ["amazonaws.com", "AKIA", "ASIA"] {
            assert!(
                !shipped.contains(needle),
                "the shipped half of this module carries {needle}"
            );
        }
    }
}
