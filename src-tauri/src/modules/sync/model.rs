//! The wire format two devices agree on, and the merge that resolves one
//! record's two copies.
//!
//! PURE: no clock, no filesystem, no network, no randomness. Everything here
//! is a function of its arguments, which is what makes the merge testable at
//! all and what makes two devices running it on the same pair agree.
//!
//! THE RECORD BODY IS OPAQUE. [`Envelope::record`] is a `serde_json::Value`
//! and [`Envelope::kind`] is a `String`, rather than a Rust type mirroring
//! each record. A second schema would have to be hand-maintained against the
//! first, and a field added on one side and not the other would be silently
//! dropped on every round trip through here. The one exception is a pair of
//! live entries, which [`merge`] hands to
//! `crate::modules::vault::merge_history::merge_entries`; that function does
//! read the whole record, so a rename in `src/modules/vault/types.ts` breaks
//! it loudly instead of silently.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::modules::vault::model::Entry;

/// Bumped when a change to this format stops an older build from reading it.
///
/// A VALUE CHECK, not a guess: [`merge`] refuses an envelope it does not
/// recognise rather than assuming the fields it knows mean what they used to.
pub const WIRE_VERSION: u32 = 1;

/// The record kinds this build knows how to store.
///
/// `kind` is a plain string so a newer build can add one without this one
/// guessing at what its fields mean; the cost is [`known_kind`], which the
/// pull uses to quarantine a record whose kind it cannot interpret.
pub const ENTRY_KIND: &str = "entry";
pub const GROUP_KIND: &str = "group";

/// Whether this build has a rule for `kind`.
pub fn known_kind(kind: &str) -> bool {
    kind == ENTRY_KIND || kind == GROUP_KIND
}

/// Fields a record carries that describe THIS MACHINE rather than the record,
/// removed before a record is published.
///
/// Keyed on the field name alone rather than on `kind`, because every kind
/// that reuses the name means the same device-local thing by it. `lastUsedAt`
/// comes from [`crate::modules::vault::model::Entry`] and names when THIS
/// device last used the entry, so it must never travel: another device would
/// claim a use it never had. Letting it travel later is additive and not a
/// format break.
const DEVICE_LOCAL_FIELDS: [&str; 1] = ["lastUsedAt"];

/// One record, as it travels.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    /// [`WIRE_VERSION`] at the time this was written.
    pub v: u32,
    /// Which store owns the record: [`ENTRY_KIND`] or [`GROUP_KIND`].
    pub kind: String,
    pub id: String,
    /// Unix ms of the record's last content change, copied from the record's
    /// own `updatedAt`, or the tombstone's `deletedAt`.
    ///
    /// ABSENT IS NOT ZERO and is never backfilled: it orders BELOW any stamp,
    /// because absent means "written before the field existed" and must never
    /// outrank a real one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
    /// Which device published this. PROVENANCE ONLY: the merge does not read
    /// it, and [`ordering_key`] says why not. The one reader is the prune, and
    /// there the question being asked IS provenance.
    ///
    /// `default` so a frontend that assembles an envelope may leave it out.
    /// That is not leniency: the push STAMPS this field over whatever arrived,
    /// because the prune deletes remote objects on the strength of it and a
    /// value a webview could choose is a value a webview could get wrong.
    #[serde(default)]
    pub device: String,
    /// A tombstone. `record` is `Null` and `updated_at` is the `deletedAt`.
    #[serde(default)]
    pub deleted: bool,
    /// The record with every device-local field already removed, see
    /// [`strip_device_local`]. `Null` when `deleted`.
    pub record: Value,
}

