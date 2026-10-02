//! Payload to envelopes and back: what this device would publish, what a pull's
//! landings write, and the dirty-mark bookkeeping a push leaves behind.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::layout::{slot, TOMBSTONE_TTL_MS};
use super::types::{Applied, Outcome, PullReport, PushReport, Quarantined, SlotRef};
use crate::modules::sync::model::{
    content_differs, merge, Envelope, ENTRY_KIND, GROUP_KIND, WIRE_VERSION,
};
use crate::modules::vault::model::{Entry, Group, Tombstone, TombstoneKind, VaultPayload};

// ---------------------------------------------------------------------------
// Payload to envelopes
// ---------------------------------------------------------------------------

/// The stored shape of one slot. `record` is `None` for a tombstone, which
/// carries no record and reads as `deleted`.
fn envelope(kind: &str, id: &str, updated_at: u64, record: Option<Value>) -> Envelope {
    Envelope {
        v: WIRE_VERSION,
        kind: kind.to_string(),
        id: id.to_string(),
        updated_at: Some(updated_at),
        device: String::new(),
        deleted: record.is_none(),
        record: record.unwrap_or(Value::Null),
    }
}

/// The record as it sits in the payload, device-local fields included.
///
/// NOT STRIPPED, and that is what the merge expects: `model::merge` builds its
/// result with `serde_json::to_value(merged)`, which puts `lastUsedAt` back on
/// an entry anyway, and `Merged::changed` compares the result against the local
/// envelope it was handed. Stripping here would make every agreed pair look
/// changed. The strip that matters happens on PUBLISH (`seal_envelope`) and on
/// a merged envelope before it is compared against a remote copy.
pub(crate) fn entry_envelope(entry: &Entry) -> Envelope {
    envelope(
        ENTRY_KIND,
        &entry.id,
        entry.updated_at,
        Some(serde_json::to_value(entry).expect("entry serialization")),
    )
}

pub(crate) fn group_envelope(group: &Group) -> Envelope {
    envelope(
        GROUP_KIND,
        &group.id,
        group.updated_at,
        Some(serde_json::to_value(group).expect("group serialization")),
    )
}

fn tombstone_envelope(tombstone: &Tombstone) -> Envelope {
    let kind = match tombstone.kind {
        TombstoneKind::Entry => ENTRY_KIND,
        TombstoneKind::Group => GROUP_KIND,
    };
    envelope(kind, &tombstone.id, tombstone.deleted_at, None)
}

/// The stored shape of one slot, tombstone included and regardless of age.
fn envelope_of_slot(payload: &VaultPayload, kind: &str, id: &str) -> Option<Envelope> {
    if kind == ENTRY_KIND {
        if let Some(entry) = payload.entries.iter().find(|e| e.id == id) {
            return Some(entry_envelope(entry));
        }
        if let Some(tombstone) = payload
            .tombstones
            .iter()
            .find(|t| t.id == id && matches!(t.kind, TombstoneKind::Entry))
        {
            return Some(tombstone_envelope(tombstone));
        }
    } else if kind == GROUP_KIND {
        if let Some(group) = payload.groups.iter().find(|g| g.id == id) {
            return Some(group_envelope(group));
        }
        if let Some(tombstone) = payload
            .tombstones
            .iter()
            .find(|t| t.id == id && matches!(t.kind, TombstoneKind::Group))
        {
            return Some(tombstone_envelope(tombstone));
        }
    }
    None
}

/// Every envelope this device would publish: live records plus tombstones
/// younger than the window.
pub fn locals_from_payload(payload: &VaultPayload, now: u64) -> Vec<Envelope> {
    let mut out =
        Vec::with_capacity(payload.entries.len() + payload.groups.len() + payload.tombstones.len());
    for entry in &payload.entries {
        out.push(entry_envelope(entry));
    }
    for group in &payload.groups {
        out.push(group_envelope(group));
    }
    for tombstone in &payload.tombstones {
        if now.saturating_sub(tombstone.deleted_at) < TOMBSTONE_TTL_MS {
            out.push(tombstone_envelope(tombstone));
        }
    }
    out
}

