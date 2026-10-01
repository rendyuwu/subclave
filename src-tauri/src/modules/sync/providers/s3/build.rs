//! The request builders: every decision about which URL and which headers,
//! with nothing left to send.
//!
//! THE SIGNED PATH AND THE SENT PATH HAVE TO BE THE SAME STRING, which is why
//! [`checked_path`] sits in front of every one of them: the client reparses the
//! URL under the WHATWG rules, and those remove a `.` or `..` segment, so a key
//! carrying one would be signed as written and sent collapsed.
//!
//! NO `Content-Type` IS SIGNED OR SENT on any verb. A typeless object defaults
//! to a binary media type, which is what a sealed envelope is, and a present
//! content type would add a fourth signed header and a fourth way to get the
//! canonical request wrong, for no reader.

use std::time::SystemTime;

use super::super::http::Request;
use super::super::sigv4;
use super::S3Config;
use crate::modules::sync::provider::ProviderError;

/// The service name that goes into the credential scope. Fixed by the
/// protocol, not by the vendor: every S3-compatible server signs under it.
const SERVICE: &str = "s3";

/// The endpoint, taken apart once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Endpoint {
    /// Scheme and authority, no trailing slash.
    base: String,
    /// The `host` header value, carrying a port only when it is not the
    /// scheme's default - which is exactly when the canonical request must
    /// carry one too.
    host: String,
    /// A base path for a server mounted under one, `""` otherwise. Part of
    /// every signed path.
    prefix: String,
}

/// Take the endpoint apart, or say why it cannot be used.
///
/// Refused HERE and at construction rather than at the first request, so a
/// provider that exists is a provider that can sign. The scheme check is part
/// of that: several URL spellings parse happily and then have no host to put in
/// the canonical request.
pub(super) fn endpoint(cfg: &S3Config) -> Result<Endpoint, ProviderError> {
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
    let prefix = parsed.path().trim_end_matches('/').to_string();
    // A base path the url parser already had to escape would be escaped a
    // SECOND time on the way into the signed path, since the percent itself is
    // not an unreserved character. Signed and sent would still agree, so there
    // is no rejected signature to notice - the request would simply address a
    // path nobody has. Refused rather than repaired, because the repair is a
    // guess at what the user meant.
    if prefix.contains('%') {
        return Err(bad(format!(
            "the sync endpoint's path must be plain, and \"{prefix}\" is escaped"
        )));
    }
    Ok(Endpoint {
        base: format!("{}://{host}", parsed.scheme()),
        host,
        prefix,
    })
}

/// Every path this provider signs, refusing the one shape that cannot survive
/// the trip.
///
/// THE SIGNED PATH AND THE SENT PATH HAVE TO BE THE SAME STRING. The canonical
/// request takes the path verbatim - the signer deliberately does not
/// normalize, because the service does not either - but the URL is reparsed by
/// the http client under the WHATWG rules, and those REMOVE a `.` or `..`
/// segment. A key carrying one would therefore be signed as written and sent
/// collapsed, and the answer is a rejected signature: the one failure that
/// reads exactly like a wrong secret key.
///
/// Refusing is the only option that leaves no gap. Encoding does not help - the
/// same rules decode a percent-escaped dot before collapsing it - and
/// normalizing here would sign a path the caller did not ask for.
fn checked_path(path: String) -> Result<String, ProviderError> {
    if path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(ProviderError::Config(format!(
            "a sync path may not carry a \".\" or \"..\" segment, and \"{path}\" does"
        )));
    }
    Ok(path)
}

/// One request's worth of signing context.
///
/// A struct because the builders otherwise pass the same five values
/// positionally, and a mix-up between `region` and the service name would
/// compile and then fail as a rejected signature.
struct Signing<'a> {
    cfg: &'a S3Config,
    endpoint: Endpoint,
    now: SystemTime,
}

impl<'a> Signing<'a> {
    fn new(cfg: &'a S3Config, now: SystemTime) -> Result<Self, ProviderError> {
        Ok(Self {
            endpoint: endpoint(cfg)?,
            cfg,
            now,
        })
    }

    /// The path of one object, unencoded. Exactly one separator between each
    /// part, whatever trailing slashes the endpoint carried.
    fn path(&self, key: &str) -> Result<String, ProviderError> {
        checked_path(format!(
            "{}/{}/{key}",
            self.endpoint.prefix, self.cfg.bucket
        ))
    }

    /// The path of the bucket itself, which is what a listing addresses.
    fn bucket_path(&self) -> Result<String, ProviderError> {
        checked_path(format!("{}/{}", self.endpoint.prefix, self.cfg.bucket))
    }