/// Why an envelope pair could not be merged.
///
/// A typed error rather than a `String` because the caller has to tell
/// dispositions apart: "the other device runs a newer build, skip this object
/// and say so", "this object is corrupt, quarantine it", and "this object was
/// overwritten by a different record". String-matching for that would be
/// decided once every call site was already written.
///
/// A refusal is PER OBJECT. One bad object must not block the rest of an
/// inventory, so the caller quarantines the object and carries on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// One side is not [`WIRE_VERSION`]. Unknown is not guessable, which is
    /// the entire reason [`Envelope::v`] exists.
    Version { found: u32, expected: u32 },
    /// The two sides disagree on `kind` or `id`. An object name is derived
    /// from both, so a decrypted envelope disagreeing with its sibling is the
    /// signal that an object was overwritten by a DIFFERENT record. Picking
    /// one of the two is corruption laundering.
    IdentityMismatch,
    /// `deleted` with no `updated_at`. The delete would lose to every stamped
    /// record, which is a delete that quietly fails to propagate.
    UnstampedTombstone,
    /// Not `deleted`, yet `record` is `Null`. The mirror of the above and
    /// worse: the applied record would be written over a live one.
    NullLiveRecord,
    /// A live entry whose `record` does not deserialize as
    /// [`crate::modules::vault::model::Entry`]. There is no rule at this layer
    /// for repairing it, so the object is quarantined rather than stored.
    BadRecord,
    /// A `kind` this build has no rule for, see [`known_kind`]. A newer
    /// build's record must not be written into a store this one cannot
    /// interpret.
    UnknownKind,
}

/// The resolved record.
///
/// NO SIDE AND NO CONTENT FLAGS. Which of the two copies won is provenance
/// nothing downstream needs, and the one question a caller has is whether the
/// store now holds something different, which is [`Merged::changed`].
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    pub envelope: Envelope,
    /// Whether the result differs from `local` in what the STORE holds.
    ///
    /// Computed over `(updated_at, deleted, canonical(record))` and NOT over
    /// whole envelopes: `device` always differs between two devices, so a
    /// whole-envelope comparison would be `true` on every remote win and the
    /// store would be rewritten on every pull.
    pub changed: bool,
}

/// The compact JSON for a value, which is what the ordering key compares.
///
/// `Value`'s `Display` is serde_json's own compact serializer, so this is
/// `serde_json::to_string` without the `Result` that cannot fire for a
/// `Value` that is already in hand.
///
/// STABILITY RESTS ON TWO SERDE_JSON DEFAULTS, and Cargo features are additive
/// across the whole dependency graph, so a crate elsewhere enabling either
/// would change this build silently:
///
/// - `preserve_order` swaps the map behind `Value` for an insertion-ordered
///   one, making key order insertion-dependent, so two devices that built the
///   same record by different routes would compute different strings;
/// - `arbitrary_precision` turns `Number` into a text-preserving wrapper, so
///   `1` and `1.0` stop unifying.
///
/// Neither is enabled on this branch, and [`canonical_is_sorted_and_numeric`]
/// is what notices if that changes.
///
/// BENIGN RESIDUE, named so the next reader does not mistake it for a bug: a
/// field present as `null` and a field absent canonicalize differently, and a
/// `Value` parsed out of remote ciphertext took a different path than one
/// built from the local payload. Two devices can therefore see a "content
/// difference" that is serialization noise. It still converges, because both
/// sides compute the SAME key from the same pair, so all it does is make the
/// tie-break jitter.
///
/// Non-finite floats need no guard: `Number::from_f64` rejects them and JSON
/// has no NaN literal, so no `Value` can hold one.
fn canonical(v: &Value) -> String {
    v.to_string()
}

/// A tombstone's `record` is `Null`, whatever arrived in it.
///
/// Run BEFORE the ordering key is computed, not after. A tombstone can arrive
/// carrying a junk record, and normalizing afterwards would let that junk into
/// the key; harmless today, since `deleted` is compared first, but it would
/// stop the key being a function of the envelope's MEANING.
fn normalize(e: &mut Envelope) {
    if e.deleted {
        e.record = Value::Null;
    }
}