/// The stored shape of one slot for the merge, with expired tombstones reading
/// as absent so this view matches [`locals_from_payload`].
pub(crate) fn local_envelope(
    payload: &VaultPayload,
    kind: &str,
    id: &str,
    now: u64,
) -> Option<Envelope> {
    let envelope = envelope_of_slot(payload, kind, id)?;
    if envelope.deleted {
        let deleted_at = envelope.updated_at.unwrap_or(0);
        if now.saturating_sub(deleted_at) >= TOMBSTONE_TTL_MS {
            return None;
        }
    }
    Some(envelope)
}

/// One envelope per dirty slot that still exists, plus that subset of the etag
/// map.
///
/// Slots with neither a record nor a tombstone are dropped from `dirty`:
/// nothing is left to push, and the mark would only ever be re-dropped.
pub fn take_dirty_envelopes(
    payload: &mut VaultPayload,
) -> (Vec<Envelope>, BTreeMap<String, String>) {
    let slots: Vec<String> = payload.device.sync.dirty.iter().cloned().collect();
    let mut envelopes = Vec::with_capacity(slots.len());
    let mut etags = BTreeMap::new();
    let mut dead: Vec<String> = Vec::new();
    for key in slots {
        let Some((kind, id)) = key.split_once(':') else {
            dead.push(key);
            continue;
        };
        match envelope_of_slot(payload, kind, id) {
            Some(envelope) => {
                if let Some(etag) = payload.device.sync.etags.get(&key) {
                    etags.insert(key, etag.clone());
                }
                envelopes.push(envelope);
            }
            None => dead.push(key),
        }
    }
    for key in dead {
        payload.device.sync.dirty.remove(&key);
    }
    (envelopes, etags)
}

/// Drop the record `id` from `records`, answering whether it was there.
fn remove<T>(records: &mut Vec<T>, id: &str, id_of: impl Fn(&T) -> &str) -> bool {
    let before = records.len();
    records.retain(|record| id_of(record) != id);
    records.len() != before
}

/// Store one reconciled envelope into the payload.
///
/// `Err(reason)` = the envelope names a record this build refuses to store (a
/// live entry whose record does not deserialize, or an unknown kind); the
/// caller quarantines it. `Ok(false)` = a tombstone for a record that was not
/// there; `Ok(true)` = the stored form changed.
fn write_envelope(
    payload: &mut VaultPayload,
    envelope: &Envelope,
    now: u64,
) -> Result<bool, String> {
    if envelope.kind != ENTRY_KIND && envelope.kind != GROUP_KIND {
        return Err(format!(
            "sync: this build does not know the record kind \"{}\"",
            envelope.kind
        ));
    }
    if envelope.deleted {
        let kind = if envelope.kind == ENTRY_KIND {
            TombstoneKind::Entry
        } else {
            TombstoneKind::Group
        };
        let removed = match kind {
            TombstoneKind::Entry => remove(&mut payload.entries, &envelope.id, |e| &e.id),
            TombstoneKind::Group => remove(&mut payload.groups, &envelope.id, |g| &g.id),
        };
        payload.tombstones.retain(|t| t.id != envelope.id);
        payload.tombstones.push(Tombstone {
            id: envelope.id.clone(),
            kind,
            deleted_at: envelope.updated_at.unwrap_or(now),
        });
        return Ok(removed);
    }
    if envelope.kind == ENTRY_KIND {
        let mut entry: Entry = serde_json::from_value(envelope.record.clone()).map_err(|_| {
            format!(
                "sync: the record for entry \"{}\" could not be read",
                envelope.id
            )
        })?;
        // The wire copy carries no `lastUsedAt`; keep the value this device
        // already had rather than erasing it on every pull.
        if entry.last_used_at.is_none() {
            entry.last_used_at = payload
                .entries
                .iter()
                .find(|e| e.id == entry.id)
                .and_then(|e| e.last_used_at);
        }
        payload.tombstones.retain(|t| t.id != entry.id);
        // `position` rather than `iter_mut().find`: the index keeps no borrow
        // alive into the arms, so the push is not fighting the lookup.
        return Ok(
            match payload.entries.iter().position(|e| e.id == entry.id) {
                Some(index) => {
                    let changed = payload.entries[index] != entry;
                    payload.entries[index] = entry;
                    changed
                }
                None => {
                    payload.entries.push(entry);
                    true
                }
            },
        );
    }
    let group: Group = serde_json::from_value(envelope.record.clone()).map_err(|_| {
        format!(
            "sync: the record for group \"{}\" could not be read",
            envelope.id
        )
    })?;
    payload.tombstones.retain(|t| t.id != group.id);
    Ok(match payload.groups.iter().position(|g| g.id == group.id) {
        Some(index) => {
            let changed = payload.groups[index] != group;
            payload.groups[index] = group;
            changed
        }
        None => {
            payload.groups.push(group);
            true
        }
    })
}