    /// Sign one request.
    ///
    /// `extra` headers are sent but NOT signed, which is deliberate for the
    /// callers that use it: signing a condition would make a conditional and an
    /// unconditional request differ in their signature, and the service
    /// requires only the host and the `x-amz-*` headers to be signed anyway.
    fn sign(
        &self,
        method: &'static str,
        path: &str,
        query: &[(&str, &str)],
        payload_hash: &str,
        extra: &[(&str, &str)],
    ) -> Request {
        let (amz_date, date) = sigv4::amz_date(self.now);
        let canonical_query = sigv4::canonical_query(query);

        // The three S3 signs on EVERY verb. The suite's generic-service vectors
        // sign only two of them, which is why a signer that reproduces those
        // can still be rejected on every real request.
        let signed: [(&str, &str); 3] = [
            ("host", &self.endpoint.host),
            ("x-amz-content-sha256", payload_hash),
            ("x-amz-date", &amz_date),
        ];
        let canonical =
            sigv4::canonical_request(method, path, &canonical_query, &signed, payload_hash);
        let scope = sigv4::SigningScope {
            access_key_id: &self.cfg.access_key_id,
            secret_access_key: &self.cfg.secret_access_key,
            date: &date,
            region: &self.cfg.region,
            service: SERVICE,
        };
        let authorization = sigv4::authorization_header(
            &scope,
            &amz_date,
            &sigv4::signed_headers(&signed),
            &canonical,
        );

        let mut headers: Vec<(String, String)> = signed
            .iter()
            .map(|(n, v)| ((*n).to_string(), (*v).to_string()))
            .collect();
        headers.push(("authorization".to_string(), authorization));
        headers.extend(extra.iter().map(|(n, v)| (n.to_string(), v.to_string())));

        let encoded = sigv4::uri_encode(path, false);
        let url = if canonical_query.is_empty() {
            format!("{}{encoded}", self.endpoint.base)
        } else {
            format!("{}{encoded}?{canonical_query}", self.endpoint.base)
        };
        Request {
            method,
            url,
            headers,
        }
    }
}

pub fn build_get(cfg: &S3Config, key: &str, now: SystemTime) -> Result<Request, ProviderError> {
    let signing = Signing::new(cfg, now)?;
    Ok(signing.sign(
        "GET",
        &signing.path(key)?,
        &[],
        sigv4::EMPTY_PAYLOAD_SHA256,
        &[],
    ))
}

pub fn build_delete(cfg: &S3Config, key: &str, now: SystemTime) -> Result<Request, ProviderError> {
    let signing = Signing::new(cfg, now)?;
    Ok(signing.sign(
        "DELETE",
        &signing.path(key)?,
        &[],
        sigv4::EMPTY_PAYLOAD_SHA256,
        &[],
    ))
}

/// Which etag, if any, actually rides as a condition.
///
/// ONE DEFINITION, FOUR READERS: the two builders that attach a condition and
/// the shells that have to tell the response mapper what was sent. Two
/// spellings of the same predicate would have to stay in step, and the symptom
/// of their drifting is a 404 read as the wrong disposition.
pub(super) fn condition<'a>(cfg: &S3Config, if_match: Option<&'a str>) -> Option<&'a str> {
    if_match.filter(|_| cfg.cas)
}

/// A put, conditional only when the user said this endpoint can do it.
///
/// THE TOGGLE GATES THE CONDITION HERE rather than at the call site, so a
/// caller may pass `if_match` unconditionally and an endpoint that cannot
/// honour it degrades to last-write-wins instead of failing. Setting the toggle
/// wrong therefore costs a weaker guarantee, never a lost record.
pub fn build_put(
    cfg: &S3Config,
    key: &str,
    bytes: &[u8],
    if_match: Option<&str>,
    now: SystemTime,
) -> Result<Request, ProviderError> {
    let signing = Signing::new(cfg, now)?;
    let payload_hash = sigv4::sha256_hex(bytes);
    let extra: Vec<(&str, &str)> = match condition(cfg, if_match) {
        Some(etag) => vec![("if-match", etag)],
        None => Vec::new(),
    };
    Ok(signing.sign("PUT", &signing.path(key)?, &[], &payload_hash, &extra))
}

