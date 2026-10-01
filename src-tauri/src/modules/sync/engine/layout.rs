//! Where one record sits on the remote: the versioned prefix, the object key,
//! the keyfile key, and the `kind:id` slot the payload and the etag map speak.

/// How long a tombstone stays meaningful, in milliseconds.
///
/// It governs two different windows: when a local tombstone stops being read,
/// and when a REMOTE tombstone object is removed. Two different windows would
/// leave objects on the remote that no device still reads, or remove objects
/// devices are still comparing against.
pub(super) const TOMBSTONE_TTL_MS: u64 = 90 * 24 * 60 * 60 * 1000;

// ---------------------------------------------------------------------------
// Object layout
// ---------------------------------------------------------------------------

// `<prefix>/v1/obj/<name>`, composed HERE rather than in a provider or in
// `crypto.rs`: a provider sees keys and bytes and has no idea what a record is,
// and `crypto.rs` produces the `<name>` half and builds no path. The `v1`
// segment is the wire format's version expressed in the object namespace, so a
// format break lands beside the old objects instead of on top of them.

/// The user's prefix with the separators normalized away, so `"subclave"`,
/// `"/subclave"` and `"subclave/"` name one place rather than three.
fn root(prefix: &str) -> String {
    let trimmed = prefix.trim_matches('/');
    if trimmed.is_empty() {
        "v1".into()
    } else {
        format!("{trimmed}/v1")
    }
}

/// What a LIST is asked for. The trailing slash is load bearing: without it a
/// sibling prefix sharing this one's first characters would be listed too.
pub(super) fn object_prefix(prefix: &str) -> String {
    format!("{}/obj/", root(prefix))
}

pub(super) fn object_key(prefix: &str, name: &str) -> String {
    format!("{}{name}", object_prefix(prefix))
}

/// Where the keyfile sits: beside the object namespace, not inside it.
///
/// OUTSIDE `obj/` deliberately. The pull lists that prefix and hands every key
/// it finds to `open_envelope`, and a keyfile is not a sealed record - it would
/// quarantine on every pull, forever, and the quarantine list is a user-facing
/// surface.
pub(super) fn keyfile_key(prefix: &str) -> String {
    format!("{}/keyfile", root(prefix))
}

/// The `<name>` half of a listed key, which is what the etag map is matched on.
pub(super) fn name_of(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// Where one record sits in the etag map: `kind:id`.
///
/// NOT the object name, and that is the point. The caller stores this map and
/// hands it back on the next pull, and an object name is an HMAC under a key
/// the caller never sees. `kind:id` it can compute, so the exclusion the apply
/// path owes - drop every refused record from the map - is a plain lookup.
pub(super) fn slot(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::sync::engine::test_support::PREFIX;

    #[test]
    fn the_object_layout_is_versioned_and_the_prefix_is_normalized() {
        // Three spellings of one prefix have to name one place, or a user who
        // typed a trailing slash gets a second, empty inventory.
        for spelling in [PREFIX, "/subclave", "subclave/", "/subclave/"] {
            assert_eq!(object_prefix(spelling), "subclave/v1/obj/");
            assert_eq!(object_key(spelling, "abcd"), "subclave/v1/obj/abcd");
        }
        assert_eq!(object_prefix(""), "v1/obj/");
        // The trailing slash on the LIST prefix, which is what keeps a sibling
        // prefix sharing these characters out of this inventory.
        assert!(object_prefix(PREFIX).ends_with('/'));
        assert_eq!(name_of("subclave/v1/obj/abcd"), "abcd");
    }
}
