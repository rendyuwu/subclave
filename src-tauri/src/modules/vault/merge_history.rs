//! The history-union merge for two copies of one entry.
//!
//! PURE: no clock, no filesystem, no network, no randomness. The merge is a
//! function of its two arguments alone, which is what lets two devices that
//! merged the same pair independently reach the same record, and what lets
//! `crate::modules::sync::model::merge` publish a union instead of a
//! winner-takes-all edit.
//!
//! WHY THE HISTORY IS THE HARD PART. Every other field of an [`Entry`]
//! resolves by last-write-wins: the newer stamp takes the field whole and the
//! older copy's value is gone. The history is the one field where the older
//! copy has to survive, because it is the record of edits the user made on
//! BOTH devices. So the winner keeps its own fields and its stamp, and the two
//! histories are unioned underneath it.
//!
//! THE MERGE IS AN ALGEBRA, NOT A RACE RESOLUTION. For any two entries the
//! result does not depend on the argument order (commutative); for any three,
//! the order they are folded in does not change the result (associative); and
//! folding an already merged result with one of its inputs changes nothing
//! (idempotent). That is what the property tests below hold, and it is what
//! the pull's apply path rests on: it re-merges a pull result into a payload
//! that the user may have edited while the network call was in flight, and
//! the properties are the guarantee that the edit survives.

use std::collections::BTreeMap;

use crate::modules::vault::model::version_of;
use crate::modules::vault::model::VersionReason::{Conflict, Edit, Restore};
use crate::modules::vault::model::{Entry, EntryVersion, VersionReason};

/// The most versions one entry keeps, matching the cap the vault applies when
/// it records an edit in `vault_entry_upsert_inner`.
const HISTORY_CAP: usize = 10;

/// The compact JSON for an entry, with its history removed.
///
/// HISTORY IS EXCLUDED, and the exclusion is load-bearing. This function is
/// the second half of [`entry_key`], the key that decides which of two live
/// copies wins. The merge accumulates histories, so a key that read one would
/// make the pair winner depend on the union already collected: merging a
/// result with one of its inputs again could flip the winner and change the
/// record, and two devices that folded the same three copies in a different
/// order could keep different histories. Everything else in the entry is
/// exactly what rides along from the winner, so everything else being in the
/// key is what makes the three properties hold.
///
/// KEY ORDER IS SERDE_JSON'S DEFAULT, and Cargo features are additive across
/// the whole dependency graph, so a crate elsewhere enabling `preserve_order`
/// would change this build silently: that feature swaps the map behind `Value`
/// for an insertion-ordered one, so two devices that built the same record by
/// different routes would compute different strings and the tie-break would
/// stop being reproducible.
pub(crate) fn canonical(e: &Entry) -> String {
    let mut value = serde_json::to_value(e).expect("entry serialization");
    if let Some(obj) = value.as_object_mut() {
        obj.remove("history");
    }
    value.to_string()
}

/// What decides which of two live copies of one entry wins: stamp first, then
/// the canonical content.
///
/// A total order over content, so two devices comparing the same pair cannot
/// disagree. `updated_at` is a plain `u64` and not an `Option`, unlike the
/// envelope's, because a live record always carries one.
pub(crate) fn entry_key(e: &Entry) -> (u64, String) {
    (e.updated_at, canonical(e))
}

/// Which reason wins when two versions share a stamp.
fn rank(reason: VersionReason) -> u8 {
    match reason {
        Edit => 3,
        Restore => 2,
        Conflict => 1,
    }
}

/// The compact JSON of one history version, the other half of the dedupe key.
fn version_canonical(v: &EntryVersion) -> String {
    serde_json::to_value(v)
        .expect("version serialization")
        .to_string()
}

/// Resolve two live copies of one entry.
///
/// The winner is the greater [`entry_key`], and it keeps its own fields and
/// stamp. Its history becomes the union of both histories, plus a
/// [`Conflict`] snapshot of the loser when the two stamps differ; a loser with
/// the winner's own stamp is already represented by the winner's state, so no
/// extra version is minted for it. [`dedupe_and_cap`] then collapses the
/// union.
pub fn merge_entries(local: &Entry, remote: &Entry) -> Entry {
    let (mut winner, loser) = if entry_key(remote) > entry_key(local) {
        (remote.clone(), local)
    } else {
        (local.clone(), remote)
    };
    if winner.updated_at != loser.updated_at {
        winner.history.push(version_of(loser, Conflict));
    }
    winner.history.extend(loser.history.iter().cloned());
    winner.history = dedupe_and_cap(winner.history);
    winner
}