/// What decides the winner, derived from CONTENT ALONE.
///
/// That is what makes the merge commutative by construction: two devices
/// running it locally on the same pair compare the same three values in the
/// same order and cannot disagree.
///
/// - `updated_at` first. `Option<u64>` already orders `None` below any `Some`,
///   which is exactly the wanted behaviour for an unstamped record.
/// - `deleted` breaks an exact timestamp tie in favour of the DELETE: a lost
///   delete re-spreads data the user removed, while a lost resurrection costs
///   one re-create.
/// - the canonical record breaks what is left of the tie.
///
/// DEVICE IS DELIBERATELY NOT IN HERE. No record carries a `device` field, so
/// the only place one can be stamped is when the envelope is built, at push
/// time. A record pulled from A and pushed back unedited by B would come back
/// stamped `device: "B"`, so the key would not be a function of the record and
/// the merge would not be idempotent under re-push: identical content,
/// different key. A lexicographic content tie-break is exactly as arbitrary as
/// a device-id one and has none of that.
pub fn ordering_key(e: &Envelope) -> (Option<u64>, bool, String) {
    (e.updated_at, e.deleted, canonical(&e.record))
}

/// Whether two envelopes say anything different at all, `device` excluded.
///
/// The question a PUSH has to answer ("does the remote object already hold
/// this?"), and the reason [`Merged::changed`] is not this.
///
/// `device` is excluded for the reason it is outside [`ordering_key`]: it
/// always differs between two devices, so including it would make this `true`
/// for every pair and every pull would republish the whole inventory.
pub fn content_differs(a: &Envelope, b: &Envelope) -> bool {
    ordering_key(a) != ordering_key(b)
}

/// Remove every field that describes this machine rather than the record.
///
/// Called on the record before it is put in an [`Envelope`], so what travels
/// is already stripped and no reader has to remember to do it.
pub fn strip_device_local(record: &mut Value) {
    let Some(obj) = record.as_object_mut() else {
        return;
    };
    for field in DEVICE_LOCAL_FIELDS {
        obj.remove(field);
    }
}

