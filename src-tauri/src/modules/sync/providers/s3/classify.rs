//! The response mapping: a status turned into a disposition, and the two ways
//! a request can fail before it ever gets one.
//!
//! TAKES BYTES AND DECODES ONLY WHERE IT HAS TO. A sealed envelope is not text,
//! and a lossy decode of one allocates a second full copy of the object on a
//! path that never reads it.

use super::super::http;
use super::list_xml::tag_text;
use crate::modules::sync::provider::ProviderError;

// --- response mapping -----------------------------------------------------

/// The shared base: a status the caller has already decided is a failure,
/// turned into a disposition.
///
/// Three statuses get their own answer. A stale-etag rejection and a
/// mid-upload conflict are NOT the same thing - after the first the caller
/// knows its copy is stale, after the second it knows nothing at all - and a
/// redirect is refused rather than followed, so the 3xx that a region mismatch
/// produces is surfaced as a refusal rather than chased into a signature
/// rejection.
///
/// TAKES BYTES AND DECODES ONLY WHERE IT HAS TO, which is the last arm. A
/// sealed envelope is not text, and a lossy decode of one allocates a second
/// full copy of the object on a path that never reads it.
pub fn classify(status: u16, body: &[u8]) -> ProviderError {
    match status {
        412 => ProviderError::PreconditionFailed,
        409 => ProviderError::Conflict,
        300..=399 => ProviderError::Blocked(format!(
            "blocked: the remote answered {status}, which this client neither follows nor reads"
        )),
        _ => ProviderError::Remote {
            status,
            code: error_code(&String::from_utf8_lossy(body)),
        },
    }
}

/// A get's outcome. `Ok(None)` means the key is simply not there.
pub fn classify_get(status: u16, body: &[u8]) -> Result<Option<()>, ProviderError> {
    match status {
        200..=299 => Ok(Some(())),
        404 => Ok(None),
        _ => Err(classify(status, body)),
    }
}

/// A put's outcome.
///
/// `conditional` is needed because a status alone cannot tell a conditional put
/// from an unconditional one, and a missing key means something different in
/// each: under a condition it is a key that was deleted from under the caller,
/// and without one it is an ordinary remote failure.
pub fn classify_put(status: u16, body: &[u8], conditional: bool) -> Result<(), ProviderError> {
    match status {
        200..=299 => Ok(()),
        404 if conditional => Err(ProviderError::NotFound),
        _ => Err(classify(status, body)),
    }
}

/// A conditional create's outcome. `Ok(None)` means the object was already
/// there and nothing was written.
///
/// A 412 IS THE ANSWER AND NOT A FAILURE, which is the one place this verb
/// parts from every other: for a create the refusal means another writer won
/// the race, so the caller reads the object rather than treating its own copy
/// as stale. Routing 412 through [`classify`] would report
/// `ProviderError::PreconditionFailed` and send it to pull, merge and retry -
/// the wrong disposition for a keyfile that simply already exists.
///
/// 404 is read the same way [`classify_put`] reads it: an anomaly when a
/// condition was sent, an ordinary remote failure otherwise.
pub fn classify_put_if_absent(
    status: u16,
    body: &[u8],
    conditional: bool,
) -> Result<Option<()>, ProviderError> {
    match status {
        200..=299 => Ok(Some(())),
        412 => Ok(None),
        404 if conditional => Err(ProviderError::NotFound),
        _ => Err(classify(status, body)),
    }
}

/// The three failure shapes a request can have before it ever gets a status.
///
/// Pure, taking the booleans rather than the error, because this is the ONLY
/// producer of `ProviderError::Transport` and one of the producers of
/// `ProviderError::Blocked`: leaving it in the shell left the whole taxonomy
/// untestable.
///
/// The timeout and connect arms come from the shared shell in
/// `src-tauri/src/modules/sync/providers/http.rs`. The redirect arm is added
/// here: for a SIGNED request a followed redirect cannot survive the signature,
/// so it is refused rather than reported as an ordinary remote failure.
pub fn transport_error(
    is_redirect: bool,
    is_timeout: bool,
    is_connect: bool,
    message: String,
) -> ProviderError {
    if is_redirect {
        // Not a transport failure: the client was told to go somewhere else
        // and refused, which is the guard working.
        return ProviderError::Blocked(format!("blocked: a redirect was refused ({message})"));
    }
    http::transport_error(is_timeout, is_connect, message)
}