/// A create, refused when the key is already there, and only when the user
/// said this endpoint can honour the condition.
///
/// THE CONDITION IS `If-None-Match: *`, not an etag: the caller is creating a
/// key it believes is absent - the keyfile on a fresh remote - and the write
/// has to lose to any object that appeared first. Like `build_put`'s etag, the
/// header is an UNSIGNED extra through the same [`condition`] predicate, so a
/// toggle set wrong sends an unconditional create rather than a broken
/// signature, and a race on the empty prefix degrades to last-write-wins
/// instead of losing the keyfile.
pub fn build_put_if_absent(
    cfg: &S3Config,
    key: &str,
    bytes: &[u8],
    now: SystemTime,
) -> Result<Request, ProviderError> {
    let signing = Signing::new(cfg, now)?;
    let payload_hash = sigv4::sha256_hex(bytes);
    let extra: Vec<(&str, &str)> = match condition(cfg, Some("*")) {
        Some(value) => vec![("if-none-match", value)],
        None => Vec::new(),
    };
    Ok(signing.sign("PUT", &signing.path(key)?, &[], &payload_hash, &extra))
}

pub fn build_list(
    cfg: &S3Config,
    prefix: &str,
    continuation: Option<&str>,
    now: SystemTime,
) -> Result<Request, ProviderError> {
    let signing = Signing::new(cfg, now)?;
    let mut query: Vec<(&str, &str)> = vec![("list-type", "2"), ("prefix", prefix)];
    if let Some(token) = continuation {
        query.push(("continuation-token", token));
    }
    Ok(signing.sign(
        "GET",
        &signing.bucket_path()?,
        &query,
        sigv4::EMPTY_PAYLOAD_SHA256,
        &[],
    ))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{all, at, cfg, header, names, NOW};
    use super::*;

    // --- the S3-specific signing the generic vectors cannot reach ----------

    #[test]
    fn every_verb_signs_exactly_the_three_headers_s3_requires_and_no_content_type() {
        // The published suite signs a generic service over two headers. S3
        // additionally requires the payload hash to be SENT and SIGNED on every
        // verb, so a signer that reproduces the suite can still be rejected on
        // every real request. This is that gap.
        for req in all(&cfg()) {
            assert_eq!(
                names(&req),
                vec![
                    "host",
                    "x-amz-content-sha256",
                    "x-amz-date",
                    "authorization"
                ],
                "{} {}",
                req.method,
                req.url
            );
            let authorization = header(&req, "authorization").unwrap();
            assert!(
                authorization.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date,"),
                "{} signed the wrong set: {authorization}",
                req.method
            );
            assert!(
                authorization.starts_with("AWS4-HMAC-SHA256 Credential=test-access-key/20150830/us-east-1/s3/aws4_request,"),
                "{} built the wrong scope: {authorization}",
                req.method
            );
            assert_eq!(header(&req, "host"), Some("storage.example"));
            assert_eq!(header(&req, "x-amz-date"), Some("20150830T123600Z"));
            // Sending one would add a fourth signed header and a fourth way to
            // get the canonical request wrong, for no reader.
            assert!(
                !names(&req).iter().any(|n| n.contains("content-type")),
                "{} sent a content type",
                req.method
            );
        }
    }

    #[test]
    fn the_bodyless_verbs_carry_the_empty_payload_hash_and_a_put_carries_its_own() {
        let cfg = cfg();
        for req in [
            build_get(&cfg, "k", at(NOW)).unwrap(),
            build_delete(&cfg, "k", at(NOW)).unwrap(),
            build_list(&cfg, "", None, at(NOW)).unwrap(),
        ] {
            assert_eq!(
                header(&req, "x-amz-content-sha256"),
                Some(sigv4::EMPTY_PAYLOAD_SHA256),
                "{}",
                req.method
            );
        }
        let body = b"a sealed envelope";
        let put = build_put(&cfg, "k", body, None, at(NOW)).unwrap();
        assert_eq!(
            header(&put, "x-amz-content-sha256"),
            Some(sigv4::sha256_hex(body).as_str())
        );
        // And it is not silently the empty hash, which is the way this goes
        // wrong.
        assert_ne!(
            header(&put, "x-amz-content-sha256"),
            Some(sigv4::EMPTY_PAYLOAD_SHA256)
        );
    }

    /// The `Authorization` value each builder produces, for the fixture above
    /// at the fixed instant above.
    ///
    /// A REGRESSION PIN, AND AN INDEPENDENT ONE. These were not read back out
    /// of this implementation: they were computed from the protocol definition
    /// by a separate program, so a defect shared between the signer and its own
    /// output cannot hide in them. That matters because the published vectors
    /// sign a GENERIC service over two headers with a literal date, and every
    /// other test here asserts on header NAMES and substrings. Change the path
    /// fed to the canonical request without changing the URL - double-encode
    /// it, normalize it, prepend something - and every one of those still
    /// passes while every real request comes back rejected. This is what
    /// notices.
    const PINNED: [(&str, &str); 4] = [
        (
            "GET",
            "AWS4-HMAC-SHA256 Credential=test-access-key/20150830/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=69a665281a6bf951004bebdf2008288fb6ffa94805cf5042d0a15412eeaccfb3",
        ),
        (
            "PUT",
            "AWS4-HMAC-SHA256 Credential=test-access-key/20150830/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=d14693d753b0723924b8ff197d395a95b7cf7746cbbe35521dea378fa334e1ea",
        ),
        (
            "DELETE",
            "AWS4-HMAC-SHA256 Credential=test-access-key/20150830/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=fb5933170cb6756df65248aa019e1e4f4a7c57886a55cdddfaa45a6c3a9ba104",
        ),
        (
            "LIST",
            "AWS4-HMAC-SHA256 Credential=test-access-key/20150830/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature=0292a3c8636e34d33920c112897a2d626bbe9cdabcd7aa952e2251f5f491c677",
        ),
    ];

    #[test]
    fn every_builder_still_produces_the_signature_it_was_pinned_at() {
        for (req, (label, expected)) in all(&cfg()).into_iter().zip(PINNED) {
            assert_eq!(
                header(&req, "authorization"),
                Some(expected),
                "{label} no longer signs what it was pinned at"
            );
        }
    }

    #[test]
    fn the_url_that_is_sent_carries_the_path_that_was_signed() {
        // THE SEAM THE PURE HALF CANNOT SEE ON ITS OWN. The canonical request
        // takes the path verbatim, but the http client reparses this URL, and
        // that parse has rules of its own. Asserting on the signer's output
        // alone would pass while the two disagree, and the symptom is a
        // rejected signature rather than anything about paths.
        let cfg = cfg();
        for key in ["v1/obj/abc", "v1/obj/aa%bb", "caf\u{e9}/obj", "a//b", ""] {
            let req = build_get(&cfg, key, at(NOW)).unwrap();
            let parsed = url::Url::parse(&req.url).expect("the builder produced a url");
            assert_eq!(
                parsed.path(),
                sigv4::uri_encode(&format!("/subclave/{key}"), false),
                "\"{key}\" survived signing but not the parse"
            );
        }
    }

    #[test]
    fn a_dot_segment_is_refused_rather_than_signed_and_then_collapsed() {
        // The parse above REMOVES a `.` or `..` segment, and decodes a
        // percent-escaped one first, so neither passing it through nor
        // encoding it keeps the two halves equal. Refusing is the only answer
        // that leaves no gap, and the key arrives here composed from a
        // user-typed prefix.
        let cfg = cfg();
        for key in ["v1/../obj/abc", "v1/./obj", "..", ".", "v1/obj/.."] {
            for built in [
                build_get(&cfg, key, at(NOW)),
                build_delete(&cfg, key, at(NOW)),
                build_put(&cfg, key, b"x", None, at(NOW)),
                build_put_if_absent(&cfg, key, b"x", at(NOW)),
            ] {
                let err = built
                    .err()
                    .unwrap_or_else(|| panic!("\"{key}\" must be refused"));
                assert!(
                    matches!(err, ProviderError::Config(_)),
                    "\"{key}\" gave {err:?}"
                );
            }
        }
        // And an ordinary key still builds, so the guard is not simply
        // refusing everything.
        assert!(build_get(&cfg, "v1/obj/abc", at(NOW)).is_ok());
        // Nor is a dot INSIDE a segment a dot segment.
        assert!(build_get(&cfg, "v1/obj/a.b", at(NOW)).is_ok());
    }

    #[test]
    fn an_endpoint_whose_path_is_already_escaped_is_refused() {
        // It would be escaped a second time on the way into the signed path,
        // and because signed and sent would still AGREE there is no rejected
        // signature to notice - the request would quietly address a path
        // nobody has.
        let mut spaced = cfg();
        spaced.endpoint = "https://storage.example/my path/".to_string();
        let err =
            build_get(&spaced, "k", at(NOW)).expect_err("an escaped endpoint path must be refused");
        assert!(matches!(err, ProviderError::Config(_)), "{err:?}");
    }

    #[test]
    fn a_non_default_port_rides_into_the_host_header_and_a_default_one_does_not() {
        // The canonical request must carry the port exactly when the URL does,
        // and a self-hosted server on a non-standard port is the common case.
        let mut ported = cfg();
        ported.endpoint = "https://storage.example:9000".to_string();
        assert_eq!(
            header(&build_get(&ported, "k", at(NOW)).unwrap(), "host"),
            Some("storage.example:9000")
        );

        let mut default_port = cfg();
        default_port.endpoint = "https://storage.example:443".to_string();
        assert_eq!(
            header(&build_get(&default_port, "k", at(NOW)).unwrap(), "host"),
            Some("storage.example")
        );
    }

    #[test]
    fn a_list_asks_for_the_second_listing_version_and_sorts_its_query() {
        let req = build_list(&cfg(), "v1/obj/", Some("tok/en"), at(NOW)).unwrap();
        // Sorted by the ENCODED key, and the token percent encoded because a
        // continuation token is opaque and may carry anything.
        assert_eq!(
            req.url,
            "https://storage.example/subclave?continuation-token=tok%2Fen&list-type=2&prefix=v1%2Fobj%2F"
        );
        assert_eq!(
            build_list(&cfg(), "v1/obj/", None, at(NOW)).unwrap().url,
            "https://storage.example/subclave?list-type=2&prefix=v1%2Fobj%2F"
        );
    }

    // --- compare-and-swap gating ------------------------------------------

    #[test]
    fn the_condition_rides_only_when_the_endpoint_was_told_it_can_honour_it() {
        let capable = cfg();
        let mut incapable = cfg();
        incapable.cas = false;

        let with = build_put(&capable, "k", b"body", Some("etag-1"), at(NOW)).unwrap();
        assert_eq!(header(&with, "if-match"), Some("etag-1"));

        // Suppressed, and byte-identical to the unconditional call - which is
        // the real claim: the condition is not signed, so suppressing it must
        // not perturb anything else.
        let suppressed = build_put(&incapable, "k", b"body", Some("etag-1"), at(NOW)).unwrap();
        let unconditional = build_put(&incapable, "k", b"body", None, at(NOW)).unwrap();
        assert_eq!(suppressed, unconditional);
        assert!(header(&suppressed, "if-match").is_none());

        // And the capable config with no condition asked for carries none
        // either, so the toggle alone does not add one.
        assert!(build_put(&capable, "k", b"body", None, at(NOW))
            .unwrap()
            .headers
            .iter()
            .all(|(n, _)| n != "if-match"));
    }

    #[test]
    fn put_if_absent_is_conditional_only_with_cas() {
        let capable = cfg();
        let mut incapable = cfg();
        incapable.cas = false;

        let conditional = build_put_if_absent(&capable, "v1/obj/abc", b"sealed", at(NOW)).unwrap();
        assert_eq!(header(&conditional, "if-none-match"), Some("*"));
        // The header is an UNSIGNED extra, so the condition can be suppressed
        // without perturbing the signature. Pinned against the plain PUT of the
        // same key and body, because adding `if-none-match` must not move the
        // canonical request.
        assert_eq!(header(&conditional, "authorization"), Some(PINNED[1].1));

        let unconditional =
            build_put_if_absent(&incapable, "v1/obj/abc", b"sealed", at(NOW)).unwrap();
        assert!(header(&unconditional, "if-none-match").is_none());
        assert_eq!(header(&unconditional, "authorization"), Some(PINNED[1].1));
        // And an endpoint that cannot condition sends exactly the plain put.
        assert_eq!(
            unconditional,
            build_put(&incapable, "v1/obj/abc", b"sealed", None, at(NOW)).unwrap()
        );
    }

    #[test]
    fn an_object_url_joins_its_parts_with_exactly_one_separator() {
        let bare = cfg();
        let mut trailing = cfg();
        trailing.endpoint = "https://storage.example/".to_string();
        for cfg in [&bare, &trailing] {
            assert_eq!(
                build_get(cfg, "v1/obj/abc", at(NOW)).unwrap().url,
                "https://storage.example/subclave/v1/obj/abc"
            );
        }

        // A server mounted under a base path keeps it, and the signed path
        // carries it too or the signature would not match the URL.
        let mut mounted = cfg();
        mounted.endpoint = "https://storage.example/s3/".to_string();
        assert_eq!(
            build_get(&mounted, "v1/obj/abc", at(NOW)).unwrap().url,
            "https://storage.example/s3/subclave/v1/obj/abc"
        );

        // A non-ASCII key segment, which reaches here because the layer above
        // composes the key from a user-typed prefix.
        assert_eq!(
            build_get(&bare, "caf\u{e9}/obj", at(NOW)).unwrap().url,
            "https://storage.example/subclave/caf%C3%A9/obj"
        );
    }
}