/// Collapse versions that share a stamp, then keep the newest [`HISTORY_CAP`].
///
/// ONE VERSION PER STAMP, chosen by `(rank, canonical)`: `edit` beats
/// `restore` beats `conflict`, and a tie inside one rank goes to the greater
/// canonical version. The survivor has to be a function of the SET of versions
/// and not of the order they were pushed in, or two devices that collected
/// them in different orders would keep different histories, so nothing here
/// reads the input order.
fn dedupe_and_cap(versions: Vec<EntryVersion>) -> Vec<EntryVersion> {
    let mut best: BTreeMap<u64, EntryVersion> = BTreeMap::new();
    for version in versions {
        let candidate = (rank(version.reason.clone()), version_canonical(&version));
        let replace = match best.get(&version.updated_at) {
            None => true,
            Some(current) => candidate > (rank(current.reason.clone()), version_canonical(current)),
        };
        if replace {
            best.insert(version.updated_at, version);
        }
    }
    let mut out: Vec<EntryVersion> = best.into_values().collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.updated_at));
    out.truncate(HISTORY_CAP);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::vault::model::{CustomField, EntryUrl, MatchMode};

    const CHARS: &[u8] = b"abcdefghij";

    /// xorshift64* deterministic fill, seeded per test, as in
    /// `src-tauri/src/modules/generator.rs`.
    fn xorshift(seed: u64) -> impl FnMut(&mut [u8]) {
        let mut state = seed | 1;
        move |buf: &mut [u8]| {
            for slot in buf.iter_mut() {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                *slot = (state.wrapping_mul(0x2545F4914F6CDD1D) >> 56) as u8;
            }
        }
    }

    fn text(draw: &mut impl FnMut(u64) -> u64, max_len: u64) -> String {
        let len = draw(max_len + 1);
        (0..len)
            .map(|_| CHARS[draw(CHARS.len() as u64) as usize] as char)
            .collect()
    }

    /// One random entry, seeded. The id is fixed so any two draws are two
    /// copies of the SAME entry, which is the pair the merge exists for.
    ///
    /// `updated_at` and every history stamp are drawn from `0..=8`, so ties
    /// are the common case rather than an exotic one, and the history is drawn
    /// up to 12 versions so the cap is exercised.
    fn entry_with(seed: u64) -> Entry {
        let mut rng = xorshift(seed);
        let mut draw = move |n: u64| {
            let mut buf = [0u8; 8];
            rng(&mut buf);
            u64::from_le_bytes(buf) % n
        };
        let title = text(&mut draw, 8);
        let username = text(&mut draw, 8);
        let password = text(&mut draw, 8);
        let notes = text(&mut draw, 8);
        let updated_at = draw(9);
        let urls = (0..draw(4))
            .map(|_| EntryUrl {
                url: text(&mut draw, 8),
                match_mode: [MatchMode::Domain, MatchMode::Host, MatchMode::Exact]
                    [draw(3) as usize]
                    .clone(),
            })
            .collect();
        let totp = (draw(2) == 1).then(|| text(&mut draw, 8));
        let custom_fields = (0..draw(4))
            .map(|_| CustomField {
                name: text(&mut draw, 3),
                value: text(&mut draw, 3),
                hidden: draw(2) == 1,
            })
            .collect();
        let history = (0..draw(13))
            .map(|_| EntryVersion {
                updated_at: draw(9),
                reason: [
                    VersionReason::Edit,
                    VersionReason::Restore,
                    VersionReason::Conflict,
                ][draw(3) as usize]
                    .clone(),
                title: text(&mut draw, 4),
                username: text(&mut draw, 4),
                password: text(&mut draw, 4),
                urls: Vec::new(),
                notes: text(&mut draw, 4),
                totp: (draw(2) == 1).then(|| text(&mut draw, 4)),
                custom_fields: (0..draw(3))
                    .map(|_| CustomField {
                        name: text(&mut draw, 3),
                        value: text(&mut draw, 3),
                        hidden: draw(2) == 1,
                    })
                    .collect(),
            })
            .collect();
        Entry {
            id: "e1".into(),
            group_id: "g1".into(),
            title,
            username,
            password,
            urls,
            notes,
            totp,
            custom_fields,
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

    fn version(updated_at: u64, reason: VersionReason) -> EntryVersion {
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

    #[test]
    fn commutative_over_random_pairs() {
        for i in 0..20_000u64 {
            let a = entry_with(2 * i + 1);
            let b = entry_with(2 * i + 2);
            assert_eq!(merge_entries(&a, &b), merge_entries(&b, &a));
        }
    }

    #[test]
    fn associative_over_random_triples() {
        for i in 0..20_000u64 {
            let a = entry_with(3 * i + 1);
            let b = entry_with(3 * i + 2);
            let c = entry_with(3 * i + 3);
            assert_eq!(
                merge_entries(&merge_entries(&a, &b), &c),
                merge_entries(&a, &merge_entries(&b, &c))
            );
        }
    }

    #[test]
    fn idempotent_against_either_input() {
        for i in 0..20_000u64 {
            let a = entry_with(5 * i + 1);
            let b = entry_with(5 * i + 2);
            let merged = merge_entries(&a, &b);
            assert_eq!(merge_entries(&merged, &a), merged);
            assert_eq!(merge_entries(&merged, &b), merged);
        }
    }

    #[test]
    fn history_never_exceeds_ten() {
        for i in 0..20_000u64 {
            let a = entry_with(7 * i + 1);
            let b = entry_with(7 * i + 2);
            assert!(
                merge_entries(&a, &b).history.len() <= HISTORY_CAP,
                "a pair exceeded the cap"
            );
            let c = entry_with(7 * i + 3);
            assert!(
                merge_entries(&merge_entries(&a, &b), &c).history.len() <= HISTORY_CAP,
                "a triple exceeded the cap"
            );
        }
    }

    #[test]
    fn propagated_edit_keeps_edit() {
        // A was edited at 2, B is the older copy at 1. The conflict snapshot
        // the merge would mint from B sits at B's own stamp, where A's
        // recorded edit already is, so the edit must win that slot.
        let mut a = entry_with(1);
        a.updated_at = 2;
        a.history = vec![version(1, VersionReason::Edit)];
        let mut b = entry_with(2);
        b.updated_at = 1;
        b.history = Vec::new();

        let merged = merge_entries(&a, &b);
        assert_eq!(merged.updated_at, 2);
        assert_eq!(merged.history.len(), 1);
        assert_eq!(merged.history[0].updated_at, 1);
        assert_eq!(merged.history[0].reason, VersionReason::Edit);
    }

    #[test]
    fn concurrent_edits_keep_the_loser_as_conflict() {
        // Both sides edited at the same old stamp and both are now at
        // different newer stamps. The loser's live state must be kept as a
        // conflict, and the shared old edit must collapse to one version.
        let mut a = entry_with(3);
        a.updated_at = 2;
        a.history = vec![version(1, VersionReason::Edit)];
        let mut b = entry_with(4);
        b.updated_at = 3;
        b.history = vec![version(1, VersionReason::Edit)];

        let merged = merge_entries(&a, &b);
        assert_eq!(merged.updated_at, 3);
        let reasons: Vec<(u64, VersionReason)> = merged
            .history
            .iter()
            .map(|v| (v.updated_at, v.reason.clone()))
            .collect();
        assert!(reasons.contains(&(1, VersionReason::Edit)));
        assert!(reasons.contains(&(2, VersionReason::Conflict)));
        assert_eq!(
            reasons.iter().filter(|(at, _)| *at == 1).count(),
            1,
            "the shared old edit was not collapsed"
        );
    }

    #[test]
    fn equal_stamps_add_no_extra_version() {
        // Two copies stamped the same already agree on the current state, so
        // no conflict snapshot is minted; only the histories are unioned.
        let mut a = entry_with(5);
        a.updated_at = 4;
        a.history = vec![version(2, VersionReason::Restore)];
        let mut b = entry_with(6);
        b.updated_at = 4;
        b.history = vec![version(3, VersionReason::Edit)];

        let merged = merge_entries(&a, &b);
        let reasons: Vec<(u64, VersionReason)> = merged
            .history
            .iter()
            .map(|v| (v.updated_at, v.reason.clone()))
            .collect();
        assert!(
            !reasons.iter().any(|(_, r)| *r == VersionReason::Conflict),
            "an equal-stamp pair minted a conflict"
        );
        assert!(reasons.contains(&(2, VersionReason::Restore)));
        assert!(reasons.contains(&(3, VersionReason::Edit)));
    }
}
