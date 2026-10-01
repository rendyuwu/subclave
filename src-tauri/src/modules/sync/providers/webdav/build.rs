//! The request builders: which URL and which headers, with nothing left to
//! send.
//!
//! NO CONDITIONAL `PUT`. A WebDAV server may or may not honour a conditional
//! write and nothing in the protocol says which, so no builder behind the plain
//! put accepts an etag at all. The one conditional request is the create, which
//! always sends `if-none-match: *`.

use base64::{engine::general_purpose::STANDARD as B64, Engine};

use super::super::http::Request;
use super::super::sigv4;
use super::WebDavConfig;
use crate::modules::sync::provider::ProviderError;

/// Only the listing carries a body, and only the listing labels one.
const XML_CONTENT_TYPE: &str = "application/xml; charset=\"utf-8\"";

/// One level down and no further. The object namespace the layer above composes
/// is flat, and an unbounded depth is refused by default on at least one common
/// server - so asking for it would be a failure on some remotes in exchange for
/// rows nobody reads.
const LIST_DEPTH: &str = "1";

/// The endpoint, taken apart once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Scheme and authority, no trailing slash.
    base: String,
    /// The base path the user's dav mount sits at, `""` when it sits at the
    /// root. Every request path starts with it.
    path: String,
}

/// Take the endpoint apart, or say why it cannot be used.
///
/// Refused HERE and at construction rather than at the first request, so a
/// provider that exists is a provider that can address something. No `host`
/// field falls out of this, where the other backend's version produces one:
/// that one is a signed header, and nothing here signs anything.
pub(super) fn endpoint(cfg: &WebDavConfig) -> Result<Endpoint, ProviderError> {
    let bad = |why: String| ProviderError::Config(why);
    let parsed = url::Url::parse(cfg.endpoint.trim())
        .map_err(|e| bad(format!("the sync endpoint is not a url: {e}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(bad(format!("unsupported sync endpoint scheme \"{other}\""))),
    }
    let host = parsed
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| bad("the sync endpoint names no host".to_string()))?;
    let host = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let path = parsed.path().trim_end_matches('/').to_string();
    // A base path the url parser already had to escape would be escaped a
    // SECOND time on the way into every request url, since the percent itself
    // is not an unreserved character - so the request would address a path
    // nobody has. Refused rather than repaired, because the repair is a guess
    // at what the user meant.
    if path.contains('%') {
        return Err(bad(format!(
            "the sync endpoint's path must be plain, and \"{path}\" is escaped"
        )));
    }
    Ok(Endpoint {
        base: format!("{}://{host}", parsed.scheme()),
        path,
    })
}

/// The server-side path of one key, unencoded. Exactly one separator between
/// the base path and the key, whatever trailing slashes the endpoint carried.
pub(super) fn object_path(ep: &Endpoint, key: &str) -> String {
    format!("{}/{key}", ep.path)
}

/// Where one object lives, as a URL.
pub fn object_url(ep: &Endpoint, key: &str) -> String {
    format!(
        "{}{}",
        ep.base,
        sigv4::uri_encode(&object_path(ep, key), false)
    )
}

/// The `Authorization` value.
///
/// PLAIN CREDENTIALS IN A REVERSIBLE ENCODING, which is what this scheme is and
/// is why the frontend warns when the endpoint is not a secure one. Nothing
/// about the encoding is a secret and nothing here pretends otherwise.
pub fn basic_auth(username: &str, password: &str) -> String {
    format!("Basic {}", B64.encode(format!("{username}:{password}")))
}

/// The collections that have to exist before `key` can be stored, shallowest
/// first.
///
/// SHALLOWEST FIRST AND ONE AT A TIME because a server will not create the
/// intermediate collections of a deep request for you - that is stated
/// normatively for the create-collection verb - so the ladder has to be walked
/// rung by rung. A key at the root names no collection and produces none.
pub fn ancestors(key: &str) -> Vec<String> {
    let segments: Vec<&str> = key.split('/').filter(|s| !s.is_empty()).collect();
    let mut out = Vec::new();
    let mut so_far = String::new();
    for segment in segments.iter().take(segments.len().saturating_sub(1)) {
        if !so_far.is_empty() {
            so_far.push('/');
        }
        so_far.push_str(segment);
        out.push(so_far.clone());
    }
    out
}

fn request(method: &'static str, url: String, auth: &str, extra: &[(&str, &str)]) -> Request {
    let mut headers = vec![("authorization".to_string(), auth.to_string())];
    headers.extend(extra.iter().map(|(n, v)| (n.to_string(), v.to_string())));
    Request {
        method,
        url,
        headers,
    }
}

pub fn build_get(ep: &Endpoint, auth: &str, key: &str) -> Request {
    request("GET", object_url(ep, key), auth, &[])
}

/// A put, with no condition and no way to express one.
///
/// NO `if_match` PARAMETER, which is the decision this whole provider rests on
/// made unrepresentable rather than merely unused. The trait says a conditional
/// write is honoured only when the backend reports that capability, and this
/// one reports it as false, so a caller may pass a condition and get
/// last-write-wins instead of a failure.
pub fn build_put(ep: &Endpoint, auth: &str, key: &str) -> Request {
    request("PUT", object_url(ep, key), auth, &[])
}

/// A create: a put that is refused when the key is already there.
///
/// THE HEADER GOES OUT ALWAYS, unlike the other backend where it is gated on a
/// user-set `cas`. There is no such toggle here to gate it on, and the servers
/// this has to work against disagree about whether they honour it - so the
/// request is the same everywhere and the answer is read for what it is. A
/// server that ignores the header stores the object anyway; that is the
/// residue the module header names, not a failure.
pub fn build_put_if_absent(ep: &Endpoint, auth: &str, key: &str) -> Request {
    request("PUT", object_url(ep, key), auth, &[("if-none-match", "*")])
}

