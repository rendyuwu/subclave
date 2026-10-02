//! The response mapping: a status turned into a disposition, with the two
//! verb-specific readings the trait forces out of the shell.
//!
//! NO ERROR CODE IS READ OUT OF THE BODY, unlike the signed backend: that
//! protocol defines a machine-readable code element every server fills in, and
//! this one's error bodies are an optional element most servers answer with a
//! human-readable page instead.

use super::build::ancestors;
use crate::modules::sync::provider::ProviderError;

// --- response mapping -----------------------------------------------------

/// The shared base: a status the caller has already decided is a failure,
/// turned into a disposition.
///
/// Two answers of its own. A 409 that reaches here is a conflict the caller
/// should re-read and retry - the one 409 this provider produces on its own
/// account is intercepted before it gets here, by the put ladder. And a 3xx is
/// a redirect the client refused rather than followed, which arrives as a
/// status because a refusing client returns the response rather than erroring
/// on it.
///
/// NO ERROR CODE IS READ OUT OF THE BODY, where the other backend reads one:
/// that protocol defines a machine-readable code element that every server
/// fills in, and this one's error bodies are an optional element that most
/// servers answer with a human-readable page instead. A `None` code is honest;
/// a code scraped out of an HTML page would not be.
pub fn classify(status: u16) -> ProviderError {
    match status {
        409 => ProviderError::Conflict,
        300..=399 => ProviderError::Blocked(format!(
            "blocked: the remote answered {status}, which this client neither follows nor reads"
        )),
        _ => ProviderError::Remote { status, code: None },
    }
}

/// The refusal for a 409 that survives the collection ladder: the parents
/// exist, so the only reading left is that one of them is an ordinary file
/// where a collection was needed. Nothing about trying again changes that.
fn not_a_collection(key: &str) -> ProviderError {
    let parent = ancestors(key).pop().unwrap_or_else(|| "/".to_string());
    ProviderError::Blocked(format!(
        "blocked: \"{parent}\" exists but is not a collection, so \"{key}\" cannot be stored under it"
    ))
}

/// A get's outcome. `Ok(None)` means the key is simply not there.
///
/// An authentication failure is NOT that: it goes to `classify` and comes back
/// as a remote failure naming its status, so a wrong password is reported as a
/// wrong password rather than as an empty remote.
pub fn classify_get(status: u16) -> Result<Option<()>, ProviderError> {
    match status {
        200..=299 => Ok(Some(())),
        404 => Ok(None),
        _ => Err(classify(status)),
    }
}

/// A put's outcome. `Ok(None)` means the parent collections are missing and the
/// caller should make them.
///
/// `retried` is needed because a status alone cannot tell the two calls apart,
/// and a 409 means something different in each. Before the ladder it is a
/// missing parent, which is the ordinary first write against a fresh remote.
/// After the ladder the parents exist, so the only reading left is that one of
/// them is not a collection at all - an ordinary file where a directory was
/// expected. That is not something a retry fixes, so it is reported as a
/// refusal naming the parent rather than as a conflict, whose documented
/// disposition is to re-read and try again.
pub fn classify_put(status: u16, key: &str, retried: bool) -> Result<Option<()>, ProviderError> {
    match status {
        200..=299 => Ok(Some(())),
        409 if !retried => Ok(None),
        409 => Err(not_a_collection(key)),
        _ => Err(classify(status)),
    }
}

/// A create's outcome.
///
/// `Ok(None)` IS NOT REUSED FOR A MISSING PARENT, where [`classify_put`] uses
/// exactly that value for exactly that case. The create's shell has to tell a
/// refused create - an object is there, which is a real answer - from a
/// collection it should build before trying again, and overloading one value
/// for both would have it build a collection in front of an object that already
/// exists, on the one path where the object being there is the point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutIfAbsent {
    /// The object was written.
    Stored,
    /// An object is already at the key. Nothing was written.
    Exists,
    /// The parent collections are missing; build them and try once more.
    NeedsParents,
}

/// A create's outcome, mapped from its status.
///
/// 412 IS THE ANSWER AND NOT A FAILURE, which is why it does not reach
/// [`classify`]: the shared mapper renders a 412 as an anonymous remote failure
/// the user is asked to resolve, and here a refusal is precisely what was
/// asked for - another device got there first, which is a state the caller acts
/// on rather than an error it reports.
///
/// `retried` carries the same distinction it does in [`classify_put`], and for
/// the same reason: before the collection ladder a 409 is a missing parent,
/// after it the parents exist and a second 409 can only mean one of them is an
/// ordinary file where a collection was needed. Nothing about trying again
/// changes that, so it is reported as a refusal naming the parent rather than
/// as a conflict, whose documented disposition is to re-read and retry.
pub fn classify_put_if_absent(
    status: u16,
    key: &str,
    retried: bool,
) -> Result<PutIfAbsent, ProviderError> {
    match status {
        200..=299 => Ok(PutIfAbsent::Stored),
        412 => Ok(PutIfAbsent::Exists),
        409 if !retried => Ok(PutIfAbsent::NeedsParents),
        409 => Err(not_a_collection(key)),
        _ => Err(classify(status)),
    }
}