/// The remote's own error code, when the body carried one.
pub(super) fn error_code(body: &str) -> Option<String> {
    tag_text(body, "Code")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- error taxonomy ---------------------------------------------------

    #[test]
    fn a_stale_etag_and_a_mid_upload_conflict_are_different_dispositions() {
        assert_eq!(classify(412, b""), ProviderError::PreconditionFailed);
        assert_eq!(
            classify(
                409,
                b"<Error><Code>ConditionalRequestConflict</Code></Error>"
            ),
            ProviderError::Conflict
        );
        assert_ne!(classify(412, b""), classify(409, b""));
    }

    #[test]
    fn a_412_reports_the_object_exists() {
        // For a create the refusal IS the answer: another writer got there
        // first. It must not be read as `PreconditionFailed`, which would send
        // the caller to pull, merge and retry instead of reading the object.
        assert_eq!(classify_put_if_absent(412, b"", true), Ok(None));
        assert_eq!(classify_put_if_absent(412, b"", false), Ok(None));
        assert_ne!(
            classify_put_if_absent(412, b"", true),
            Err(ProviderError::PreconditionFailed)
        );
        // The shared classifier still maps 412 to the stale-etag disposition,
        // so this verb's reading is local to the create.
        assert_eq!(classify(412, b""), ProviderError::PreconditionFailed);
        // A 2xx is a stored object, and a conditional 404 is the key that was
        // deleted from under the caller.
        assert_eq!(classify_put_if_absent(200, b"", true), Ok(Some(())));
        assert_eq!(
            classify_put_if_absent(404, b"", true),
            Err(ProviderError::NotFound)
        );
        assert_ne!(
            classify_put_if_absent(404, b"", false),
            Err(ProviderError::NotFound)
        );
    }

    #[test]
    fn a_409_is_still_a_conflict() {
        assert_eq!(
            classify_put_if_absent(
                409,
                b"<Error><Code>ConditionalRequestConflict</Code></Error>",
                true
            ),
            Err(ProviderError::Conflict)
        );
    }

    #[test]
    fn nothing_but_412_is_ever_a_stale_etag() {
        // Over the whole range, because the way this goes wrong is a range arm
        // written a little too wide and a caller then retrying a merge it never
        // needed to do.
        for status in 100u16..=599 {
            let err = classify(status, b"");
            if status == 412 {
                assert_eq!(err, ProviderError::PreconditionFailed);
            } else {
                assert_ne!(err, ProviderError::PreconditionFailed, "{status}");
            }
        }
    }

    #[test]
    fn a_redirect_is_a_refusal_rather_than_a_remote_failure() {
        // The client follows none, so a region redirect arrives as a 3xx
        // RESPONSE. Reporting it as an ordinary remote failure would hide why
        // the request is going nowhere.
        for status in [301u16, 302, 307, 308] {
            assert!(
                matches!(classify(status, b""), ProviderError::Blocked(_)),
                "{status}"
            );
        }
    }

    #[test]
    fn an_error_body_contributes_its_code_and_a_bodyless_one_does_not() {
        assert_eq!(
            classify(403, b"<Error><Code>SignatureDoesNotMatch</Code></Error>"),
            ProviderError::Remote {
                status: 403,
                code: Some("SignatureDoesNotMatch".to_string()),
            }
        );
        assert_eq!(
            classify(500, b""),
            ProviderError::Remote {
                status: 500,
                code: None,
            }
        );
    }

    #[test]
    fn the_three_transport_shapes_are_distinct_and_a_refused_redirect_is_not_one() {
        let redirect = transport_error(true, false, false, "too many redirects".to_string());
        let timeout = transport_error(false, true, false, "operation timed out".to_string());
        let connect = transport_error(false, false, true, "connection refused".to_string());
        let other = transport_error(false, false, false, "body stream ended".to_string());

        assert!(
            matches!(redirect, ProviderError::Blocked(_)),
            "a refused redirect is the guard working, not a transport failure: {redirect:?}"
        );
        assert!(matches!(timeout, ProviderError::Transport(_)));
        assert!(matches!(connect, ProviderError::Transport(_)));
        assert_ne!(timeout, connect);
        assert_ne!(timeout, other);
        assert_ne!(connect, other);
    }

    #[test]
    fn a_missing_key_reads_as_absent_when_getting_and_as_a_failure_when_writing_conditionally() {
        assert_eq!(classify_get(404, b""), Ok(None));
        assert_eq!(classify_get(200, b""), Ok(Some(())));
        assert_eq!(classify_put(404, b"", true), Err(ProviderError::NotFound));
        // Without a condition a 404 is an ordinary remote failure, because
        // nothing was raced.
        assert_ne!(classify_put(404, b"", false), Err(ProviderError::NotFound));
        assert_eq!(classify_put(204, b"", false), Ok(()));
        // A body that is not text at all reaches these on the failure path, so
        // nothing here may assume a decode succeeded.
        assert_eq!(
            classify_get(500, &[0xff, 0xfe]),
            Err(ProviderError::Remote {
                status: 500,
                code: None
            })
        );
    }
}