pub fn build_delete(ep: &Endpoint, auth: &str, key: &str) -> Request {
    request("DELETE", object_url(ep, key), auth, &[])
}

pub fn build_propfind(ep: &Endpoint, auth: &str, prefix: &str) -> Request {
    request(
        "PROPFIND",
        object_url(ep, prefix),
        auth,
        &[("depth", LIST_DEPTH), ("content-type", XML_CONTENT_TYPE)],
    )
}

pub fn build_mkcol(ep: &Endpoint, auth: &str, collection: &str) -> Request {
    request("MKCOL", object_url(ep, collection), auth, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> WebDavConfig {
        WebDavConfig {
            endpoint: "https://cloud.example/remote.php/dav/files/rendi".to_string(),
            username: "rendi".to_string(),
            password: "hunter2".to_string(),
        }
    }

    fn ep() -> Endpoint {
        endpoint(&cfg()).expect("the test endpoint parses")
    }

    fn header<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
        req.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    // --- the endpoint and the urls ----------------------------------------

    #[test]
    fn an_endpoint_that_is_not_a_usable_url_is_refused() {
        for bad in [
            "not a url",
            "ftp://dav.example",
            "https://",
            "https://cloud.example/dav%20files",
        ] {
            let mut cfg = cfg();
            cfg.endpoint = bad.to_string();
            let err = endpoint(&cfg)
                .err()
                .unwrap_or_else(|| panic!("{bad} must be refused"));
            assert!(
                matches!(err, ProviderError::Config(_)),
                "{bad} gave {err:?}"
            );
        }
    }

    #[test]
    fn an_object_url_joins_the_base_path_and_the_key_with_one_separator() {
        assert_eq!(
            object_url(&ep(), "vault/v1/obj/ab12"),
            "https://cloud.example/remote.php/dav/files/rendi/vault/v1/obj/ab12"
        );

        // A mount at the root, and an endpoint whose trailing slash the user
        // typed: neither may double or drop a separator.
        let mut bare = cfg();
        bare.endpoint = "https://cloud.example/".to_string();
        assert_eq!(
            object_url(&endpoint(&bare).unwrap(), "v1/keyfile"),
            "https://cloud.example/v1/keyfile"
        );

        // A non-ASCII segment, which reaches here because the layer above
        // composes the key from a user-typed prefix.
        assert_eq!(
            object_url(&ep(), "caf\u{e9}/obj"),
            "https://cloud.example/remote.php/dav/files/rendi/caf%C3%A9/obj"
        );
    }

    #[test]
    fn basic_auth_is_the_pair_joined_by_a_colon_and_encoded() {
        // Hand-computed: `u:p` is the three bytes 0x75 0x3a 0x70, which pack
        // into `dTpw`.
        assert_eq!(basic_auth("u", "p"), "Basic dTpw");
    }

    // --- the collection ladder --------------------------------------------

    #[test]
    fn ancestors_are_every_parent_collection_shallowest_first() {
        assert_eq!(
            ancestors("vault/v1/obj/ab12"),
            vec!["vault", "vault/v1", "vault/v1/obj"]
        );
        // The empty-user-prefix spelling the layer above produces.
        assert_eq!(ancestors("v1/keyfile"), vec!["v1"]);
        // A key at the root names no collection, so nothing is created.
        assert!(ancestors("keyfile").is_empty());
    }

    // --- the request builders ---------------------------------------------

    #[test]
    fn every_request_carries_the_credential_and_only_the_listing_carries_a_type() {
        let auth = basic_auth("rendi", "hunter2");
        let ep = ep();

        let put = build_put(&ep, &auth, "vault/v1/obj/ab12");
        assert_eq!(put.method, "PUT");
        assert_eq!(header(&put, "authorization"), Some(auth.as_str()));
        // A sealed envelope has no meaningful media type and nothing reads one
        // back, so the server picks its default.
        assert_eq!(header(&put, "content-type"), None);
        // The ordinary put carries no condition: the capability is false, so a
        // caller's `if_match` is dropped and the write is last-write-wins.
        assert_eq!(header(&put, "if-none-match"), None);

        let propfind = build_propfind(&ep, &auth, "vault/v1/obj/");
        assert_eq!(propfind.method, "PROPFIND");
        assert_eq!(header(&propfind, "depth"), Some("1"));
        assert_eq!(
            header(&propfind, "content-type"),
            Some("application/xml; charset=\"utf-8\"")
        );

        for req in [
            build_get(&ep, &auth, "vault/v1/keyfile"),
            build_delete(&ep, &auth, "vault/v1/obj/ab12"),
            build_mkcol(&ep, &auth, "vault/v1"),
        ] {
            assert_eq!(header(&req, "authorization"), Some(auth.as_str()));
        }
    }

    #[test]
    fn put_if_absent_always_sends_the_header() {
        let auth = basic_auth("rendi", "hunter2");
        let ep = ep();
        let create = build_put_if_absent(&ep, &auth, "vault/v1/keyfile");
        assert_eq!(create.method, "PUT");
        assert_eq!(header(&create, "authorization"), Some(auth.as_str()));
        assert_eq!(header(&create, "if-none-match"), Some("*"));
        assert_eq!(create.url, build_put(&ep, &auth, "vault/v1/keyfile").url);
        assert_eq!(header(&create, "content-type"), None);
    }

    #[test]
    fn a_formatted_request_does_not_print_the_credential() {
        let auth = basic_auth("rendi", "hunter2");
        let printed = format!("{:?}", build_get(&ep(), &auth, "vault/v1/keyfile"));
        assert!(printed.contains("<redacted>"), "{printed}");
        assert!(!printed.contains(&auth), "{printed}");
    }
}