/// A listing's outcome. `Ok(None)` means the collection is not there, which
/// this provider reads as an empty inventory.
///
/// THAT MAPPING IS A REAL WEAKENING AND IS TAKEN DELIBERATELY. The other
/// backend refuses exactly this, because its protocol tells a missing container
/// apart from an empty one by its error code and only the first is a real
/// failure. Here there is no such code, so a mistyped base path and an empty
/// collection are indistinguishable at this layer.
///
/// What makes it safe is one layer up rather than an argument at this one: the
/// layer above writes a keyfile before any listing can run, through the put
/// ladder, so by the time this is ever reached the base path has already been
/// proven reachable and writable. A 404 here therefore means nothing has been
/// pushed yet, which is precisely the empty case.
///
/// The residue, named rather than argued away: a user who edits their prefix
/// after a successful setup, to another value under the same reachable base
/// path, gets an empty inventory instead of an error - which is also the
/// correct reading of a genuinely fresh prefix. The cost is the one
/// `parse_multistatus` names: an empty inventory is republished whole
/// under the new prefix.
pub fn classify_list(status: u16) -> Result<Option<()>, ProviderError> {
    // The same map as a get: a 404 is the ordinary "nothing is there" answer,
    // which for a listing is an empty inventory.
    classify_get(status)
}

/// A delete's outcome. Removing something already gone is not an error.
///
/// A MULTI-STATUS IS A FAILURE HERE even though it is a success status.
/// It reports a per-resource outcome for a request that touched several, and
/// every delete this provider issues names exactly one non-collection - so a
/// multi-status means the server did something other than what was asked, and
/// reading it as success would report a delete that did not happen.
pub fn classify_delete(status: u16) -> Result<(), ProviderError> {
    match status {
        207 => Err(classify(status)),
        200..=299 => Ok(()),
        404 => Ok(()),
        _ => Err(classify(status)),
    }
}

/// A collection creation's outcome.
///
/// A 405 IS SUCCESS, which reads wrong and is not. The verb may only be run
/// against a url that maps to nothing, so a server answers 405 when the
/// collection is already there - which is the state the caller wanted. It is
/// also what two devices racing the same first write see, one each, and both
/// may proceed.
///
/// A 409 IS NOT A CONFLICT TO RETRY, which is why this does not fall through to
/// the shared mapper for it. The ladder is walked shallowest first, so by the
/// time a rung is attempted every rung above it has already succeeded - and the
/// only reading left for a refusal is that the parent is an ordinary file where
/// a collection was needed. Nothing about trying again changes that, so it is
/// reported as a refusal naming the parent. Without this the case is reached
/// BEFORE the put that would have said so, for any prefix whose non-collection
/// is not the deepest one.
pub fn classify_mkcol(status: u16, collection: &str) -> Result<(), ProviderError> {
    match status {
        201 | 405 => Ok(()),
        409 => {
            let parent = ancestors(collection)
                .pop()
                .unwrap_or_else(|| "/".to_string());
            Err(ProviderError::Blocked(format!(
                "blocked: \"{parent}\" exists but is not a collection, so \"{collection}\" cannot be created under it"
            )))
        }
        _ => Err(classify(status)),
    }
}