/// Merge one incoming envelope into the live payload the way a pull lands a
/// record: against the stored slot (tombstone included, expired ones read as
/// absent), then store the result. `Ok(true)` when the stored form changed.
pub(crate) fn merge_into_payload(
    payload: &mut VaultPayload,
    incoming: &Envelope,
    now: u64,
) -> Result<bool, String> {
    let to_store = match local_envelope(payload, &incoming.kind, &incoming.id, now) {
        Some(local) => {
            merge(&local, incoming)
                .map_err(|e| format!("sync: the two copies could not be merged ({e:?})"))?
                .envelope
        }
        None => incoming.clone(),
    };
    write_envelope(payload, &to_store, now)
}

// ---------------------------------------------------------------------------
// The apply
// ---------------------------------------------------------------------------

/// Land a pull's dispositions into the payload.
///
/// Every `Merged` and `RemoteOnly` is re-merged against the LIVE payload rather
/// than trusted: an edit made while the pull's network request was in flight is
/// in the payload by the time this runs, and re-merging is what keeps it. The
/// merge's commutativity and idempotency are the guarantee.
pub fn apply_pull(payload: &mut VaultPayload, report: &PullReport, now: u64) -> Applied {
    let mut applied = Applied::default();
    for record in &report.records {
        let kind = record.kind.as_str();
        let id = record.id.as_str();
        let (envelope, republish) = match &record.outcome {
            Outcome::Merged {
                envelope,
                republish,
                ..
            } => (envelope, *republish),
            Outcome::RemoteOnly { envelope } => (envelope, false),
            Outcome::LocalOnly { stale } => {
                if *stale {
                    applied.stale.push(SlotRef {
                        kind: kind.to_string(),
                        id: id.to_string(),
                    });
                } else if local_envelope(payload, kind, id, now).is_some() {
                    payload.device.sync.mark_dirty(kind, id);
                }
                continue;
            }
        };
        match merge_into_payload(payload, envelope, now) {
            Ok(changed) => {
                if changed {
                    applied.landed += 1;
                    applied.changed_ids.push(id.to_string());
                }
            }
            Err(reason) => {
                applied.quarantine.push(quarantine_slot(kind, id, reason));
                applied.failed_slots.push(slot(kind, id));
                continue;
            }
        }
        if republish {
            payload.device.sync.mark_dirty(kind, id);
        }
    }
    // The map is the pull's whole answer. A slot whose landing was refused is
    // dropped from it, exactly as the pull itself drops a merge failure from
    // its `seen` map: the next pull must read it again.
    payload.device.sync.etags = report.etags.clone();
    for failed in &applied.failed_slots {
        payload.device.sync.etags.remove(failed);
    }
    applied
}

fn quarantine_slot(kind: &str, id: &str, reason: String) -> Quarantined {
    Quarantined {
        name: slot(kind, id),
        reason,
    }
}

