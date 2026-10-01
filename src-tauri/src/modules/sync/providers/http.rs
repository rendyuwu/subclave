//! The HTTP shell the two backends share.
//!
//! Both backends send a request they have already decided, read back a status,
//! an etag and a body, and turn a `reqwest` failure into a disposition. None of
//! that depends on the protocol, so it is written once here and each provider
//! keeps only what its protocol decides.
//!
//! REDIRECTS ARE REFUSED, NOT RE-GUARDED. `reqwest` follows 3xx by default,
//! and the client built here carries the credential on every request: for the
//! signed backend `host` is part of the canonical request, so a redirect to
//! another host guarantees a signature rejection that reads exactly like a bad
//! secret key, and for the password-bearing one a redirect would send the
//! password to a host the user never named. The client is built to follow none
//! at all. A refused redirect is not an error to a client that follows none -
//! it comes back as an ordinary 3xx response - so each backend classifies that
//! status for itself.
//!
//! THE SSRF GUARD PASS IS CACHED AND ITS FAILURE IS NOT. The guard resolves the
//! host, so re-running it per object would put a DNS lookup in front of every
//! request in an inventory-sized listing, and the endpoint cannot change
//! without a new provider. Caching a FAILURE would be worse than re-resolving:
//! the guard answers with the same error type for a link-local address and for
//! a name that simply did not resolve, so one attempt made on a dropped network
//! would poison the provider for the life of the process and report it as a
//! security refusal, which sends the user looking for a policy that does not
//! exist. Only the pass is stored.

use std::time::Duration;

use crate::modules::sync::provider::ProviderError;

/// Generous next to a reachability ping, because this one carries an object
/// body over whatever link the user's remote is on. The same bound on both
/// backends.
pub(super) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A request the shell only has to send: nothing left to decide.
///
/// ONE TYPE FOR BOTH BACKENDS. The signed backend and the password-bearing one
/// spell the same three fields, so a request is a request whichever protocol
/// built it, and the shell that sends it needs no second shape.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub(super) method: &'static str,
    pub(super) url: String,
    /// In a fixed order: the signed headers and the authorization for one
    /// backend, or the authorization and any extras for the other. Fixed so a
    /// test can assert the whole list.
    pub(super) headers: Vec<(String, String)>,
}

/// HAND-WRITTEN AND REDACTING. The `Authorization` value is either the access
/// key id and the request's signature or the user's password in a reversible
/// encoding, and a derived formatter would put that into whatever log line or
/// assertion message ever formats a request. Written rather than omitted
/// because `assert_eq!` needs one, and an assertion that cannot print its two
/// sides is worse than a redacted one.
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(n, v)| {
                let shown = if n == "authorization" {
                    "<redacted>"
                } else {
                    v.as_str()
                };
                (n.as_str(), shown)
            })
            .collect();
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &headers)
            .finish()
    }
}

/// What a send came back with, before anything is decided about it.
pub(super) struct RawResponse {
    pub(super) status: u16,
    pub(super) etag: Option<String>,
    pub(super) body: Vec<u8>,
}

/// Build the client both backends send through: no redirect following, one
/// timeout, and the process-wide crypto provider installed first.
///
/// `reqwest` is built with its `rustls-no-provider` feature, which deliberately
/// installs nothing, so every `Client::builder().build()` panics until the
/// process names a provider. Called here rather than from
/// `src-tauri/src/lib.rs`, because a unit test builds a client too.
pub(super) fn build_client() -> Result<reqwest::Client, ProviderError> {
    super::ensure_crypto_provider();
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ProviderError::Config(format!("the sync http client could not be built: {e}")))
}

/// Run the SSRF guard once per provider, and only ever store a pass.
///
/// The caller passes its own cell and endpoint, because the endpoint is typed
/// by the user and read from a per-provider field. Two first calls racing here
/// both run the guard and both set the cell; that costs one extra resolution
/// and cannot disagree, which is cheaper than the lock that would prevent it.
pub(super) async fn ensure_allowed(
    cell: &tokio::sync::OnceCell<()>,
    endpoint_url: &str,
) -> Result<(), ProviderError> {
    if cell.initialized() {
        return Ok(());
    }
    super::reject_metadata_ssrf(endpoint_url)
        .await
        .map_err(guard_failure)?;
    let _ = cell.set(());
    Ok(())
}