/// Resolve one record's two copies.
///
/// Refuses before it compares anything: see [`MergeError`] for each refusal
/// and why it is one. After the identity check the two `kind`s are equal, so
/// there is only one kind left to read.
///
/// Then normalize both, compare [`ordering_key`], and resolve by kind:
///
/// - two LIVE ENTRY envelopes go to
///   `crate::modules::vault::merge_history::merge_entries`, which returns the
///   newer stamp with the union of both histories underneath it. Both records
///   must deserialize as [`crate::modules::vault::model::Entry`] or the pair
///   is refused with [`MergeError::BadRecord`]; that check runs before either
///   side is picked, so it fires whatever the argument order.
/// - everything else (groups, tombstones, a live copy against a tombstone) is
///   taken WHOLE from the greater [`ordering_key`]. A group has no history to
///   union, and a delete must not carry the record it deleted.
///
/// An exact tie resolves to LOCAL, which is arbitrary only in the sense that
/// the two envelopes are then identical on everything read here.
pub fn merge(local: &Envelope, remote: &Envelope) -> Result<Merged, MergeError> {
    for e in [local, remote] {
        if e.v != WIRE_VERSION {
            return Err(MergeError::Version {
                found: e.v,
                expected: WIRE_VERSION,
            });
        }
    }
    if local.kind != remote.kind || local.id != remote.id {
        return Err(MergeError::IdentityMismatch);
    }
    if !known_kind(&local.kind) {
        return Err(MergeError::UnknownKind);
    }
    for e in [local, remote] {
        if e.deleted && e.updated_at.is_none() {
            return Err(MergeError::UnstampedTombstone);
        }
        if !e.deleted && e.record.is_null() {
            return Err(MergeError::NullLiveRecord);
        }
    }

    let mut l = local.clone();
    let mut r = remote.clone();
    normalize(&mut l);
    normalize(&mut r);

    let union = if !l.deleted && !r.deleted && l.kind == ENTRY_KIND {
        let local_entry: Entry =
            serde_json::from_value(l.record.clone()).map_err(|_| MergeError::BadRecord)?;
        let remote_entry: Entry =
            serde_json::from_value(r.record.clone()).map_err(|_| MergeError::BadRecord)?;
        Some(crate::modules::vault::merge_history::merge_entries(
            &local_entry,
            &remote_entry,
        ))
    } else {
        None
    };

    // Taken BEFORE the winner is picked, because `l` and `r` are moved into it
    // and `changed` has to compare against the local copy as it arrived.
    let local_key = ordering_key(&l);
    let mut winner = if ordering_key(&r) > local_key { r } else { l };
    if let Some(merged) = union {
        winner.record = serde_json::to_value(&merged).expect("entry serialization");
        winner.updated_at = Some(merged.updated_at);
    }

    let changed = ordering_key(&winner) != local_key;
    Ok(Merged {
        envelope: winner,
        changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::vault::model::{EntryVersion, VersionReason};
    use serde_json::json;

    fn env(kind: &str, id: &str, updated_at: Option<u64>, record: Value) -> Envelope {
        Envelope {
            v: WIRE_VERSION,
            kind: kind.into(),
            id: id.into(),
            updated_at,
            device: "dev-a".into(),
            deleted: false,
            record,
        }
    }

    fn group(updated_at: Option<u64>, name: &str) -> Envelope {
        env(
            GROUP_KIND,
            "g-1",
            updated_at,
            json!({"id": "g-1", "name": name}),
        )
    }

    fn tombstone(kind: &str, id: &str, deleted_at: u64) -> Envelope {
        Envelope {
            deleted: true,
            record: Value::Null,
            ..env(kind, id, Some(deleted_at), Value::Null)
        }
    }

    fn entry(id: &str, title: &str, updated_at: u64, history: Vec<EntryVersion>) -> Entry {
        Entry {
            id: id.into(),
            group_id: "root".into(),
            title: title.into(),
            username: String::new(),
            password: String::new(),
            urls: Vec::new(),
            notes: String::new(),
            totp: None,
            custom_fields: Vec::new(),
            tags: Vec::new(),
            icon: None,
            color: None,
            favorite: false,
            expires_at: None,
            trashed_from: None,
            created_at: 0,
            updated_at,
            history,
            last_used_at: None,
        }
    }

    fn entry_env(e: &Entry) -> Envelope {
        env(
            ENTRY_KIND,
            &e.id,
            Some(e.updated_at),
            serde_json::to_value(e).unwrap(),
        )
    }

    fn entry_version(updated_at: u64, reason: VersionReason) -> EntryVersion {
        EntryVersion {
            updated_at,
            reason,
            title: "old".into(),
            username: String::new(),
            password: String::new(),
            urls: Vec::new(),
            notes: String::new(),
            totp: None,
            custom_fields: Vec::new(),
        }
    }

    /// Both argument orders of one pair must resolve to the same record. This
    /// is the property that makes the merge safe to run independently on two
    /// devices, so nearly every ordering test below asserts it.
    ///
    /// THE TWO SIDES ARE STAMPED WITH DIFFERENT DEVICE IDS HERE, rather than
    /// taken as the fixtures left them. In production the one field that
    /// always differs between two copies of a record is `device`, and a helper
    /// that compared whole envelopes would pass only because every fixture in
    /// this file happened to share one id, which is the shape of an assertion
    /// that cannot fail.
    ///
    /// So `device` is excluded from the comparison, and that exclusion is the
    /// property rather than a weakening of it: `device` is deliberately
    /// outside the ordering key, so on a tie each side keeps its own and the
    /// two orders CANNOT agree on it. If `device` ever crept into the key,
    /// these differing ids would make the winner flip with the argument order
    /// and every caller below would fail on the fields that are compared.
    fn agrees_both_ways(a: &Envelope, b: &Envelope) -> Envelope {
        let a = Envelope {
            device: "dev-a".into(),
            ..a.clone()
        };
        let b = Envelope {
            device: "dev-b".into(),
            ..b.clone()
        };
        let one = merge(&a, &b).expect("merge a,b").envelope;
        let two = merge(&b, &a).expect("merge b,a").envelope;
        assert_eq!(
            Envelope {
                device: String::new(),
                ..one.clone()
            },
            Envelope {
                device: String::new(),
                ..two
            },
            "the two argument orders disagreed"
        );
        one
    }

    // --- refusals ---------------------------------------------------------

    #[test]
    fn an_unknown_wire_version_is_refused_on_either_side() {
        let good = group(Some(2), "a");
        let mut bad = group(Some(3), "b");
        bad.v = WIRE_VERSION + 1;
        let expected = MergeError::Version {
            found: WIRE_VERSION + 1,
            expected: WIRE_VERSION,
        };
        assert_eq!(merge(&bad, &good), Err(expected.clone()));
        assert_eq!(merge(&good, &bad), Err(expected));
    }

    #[test]
    fn two_sides_naming_different_records_are_refused() {
        // Both halves of the identity: an object name is derived from `kind`
        // AND `id`, so either one disagreeing means the object was overwritten
        // by a different record.
        let a = group(Some(1), "a");
        let mut other_kind = a.clone();
        other_kind.kind = ENTRY_KIND.into();
        assert_eq!(merge(&a, &other_kind), Err(MergeError::IdentityMismatch));

        let mut other_id = a.clone();
        other_id.id = "g-2".into();
        assert_eq!(merge(&a, &other_id), Err(MergeError::IdentityMismatch));
    }

    #[test]
    fn a_tombstone_with_no_stamp_is_refused_in_either_position() {
        let live = group(Some(5), "a");
        let mut stampless = tombstone(GROUP_KIND, "g-1", 1);
        stampless.updated_at = None;
        assert_eq!(
            merge(&stampless, &live),
            Err(MergeError::UnstampedTombstone)
        );
        assert_eq!(
            merge(&live, &stampless),
            Err(MergeError::UnstampedTombstone)
        );
    }

    #[test]
    fn a_live_envelope_with_a_null_record_is_refused_in_either_position() {
        let live = group(Some(5), "a");
        let null_live = env(GROUP_KIND, "g-1", Some(9), Value::Null);
        assert_eq!(merge(&null_live, &live), Err(MergeError::NullLiveRecord));
        assert_eq!(merge(&live, &null_live), Err(MergeError::NullLiveRecord));
    }

    #[test]
    fn an_unknown_kind_is_refused() {
        let a = env("widget", "w-1", Some(1), json!({"id": "w-1"}));
        assert_eq!(merge(&a, &a.clone()), Err(MergeError::UnknownKind));
        assert!(!known_kind("widget"));
        assert!(known_kind(ENTRY_KIND));
        assert!(known_kind(GROUP_KIND));
    }

    #[test]
    fn a_live_entry_record_that_does_not_deserialize_is_refused() {
        // A live entry must hold an `Entry`; nothing at this layer can repair
        // a record that does not, so the object is refused in either position.
        let broken = env(ENTRY_KIND, "e-1", Some(1), json!({"id": "e-1"}));
        assert_eq!(merge(&broken, &broken.clone()), Err(MergeError::BadRecord));
    }

    // --- ordering ---------------------------------------------------------

    #[test]
    fn the_newer_stamp_wins_from_either_side() {
        let older = group(Some(10), "older");
        let newer = group(Some(20), "newer");

        assert_eq!(
            merge(&newer, &older).unwrap().envelope.record["name"],
            "newer"
        );
        assert_eq!(
            merge(&older, &newer).unwrap().envelope.record["name"],
            "newer"
        );
    }

    #[test]
    fn an_unstamped_record_loses_to_any_stamp() {
        // Absent is not zero: a record written before the field existed must
        // never outrank a real stamp, in either argument position.
        let unstamped = group(None, "legacy");
        let stamped = group(Some(1), "stamped");
        assert_eq!(
            agrees_both_ways(&unstamped, &stamped).record["name"],
            "stamped"
        );

        // And two unstamped records still resolve identically both ways,
        // through `deleted` and the canonical strings: there is no stamp left
        // to break the tie with.
        agrees_both_ways(&unstamped, &group(None, "also-legacy"));
    }

    #[test]
    fn an_exact_tie_on_different_records_resolves_the_same_way_every_time() {
        let a = group(Some(7), "alpha");
        let b = group(Some(7), "beta");
        let first = agrees_both_ways(&a, &b);
        let again = agrees_both_ways(&a, &b);
        assert_eq!(
            first, again,
            "the same pair resolved differently on a rerun"
        );
    }

    #[test]
    fn an_exact_tie_between_a_delete_and_an_edit_goes_to_the_delete() {
        // The conservative side: a lost delete re-spreads data the user
        // removed, a lost resurrection costs one re-create.
        let live = group(Some(42), "still here");
        let gone = tombstone(GROUP_KIND, "g-1", 42);
        assert!(agrees_both_ways(&live, &gone).deleted);
    }

    #[test]
    fn a_newer_tombstone_wins_and_carries_no_record() {
        // The inbound tombstone carries junk, which also proves the
        // normalization to Null happens BEFORE the ordering key is computed:
        // if it happened after, this junk would be in the key.
        let mut junk = tombstone(GROUP_KIND, "g-1", 99);
        junk.record = json!({"id": "g-1", "name": "junk that should not travel"});
        let live = group(Some(1), "local");

        let merged = merge(&live, &junk).unwrap();
        assert!(merged.envelope.deleted);
        assert_eq!(merged.envelope.record, Value::Null);
    }

    #[test]
    fn an_older_tombstone_does_not_resurrect_itself_over_a_newer_edit() {
        let live = group(Some(100), "edited after the delete");
        let stale = tombstone(GROUP_KIND, "g-1", 50);
        let merged = merge(&live, &stale).unwrap();
        assert!(!merged.envelope.deleted);
        assert_eq!(merged.envelope.record["name"], "edited after the delete");
    }

    #[test]
    fn a_group_pair_takes_the_newer_stamp_whole() {
        let older = group(Some(1), "old");
        let newer = group(Some(2), "new");
        let merged = merge(&older, &newer).unwrap();
        assert!(merged.changed);
        assert_eq!(merged.envelope.record, newer.record);
        assert_eq!(merged.envelope.updated_at, Some(2));
        assert_eq!(agrees_both_ways(&older, &newer).record, newer.record);
    }

    // --- the entry exception ---------------------------------------------

    #[test]
    fn two_live_entries_union_their_histories() {
        let a = entry("e-1", "a", 20, vec![entry_version(5, VersionReason::Edit)]);
        let b = entry(
            "e-1",
            "b",
            10,
            vec![entry_version(8, VersionReason::Restore)],
        );

        let merged = agrees_both_ways(&entry_env(&a), &entry_env(&b));
        assert_eq!(merged.record["title"], "a");
        let winner: Entry = serde_json::from_value(merged.record.clone()).unwrap();
        assert_eq!(winner.updated_at, 20);
        let stamps: Vec<(u64, VersionReason)> = winner
            .history
            .iter()
            .map(|v| (v.updated_at, v.reason.clone()))
            .collect();
        assert_eq!(stamps.len(), 3);
        assert!(stamps.contains(&(10, VersionReason::Conflict)));
        assert!(stamps.contains(&(8, VersionReason::Restore)));
        assert!(stamps.contains(&(5, VersionReason::Edit)));
    }

    #[test]
    fn changed_is_false_on_a_fixed_point() {
        let a = entry_env(&entry(
            "e-1",
            "a",
            1,
            vec![entry_version(1, VersionReason::Edit)],
        ));
        let b = entry_env(&entry("e-1", "b", 2, Vec::new()));

        let first = merge(&a, &b).unwrap();
        assert!(first.changed);

        // Merging the result with the same remote again is the fixed point:
        // the union is already there, so nothing lands.
        let again = merge(&first.envelope, &b).unwrap();
        assert!(!again.changed, "a fixed point reported as a change");
        assert_eq!(again.envelope.record, first.envelope.record);
    }

    // --- content comparison ----------------------------------------------

    #[test]
    fn content_differs_reads_everything_except_the_device() {
        // What a push asks before it re-uploads. The `device` clause is the
        // load-bearing one: without it every pair differs and every pull
        // republishes the whole inventory.
        let a = group(Some(5), "same");
        let b = Envelope {
            device: "dev-b".into(),
            ..a.clone()
        };
        assert!(!content_differs(&a, &b), "the device id was compared");
        assert!(content_differs(&a, &group(Some(6), "same")));
        assert!(content_differs(&a, &group(Some(5), "other")));
        assert!(content_differs(&a, &tombstone(GROUP_KIND, "g-1", 5)));
    }

    // --- shape ------------------------------------------------------------

    #[test]
    fn canonical_is_sorted_and_numeric() {
        // One line covering the two hazards the ordering key depends on:
        // nested map ordering and number normalization, which is what
        // `preserve_order` and `arbitrary_precision` would break.
        let v: Value = serde_json::from_str(r#"{"b":1.0,"a":{"d":1,"c":1e3}}"#).unwrap();
        assert_eq!(canonical(&v), r#"{"a":{"c":1000.0,"d":1},"b":1.0}"#);
    }

    #[test]
    fn an_envelope_round_trips_and_the_version_is_a_plain_number() {
        // Both `None` and `Some` for the skipped field, or
        // `skip_serializing_if` goes untested.
        let absent = env(
            GROUP_KIND,
            "g-1",
            None,
            json!({"id": "g-1", "name": "prod"}),
        );
        let json_text = serde_json::to_string(&absent).unwrap();
        assert!(json_text.contains(r#""v":1"#), "unexpected: {json_text}");
        assert!(!json_text.contains("updatedAt"), "unexpected: {json_text}");
        assert_eq!(
            serde_json::from_str::<Envelope>(&json_text).unwrap(),
            absent
        );

        let present = Envelope {
            updated_at: Some(1),
            ..absent
        };
        let json_text = serde_json::to_string(&present).unwrap();
        assert!(
            json_text.contains(r#""updatedAt":1"#),
            "unexpected: {json_text}"
        );
        assert_eq!(
            serde_json::from_str::<Envelope>(&json_text).unwrap(),
            present
        );
    }

    #[test]
    fn a_field_this_module_has_no_rules_for_survives_the_whole_trip() {
        // `sortOrder` is never named in this module outside this fixture,
        // which is what makes the record opaque by construction rather than by
        // discipline. A group is taken whole, so the field rides along.
        let record = json!({"id": "g-1", "name": "n", "sortOrder": 3});
        let sent = serde_json::to_string(&env(GROUP_KIND, "g-1", Some(5), record.clone())).unwrap();
        let received: Envelope = serde_json::from_str(&sent).unwrap();
        let merged = merge(&received, &group(Some(1), "older")).unwrap();
        assert_eq!(merged.envelope.record["sortOrder"], 3);
        assert_eq!(serde_json::to_string(&merged.envelope).unwrap(), sent);
    }

    #[test]
    fn strip_removes_exactly_the_device_local_field() {
        let mut record = json!({
            "id": "e-1",
            "title": "bank",
            "username": "u",
            "updatedAt": 5,
            "lastUsedAt": 1700
        });
        // SPELLED OUT, not read back out of `DEVICE_LOCAL_FIELDS`. Iterating
        // the same constant the implementation iterates is an assertion that
        // cannot fail: a typo in the constant would remove nothing, the
        // mistyped name would drop out of the skip-list below, and the test
        // would confirm that the field it no longer strips is still present.
        // This literal is what a rename in
        // `src-tauri/src/modules/vault/model.rs` has to break.
        let expected_gone = ["lastUsedAt"];
        assert_eq!(
            expected_gone.len(),
            DEVICE_LOCAL_FIELDS.len(),
            "the strip list grew or shrank without this test noticing"
        );

        let before = record.clone();
        strip_device_local(&mut record);

        for gone in expected_gone {
            assert!(
                before.get(gone).is_some(),
                "{gone} is missing from the fixture"
            );
            assert!(record.get(gone).is_none(), "{gone} survived the strip");
        }
        for (field, value) in before.as_object().unwrap() {
            if expected_gone.contains(&field.as_str()) {
                continue;
            }
            assert_eq!(record.get(field), Some(value), "{field} was disturbed");
        }
    }

    #[test]
    fn strip_leaves_a_non_object_record_alone() {
        let mut record = Value::Null;
        strip_device_local(&mut record);
        assert_eq!(record, Value::Null);
    }
}