/// Fold a push's answer into the payload.
///
/// Every successful etag is recorded; every slot whose envelope is unchanged
/// since the snapshot loses its dirty mark; every slot that failed, and every
/// slot edited during the network window, stays dirty so the edit is not lost
/// behind the etag skip. Returns how many slots are still dirty.
pub fn finish_push(payload: &mut VaultPayload, pushed: &[Envelope], report: &PushReport) -> usize {
    for (key, etag) in &report.etags {
        payload.device.sync.etags.insert(key.clone(), etag.clone());
    }
    let failed: BTreeSet<String> = report.failed.iter().map(|f| slot(&f.kind, &f.id)).collect();
    for envelope in pushed {
        let key = slot(&envelope.kind, &envelope.id);
        if failed.contains(&key) {
            continue;
        }
        match envelope_of_slot(payload, &envelope.kind, &envelope.id) {
            Some(current) => {
                if !content_differs(&current, envelope) {
                    payload.device.sync.dirty.remove(&key);
                }
            }
            None => {
                payload.device.sync.dirty.remove(&key);
            }
        }
    }
    payload.device.sync.dirty.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::sync::engine::test_support::*;
    use crate::modules::sync::engine::{push, PushFailure, Reconciled};
    use serde_json::json;

    #[test]
    fn apply_marks_a_republish_slot_dirty() {
        let mut payload = payload_of(vec![test_entry("e1", "mine", 2)], vec![]);
        let report = PullReport {
            records: vec![Reconciled {
                kind: ENTRY_KIND.into(),
                id: "e1".into(),
                outcome: Outcome::Merged {
                    envelope: entry_env("e1", 2, "dev-b", "mine"),
                    changed: false,
                    republish: true,
                },
            }],
            ..Default::default()
        };
        apply_pull(&mut payload, &report, NOW);
        assert!(payload.device.sync.dirty.contains("entry:e1"));
    }
    #[test]
    fn apply_keeps_an_edit_made_during_the_pull() {
        let mut payload = payload_of(vec![test_entry("e1", "old", 1)], vec![]);
        let report = PullReport {
            records: vec![Reconciled {
                kind: ENTRY_KIND.into(),
                id: "e1".into(),
                outcome: Outcome::Merged {
                    envelope: entry_env("e1", 2, "dev-b", "remote"),
                    changed: true,
                    republish: false,
                },
            }],
            ..Default::default()
        };
        // The edit lands AFTER the report was computed, while the request was
        // in flight.
        payload.entries[0].title = "edited during the pull".into();
        payload.entries[0].updated_at = 3;
        apply_pull(&mut payload, &report, NOW);
        assert_eq!(payload.entries[0].title, "edited during the pull");
        assert_eq!(payload.entries[0].updated_at, 3);
    }
    #[test]
    fn a_landed_tombstone_removes_the_record() {
        let mut payload = payload_of(vec![test_entry("e1", "gone", 1)], vec![]);
        let report = PullReport {
            records: vec![Reconciled {
                kind: ENTRY_KIND.into(),
                id: "e1".into(),
                outcome: Outcome::Merged {
                    envelope: grave(ENTRY_KIND, "e1", NOW - 1000, "dev-b"),
                    changed: true,
                    republish: false,
                },
            }],
            ..Default::default()
        };
        let applied = apply_pull(&mut payload, &report, NOW);
        assert!(payload.entries.is_empty());
        assert!(payload.tombstones.iter().any(|t| t.id == "e1"));
        assert_eq!(applied.landed, 1);
        assert_eq!(applied.changed_ids, vec!["e1".to_string()]);
    }
    #[test]
    fn etags_are_replaced_wholesale() {
        let mut payload = payload_of(vec![], vec![]);
        payload
            .device
            .sync
            .etags
            .insert("entry:old".into(), "old-etag".into());
        let report = PullReport {
            etags: BTreeMap::from([("entry:new".to_string(), "new-etag".to_string())]),
            ..Default::default()
        };
        apply_pull(&mut payload, &report, NOW);
        assert_eq!(
            payload.device.sync.etags,
            BTreeMap::from([("entry:new".to_string(), "new-etag".to_string())])
        );
    }
    #[test]
    fn apply_refuses_a_record_it_cannot_store_and_drops_its_etag() {
        // A record written by a build that knew a shape this one does not.
        // Storing it is impossible, so the slot is quarantined AND its etag is
        // dropped: keeping the etag would make every later pull skip the
        // object forever.
        let mut payload = payload_of(vec![], vec![]);
        let mut envelope = entry_env("h-1", 5, "dev-b", "fine");
        envelope.record = json!({ "not": "an entry" });
        let report = PullReport {
            records: vec![Reconciled {
                kind: ENTRY_KIND.into(),
                id: "h-1".into(),
                outcome: Outcome::RemoteOnly { envelope },
            }],
            etags: BTreeMap::from([("entry:h-1".to_string(), "etag".to_string())]),
            ..Default::default()
        };
        let applied = apply_pull(&mut payload, &report, NOW);
        assert_eq!(applied.quarantine.len(), 1);
        assert_eq!(applied.failed_slots, vec!["entry:h-1".to_string()]);
        assert!(payload.entries.is_empty());
        assert!(payload.device.sync.etags.is_empty());
    }
    #[test]
    fn a_rejected_push_stays_dirty() {
        // The push window's whole failure contract: a slot the remote refused
        // is still this device's problem, so it keeps its dirty mark and gains
        // no etag.
        let mut payload = payload_of(vec![test_entry("e1", "mine", 1)], vec![]);
        payload.device.sync.mark_dirty(ENTRY_KIND, "e1");
        let (pushed, _) = take_dirty_envelopes(&mut payload);
        let report = PushReport {
            etags: BTreeMap::new(),
            failed: vec![PushFailure {
                kind: ENTRY_KIND.into(),
                id: "e1".into(),
                reason: "the remote refused it".into(),
            }],
        };
        assert_eq!(finish_push(&mut payload, &pushed, &report), 1);
        assert!(payload.device.sync.dirty.contains("entry:e1"));
        assert!(payload.device.sync.etags.is_empty());
    }
    #[tokio::test]
    async fn an_edit_during_the_push_window_stays_dirty() {
        let keys = keys();
        let fake = Fake::cas(true);
        let mut payload = payload_of(vec![test_entry("e1", "first", 1)], vec![]);
        payload.device.sync.mark_dirty(ENTRY_KIND, "e1");
        let (envelopes, etags) = take_dirty_envelopes(&mut payload);
        assert_eq!(envelopes.len(), 1);

        // The user edits after the snapshot was taken but before the answer.
        payload.entries[0].title = "second".into();
        payload.entries[0].updated_at = 2;
        let report = push(
            &fake,
            &keys,
            PREFIX,
            "this-device",
            envelopes.clone(),
            etags,
        )
        .await;
        assert!(report.failed.is_empty());
        finish_push(&mut payload, &envelopes, &report);
        assert!(
            payload.device.sync.dirty.contains("entry:e1"),
            "the newer edit lost its dirty mark"
        );
    }
    #[test]
    fn take_dirty_envelopes_drops_slots_with_nothing_to_push() {
        let mut payload = payload_of(vec![test_entry("e1", "kept", 1)], vec![]);
        payload.device.sync.mark_dirty(ENTRY_KIND, "e1");
        payload.device.sync.mark_dirty(ENTRY_KIND, "gone");
        payload
            .device
            .sync
            .etags
            .insert("entry:e1".into(), "e1-etag".into());
        let (envelopes, etags) = take_dirty_envelopes(&mut payload);
        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].id, "e1");
        assert_eq!(etags.get("entry:e1"), Some(&"e1-etag".to_string()));
        assert!(!payload.device.sync.dirty.contains("entry:gone"));
        assert!(payload.device.sync.dirty.contains("entry:e1"));
    }
}