/// Send one already-built request and read back its status, etag and body.
pub(super) async fn send(
    client: &reqwest::Client,
    req: Request,
    body: Option<Vec<u8>>,
) -> Result<RawResponse, ProviderError> {
    let method = match req.method {
        "GET" => reqwest::Method::GET,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        // Not in the http crate's table of standard methods, so they are built
        // from their bytes. Both are upper-case ASCII tokens, which is the only
        // shape that construction can refuse.
        name @ ("PROPFIND" | "MKCOL") => {
            reqwest::Method::from_bytes(name.as_bytes()).map_err(|e| {
                ProviderError::Config(format!("the {name} method could not be built: {e}"))
            })?
        }
        other => {
            return Err(ProviderError::Config(format!(
                "unsupported sync http method \"{other}\""
            )))
        }
    };
    let mut builder = client.request(method, &req.url);
    for (name, value) in &req.headers {
        builder = builder.header(name, value);
    }
    if let Some(body) = body {
        builder = builder.body(body);
    }
    let resp = builder.send().await.map_err(from_reqwest)?;
    let status = resp.status().as_u16();
    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(normalize_etag);
    let body = resp.bytes().await.map_err(from_reqwest)?.to_vec();
    Ok(RawResponse { status, etag, body })
}

/// Which disposition a refusal from the SSRF guard is.
///
/// Keyed on the guard's OWN vocabulary: it prefixes a refusal with `blocked:`
/// and says so in plain words for a resolution that did not answer. A dropped
/// network is a transport failure, not a policy decision, and the two want
/// opposite things from the caller - one is worth retrying, the other never is.
pub(super) fn guard_failure(why: String) -> ProviderError {
    if why.starts_with("blocked:") {
        ProviderError::Blocked(why)
    } else {
        ProviderError::Transport(why)
    }
}

/// The timeout and connect shapes a request can have before it ever gets a
/// status, plus the fallback for everything else.
///
/// Pure, taking the booleans rather than the error, because this is the ONLY
/// producer of `ProviderError::Transport`: leaving it in the shell left that
/// half of the taxonomy untestable. The signed backend adds a redirect arm on
/// top of this in its own classifier; the password-bearing one has none,
/// because a client built to follow no redirect does not error on one.
///
/// `pub` because each provider re-exports it at the path its own module had
/// before the split, and an item restricted to this module tree cannot be
/// re-exported as `pub`.
pub fn transport_error(is_timeout: bool, is_connect: bool, message: String) -> ProviderError {
    if is_timeout {
        return ProviderError::Transport(format!("the remote did not answer in time ({message})"));
    }
    if is_connect {
        return ProviderError::Transport(format!("the remote could not be reached ({message})"));
    }
    ProviderError::Transport(message)
}

/// The `reqwest` failure mapped into the transport taxonomy above.
pub(super) fn from_reqwest(e: reqwest::Error) -> ProviderError {
    transport_error(e.is_timeout(), e.is_connect(), e.to_string())
}

/// An etag with its quoting and its weak marker removed, because servers differ
/// on both.
///
/// `pub` because each provider re-exports it at the path its own module had
/// before the split, and an item restricted to this module tree cannot be
/// re-exported as `pub`.
pub fn normalize_etag(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_did_not_resolve_is_a_transport_failure_and_not_a_refusal() {
        // The guard answers a link-local address and an unreachable DNS server
        // with the same error TYPE, and only one of them is a policy decision.
        // Reporting a dropped network as "blocked" sends the user looking for
        // a setting that does not exist - and, because only a PASS is cached,
        // the same call has to be able to succeed later.
        assert!(matches!(
            guard_failure("blocked: link-local / cloud-metadata address".to_string()),
            ProviderError::Blocked(_)
        ));
        assert!(matches!(
            guard_failure("blocked: cloud metadata endpoint".to_string()),
            ProviderError::Blocked(_)
        ));
        for transient in [
            "dns resolve failed: failed to lookup address information",
            "dns task failed: task panicked",
        ] {
            assert!(
                matches!(
                    guard_failure(transient.to_string()),
                    ProviderError::Transport(_)
                ),
                "{transient}"
            );
        }
    }

    #[test]
    fn an_etag_loses_its_quoting_however_the_server_spelled_it() {
        for raw in ["\"abc123\"", "abc123", "W/\"abc123\"", "  \"abc123\" "] {
            assert_eq!(normalize_etag(raw), "abc123", "{raw}");
        }
    }
}