/// The failure shapes a request can have before it ever gets a status.
///
/// The timeout and connect arms live in the shared shell in
/// `src-tauri/src/modules/sync/providers/http.rs`; this backend adds no
/// redirect arm, because a client built to follow no redirect hands the 3xx
/// back as an ordinary response and the classifier reads it there.
pub use super::super::http::transport_error;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::sync::providers::http::guard_failure;

    // --- status mapping ---------------------------------------------------

    #[test]
    fn a_missing_object_is_the_ordinary_answer_and_an_unauthorized_one_is_not() {
        assert_eq!(classify_get(404), Ok(None));
        assert_eq!(classify_get(200), Ok(Some(())));
        let err = classify_get(401).expect_err("401 is a failure");
        assert!(
            matches!(err, ProviderError::Remote { status: 401, .. }),
            "{err:?}"
        );
    }

    #[test]
    fn a_missing_collection_lists_as_empty() {
        assert_eq!(classify_list(404), Ok(None));
        // A listing answers multi-status, which is a success code.
        assert_eq!(classify_list(207), Ok(Some(())));
        assert!(classify_list(403).is_err());
    }

    #[test]
    fn deleting_something_already_gone_is_not_an_error_but_a_partial_delete_is() {
        assert_eq!(classify_delete(404), Ok(()));
        assert_eq!(classify_delete(204), Ok(()));
        assert!(classify_delete(207).is_err());
    }

    #[test]
    fn a_collection_that_already_exists_is_a_created_collection() {
        assert_eq!(classify_mkcol(201, "vault/v1"), Ok(()));
        assert_eq!(classify_mkcol(405, "vault/v1"), Ok(()));

        // A rung refused after every rung above it succeeded can only mean the
        // parent is an ordinary file, and that must NOT come back as the
        // retry-and-hope disposition: the ladder is walked before the put that
        // would otherwise be the one to say so, so for any prefix whose
        // non-collection is not the deepest ancestor this is the only place the
        // user is told what is actually wrong.
        let err = classify_mkcol(409, "vault/v1").expect_err("a refused rung is a failure");
        assert!(
            matches!(&err, ProviderError::Blocked(m) if m.contains("vault")),
            "{err:?}"
        );
        assert_ne!(err, ProviderError::Conflict);
    }

    #[test]
    fn a_conflict_means_make_the_parents_once_and_then_means_the_parent_is_a_file() {
        let key = "vault/v1/obj/ab12";
        assert_eq!(classify_put(409, key, false), Ok(None));
        assert_eq!(classify_put(201, key, false), Ok(Some(())));
        assert_eq!(classify_put(204, key, true), Ok(Some(())));
        assert!(classify_put(507, key, false).is_err());

        // After the ladder the disposition must NOT be the retry one: the
        // parents exist, so a conflict means one of them is not a collection.
        let err = classify_put(409, key, true).expect_err("a second conflict is a failure");
        assert!(
            matches!(&err, ProviderError::Blocked(m) if m.contains("vault/v1/obj")),
            "{err:?}"
        );
    }

    #[test]
    fn a_create_answers_exists_on_412_where_a_put_reports_a_failure() {
        let key = "vault/v1/keyfile";
        assert_eq!(
            classify_put_if_absent(201, key, false),
            Ok(PutIfAbsent::Stored)
        );
        assert_eq!(
            classify_put_if_absent(204, key, false),
            Ok(PutIfAbsent::Stored)
        );
        assert_eq!(
            classify_put_if_absent(412, key, false),
            Ok(PutIfAbsent::Exists)
        );
        // 412 reads the same before and after the ladder: an object is an
        // object, and the retry it refuses is not a stale copy of our own.
        assert_eq!(
            classify_put_if_absent(412, key, true),
            Ok(PutIfAbsent::Exists)
        );
        assert_eq!(
            classify_put_if_absent(409, key, false),
            Ok(PutIfAbsent::NeedsParents)
        );
        assert!(classify_put_if_absent(507, key, false).is_err());

        // The same status the create reads as an answer is a failure through
        // the shared mapper, which is why the create does not route it there.
        assert_eq!(
            classify(412),
            ProviderError::Remote {
                status: 412,
                code: None
            }
        );

        // After the ladder the parents exist, so a second 409 means one of them
        // is an ordinary file.
        let err = classify_put_if_absent(409, "vault/v1/obj/ab12", true)
            .expect_err("a second conflict is a failure");
        assert!(
            matches!(&err, ProviderError::Blocked(m) if m.contains("vault/v1/obj")),
            "{err:?}"
        );
        assert_ne!(err, ProviderError::Conflict);
    }

    #[test]
    fn a_refused_redirect_arrives_as_a_status_and_not_as_a_transport_failure() {
        // The client follows none, and a client that follows none does not
        // error on one - it hands the response back with its 3xx status.
        for status in [302, 307] {
            assert!(
                matches!(classify(status), ProviderError::Blocked(_)),
                "{status} was not refused"
            );
        }
        // Every shape that never got a status is a transport failure, and none
        // of them is a policy refusal.
        for (timeout, connect, why) in [
            (true, false, "timed out"),
            (false, true, "connection refused"),
            (false, false, "the stream broke"),
        ] {
            assert!(
                matches!(
                    transport_error(timeout, connect, why.to_string()),
                    ProviderError::Transport(_)
                ),
                "{why}"
            );
        }
        // The guard's own vocabulary decides its disposition: a refusal is a
        // policy decision, a resolution that did not answer is a dropped
        // network worth retrying.
        assert!(matches!(
            guard_failure("blocked: link-local / cloud-metadata address".to_string()),
            ProviderError::Blocked(_)
        ));
        assert!(matches!(
            guard_failure("dns resolve failed: no such host".to_string()),
            ProviderError::Transport(_)
        ));
    }
}
