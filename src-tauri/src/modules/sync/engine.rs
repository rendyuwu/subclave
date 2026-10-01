//! The two round trips, the apply path, and the commands.
//!
//! WHAT THIS FILE OWNS. The object layout, the pull and the push, the landing
//! of a pull into the vault payload, the push bookkeeping, the in-process
//! session, and the five commands the webview calls.
//!
//! WHAT IT DELIBERATELY DOES NOT OWN. No trigger and no scheduler: the webview
//! decides when a pull or a push happens (see the sibling TypeScript module
//! under `src/modules/`). No secret store of its own: the passphrase and the
//! storage credentials arrive as arguments to [`sync_configure`] and
//! [`sync_join`], are used for the one call, and are gone when it returns; what
//! survives is only the wrapped root key inside the vault. No store write
//! outside the vault: [`apply_pull`] mutates a
//! `crate::modules::vault::model::VaultPayload`, and the caller commits it.
//!
//! NO LOCK IS HELD ACROSS THE NETWORK. The pull snapshots the payload, releases
//! the guard, and only then talks to the remote. The landing re-merges against
//! the LIVE payload, so an edit made while the request was in flight is not
//! overwritten by the copy the report was computed from; the merge's
//! commutativity and idempotency, pinned by the tests in
//! `src-tauri/src/modules/vault/merge_history.rs`, are what make that safe.
//!
//! NOTHING HERE RUNS A BLOCKING CALL ON THE ASYNC THREADS. The KDF and every
//! file write go through `tokio::task::spawn_blocking` (the testable helpers)
//! or `tauri::async_runtime::spawn_blocking` (the command shells). The
//! `no_new_sync_tauri_commands` test in `src-tauri/src/lib.rs` enforces the
//! async half of that.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

use super::crypto::{
    expand_root, new_keyfile_with_root, object_name, open_keyfile_root, open_record, seal_record,
    SealedRecord, SyncKeyfile, SyncKeys, NOT_A_KEYFILE,
};
use super::model::{
    content_differs, known_kind, merge, strip_device_local, Envelope, ENTRY_KIND, GROUP_KIND,
    WIRE_VERSION,
};
use super::provider::{build, ProviderError, SyncProvider};
use crate::modules::strength::strength_of;
use crate::modules::vault::file::VAULT_FILE_NAME;
use crate::modules::vault::model::{
    seed_reserved_groups, Entry, Group, SyncDevice, Tombstone, TombstoneKind, VaultPayload,
};
use crate::modules::vault::{commit, emit_changed, install_new_vault, VaultState, LOCKED_ERR};

/// How long a tombstone stays meaningful, in milliseconds.
///
/// It governs two different windows: when a local tombstone stops being read,
/// and when a REMOTE tombstone object is removed. Two different windows would
/// leave objects on the remote that no device still reads, or remove objects
/// devices are still comparing against.
const TOMBSTONE_TTL_MS: u64 = 90 * 24 * 60 * 60 * 1000;

/// What a command answers when no configuration has been opened.
///
/// Reachable on every launch, not only on a device that never configured:
/// [`SyncState`] starts empty and [`sync_configure`] is what fills it, so a
/// stored configuration is worth nothing until the caller has opened it again.
pub const NOT_CONFIGURED: &str = "sync: no sync configuration is open on this device";

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
fn object_prefix(prefix: &str) -> String {
    format!("{}/obj/", root(prefix))
}

fn object_key(prefix: &str, name: &str) -> String {
    format!("{}{name}", object_prefix(prefix))
}

/// Where the keyfile sits: beside the object namespace, not inside it.
///
/// OUTSIDE `obj/` deliberately. The pull lists that prefix and hands every key
/// it finds to `open_envelope`, and a keyfile is not a sealed record - it would
/// quarantine on every pull, forever, and the quarantine list is a user-facing
/// surface.
fn keyfile_key(prefix: &str) -> String {
    format!("{}/keyfile", root(prefix))
}

/// The `<name>` half of a listed key, which is what the etag map is matched on.
fn name_of(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// Where one record sits in the etag map: `kind:id`.
///
/// NOT the object name, and that is the point. The caller stores this map and
/// hands it back on the next pull, and an object name is an HMAC under a key
/// the caller never sees. `kind:id` it can compute, so the exclusion the apply
/// path owes - drop every refused record from the map - is a plain lookup.
fn slot(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

// ---------------------------------------------------------------------------
// What a pull answers
// ---------------------------------------------------------------------------

/// What this pull decided about one record.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum Outcome {
    /// Both sides had a copy.
    #[serde(rename_all = "camelCase")]
    Merged {
        envelope: Envelope,
        /// Whether the landed record differs from what was already stored.
        changed: bool,
        /// Whether the remote object still has to be brought up to the winner.
        republish: bool,
    },
    /// The remote had a copy and this device has neither a record nor a living
    /// tombstone for it.
    #[serde(rename_all = "camelCase")]
    RemoteOnly { envelope: Envelope },
    /// This device has a copy and the remote has no object for it at all.
    #[serde(rename_all = "camelCase")]
    LocalOnly {
        /// Older than the tombstone window, so the absence is most likely a
        /// delete whose tombstone has already expired everywhere.
        ///
        /// REPORTED, NEVER DELETED. Deleting a local record because an object
        /// is missing from a listing is data loss driven by an inference: a
        /// truncated page, an eventually-consistent endpoint and a provider-side
        /// accident all present as absence. The user resolves it with a manual
        /// push or a local delete.
        ///
        /// NEVER SET WHEN THE LISTING WAS EMPTY. A prefix holding zero objects
        /// has never held this inventory at all, so the absence is not a delete
        /// and everything publishes.
        stale: bool,
    },
}

/// One record's disposition, named the way the caller names records.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reconciled {
    pub kind: String,
    pub id: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

/// A remote object this device could not read or refused to store.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Quarantined {
    /// The remote object's own name for a pull-side refusal (opaque hex), or
    /// the `kind:id` slot for an apply-side one, where the object name is not
    /// available without the sync keys.
    pub name: String,
    pub reason: String,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PullReport {
    pub records: Vec<Reconciled>,
    /// `kind:id` to etag, for every remote object this pull read or skipped.
    ///
    /// An object that quarantined is NOT in here, so the next pull reads it
    /// again rather than recording a landing that never happened.
    pub etags: BTreeMap<String, String>,
    pub quarantined: Vec<Quarantined>,
    /// Remote tombstone objects this device published and has now removed.
    pub pruned: usize,
    /// How many records the remote does not yet hold this device's copy of.
    ///
    /// The pull's own count, and it deliberately excludes an etag-skipped
    /// object: the remote's copy of that one has not moved since the last pull,
    /// so this pass has nothing to say about it.
    pub pending: usize,
}

/// One record a push could not place.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PushFailure {
    pub kind: String,
    pub id: String,
    pub reason: String,
}

#[derive(Serialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PushReport {
    /// `kind:id` to the etag the remote gave the object this push wrote.
    pub etags: BTreeMap<String, String>,
    /// PER OBJECT, and nothing throws: one record the remote refused must not
    /// take the rest of the inventory with it.
    pub failed: Vec<PushFailure>,
}

// ---------------------------------------------------------------------------
// Sealing
// ---------------------------------------------------------------------------

fn open_envelope(keys: &SyncKeys, bytes: &[u8]) -> Result<Envelope, String> {
    let sealed: SealedRecord = serde_json::from_slice(bytes)
        .map_err(|_| "sync: the object is not a sealed record".to_string())?;
    let plain = open_record(keys, &sealed)?;
    serde_json::from_slice(&plain[..])
        .map_err(|_| "sync: the sealed record is not an envelope".to_string())
}

/// Seal one envelope, stripping the device-local fields on the way out.
///
/// THE STRIP IS ALSO DONE BY THE CALLER, and the duplication is deliberate
/// rather than sloppy. The caller has to strip, because the envelopes it builds
/// from the payload have to compare equal to a stripped remote copy. This side
/// strips as well so that no future caller can publish a device-local value by
/// forgetting to.
fn seal_envelope(keys: &SyncKeys, envelope: &Envelope) -> Result<Vec<u8>, String> {
    let mut envelope = envelope.clone();
    strip_device_local(&mut envelope.record);
    let plain = serde_json::to_string(&envelope)
        .map_err(|_| "sync: the envelope could not be serialized".to_string())?;
    let sealed = seal_record(keys, plain.as_bytes())?;
    serde_json::to_vec(&sealed).map_err(|_| "sync: the sealed record could not be written".into())
}

// ---------------------------------------------------------------------------
// The pull
// ---------------------------------------------------------------------------

/// Reconcile the remote inventory against `locals`, pruning as it goes.
///
/// `locals` is records PLUS living tombstones, which is what lets a local
/// delete pair with its remote counterpart and resolve through the merge like
/// any other disagreement. Without the tombstones a deleted record would read
/// as `RemoteOnly` and land again on the device that deleted it.
///
/// `etags` is what the previous pull returned, minus anything the apply
/// refused.
#[allow(clippy::too_many_arguments)]
pub async fn pull(
    provider: &dyn SyncProvider,
    keys: &SyncKeys,
    prefix: &str,
    device: &str,
    locals: Vec<Envelope>,
    etags: BTreeMap<String, String>,
    now: u64,
) -> Result<PullReport, ProviderError> {
    let entries = provider.list(&object_prefix(prefix)).await?;

    // Lifts the stale rule at the bottom of this function - see
    // `Outcome::LocalOnly`.
    let remote_is_empty = entries.is_empty();

    // The map arrives keyed by `kind:id`; the listing speaks object names. One
    // HMAC per known record turns one into the other.
    let mut named: BTreeMap<String, String> = BTreeMap::new();
    for key in etags.keys() {
        if let Some((kind, id)) = key.split_once(':') {
            named.insert(object_name(keys, kind, id), key.clone());
        }
    }

    let mut report = PullReport::default();
    let mut remotes: BTreeMap<String, Envelope> = BTreeMap::new();
    let mut skipped: BTreeSet<String> = BTreeSet::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();

    for entry in entries {
        let name = name_of(&entry.key).to_string();

        // THE PRUNE, step one: candidates come off the LISTING, outside the
        // etag skip below. `Entry::modified_at` is the remote's own stamp and
        // is the only reading of "old" available before a GET - and it has to
        // be, because a tombstone object never changes, so its etag matches
        // from the second pull onward and a filter reading `updatedAt` from
        // inside the envelope would be unreachable after the first run.
        let candidate = entry
            .modified_at
            .is_some_and(|m| now.saturating_sub(m) >= TOMBSTONE_TTL_MS);

        if !candidate {
            if let Some(known) = named.get(&name) {
                if etags.get(known) == Some(&entry.etag) {
                    skipped.insert(known.clone());
                    seen.insert(known.clone(), entry.etag);
                    continue;
                }
            }
        }

        // Step two: a candidate is fetched whether or not its etag moved,
        // because reading `Envelope::device` is the only way to answer step
        // three. By definition these are a handful.
        let Some(object) = provider.get(&entry.key).await? else {
            // Raced a delete by another device. Nothing to reconcile.
            continue;
        };
        let envelope = match open_envelope(keys, &object.bytes) {
            Ok(envelope) => envelope,
            Err(reason) => {
                report.quarantined.push(Quarantined { name, reason });
                continue;
            }
        };

        // THE VERSION CHECK, HERE RATHER THAN IN THE MERGE, AND BEFORE THE
        // PRUNE. `merge` refuses a version it does not know, but a `RemoteOnly`
        // object never reaches the merge - there is no local copy to merge it
        // with - so an object from a newer build would otherwise be handed to
        // the apply path unexamined, and its record shape written straight into
        // this device's payload. That is exactly what `WIRE_VERSION` prevents.
        //
        // BEFORE the prune rather than after, because the prune's decision is
        // an irreversible DELETE authorized by `deleted` and `device` - two
        // fields read through a schema this build has just said it cannot
        // interpret.
        if envelope.v != WIRE_VERSION {
            report.quarantined.push(Quarantined {
                name,
                reason: format!(
                    "sync: this object was written by a newer build (wire version {}, this build reads {WIRE_VERSION})",
                    envelope.v
                ),
            });
            continue;
        }

        // An object whose kind this build does not know is quarantined for the
        // same reason: the apply path has no shape to store it in, and the
        // prune must not delete it on the strength of fields it cannot read.
        if !known_kind(&envelope.kind) {
            report.quarantined.push(Quarantined {
                name,
                reason: format!(
                    "sync: this build does not know the record kind \"{}\"",
                    envelope.kind
                ),
            });
            continue;
        }

        // Step three: delete only what THIS device published.
        //
        // A prune is destructive to every device, unlike a local expiry. A
        // device whose clock runs 100 days fast would otherwise delete every
        // remote tombstone on its first pull, and every other device would then
        // re-push any record edited inside the window. Locally a bad clock
        // skews one device's view and nobody else's.
        //
        // This is the first legitimate read of `Envelope::device`: the merge
        // must not read it, but a prune is exactly a question about provenance.
        //
        // The candidacy is re-asked against the envelope's OWN stamp rather
        // than taken from the listing, because that stamp is the `deletedAt`
        // every device filters by and the listing's is only when the object was
        // last written.
        let expired = envelope
            .updated_at
            .is_some_and(|u| now.saturating_sub(u) >= TOMBSTONE_TTL_MS);
        if candidate && envelope.deleted && expired {
            // A REFUSED DELETE IS NOT A FAILED PULL. A bucket with read-only
            // credentials, an object lock or a lifecycle policy refuses every
            // one of these, and propagating that would make the FIRST expired
            // tombstone abort the whole reconcile - no landings, no pushes,
            // forever, over an object whose only cost is the bytes it occupies.
            if envelope.device == device && provider.delete(&entry.key).await.is_ok() {
                report.pruned += 1;
            }
            // SKIPPED EITHER WAY, and that is not tidiness. An expired
            // tombstone is older than the window every device filters reads by,
            // so landing it writes a row that every subsequent read discards.
            continue;
        }

        let key = slot(&envelope.kind, &envelope.id);
        seen.insert(key.clone(), object.etag);
        remotes.insert(key, envelope);
    }

    // LAST WINS on a duplicate slot, and the caller sends tombstones after
    // records, so a payload that somehow held both a live record and a living
    // tombstone for one id would present the TOMBSTONE here. That is the
    // conservative side and the same one the ordering key takes on an exact
    // tie: a lost delete re-spreads data the user removed, a lost resurrection
    // costs one re-create.
    let mut mine: BTreeMap<String, Envelope> = locals
        .into_iter()
        .map(|e| (slot(&e.kind, &e.id), e))
        .collect();

    for (key, remote) in remotes {
        let (kind, id) = (remote.kind.clone(), remote.id.clone());
        let Some(local) = mine.remove(&key) else {
            report.records.push(Reconciled {
                kind,
                id,
                outcome: Outcome::RemoteOnly { envelope: remote },
            });
            continue;
        };
        match merge(&local, &remote) {
            Ok(merged) => {
                // `merge` re-serializes the merged entry, so `lastUsedAt` is
                // back on the record and `device` was never on it. Every
                // envelope this layer reports is stripped, and `republish`
                // compares against the remote copy, which was stripped by
                // `seal_envelope` before it was ever published.
                let mut envelope = merged.envelope;
                strip_device_local(&mut envelope.record);
                let republish = content_differs(&envelope, &remote);
                if republish {
                    report.pending += 1;
                }
                report.records.push(Reconciled {
                    kind,
                    id,
                    outcome: Outcome::Merged {
                        envelope,
                        changed: merged.changed,
                        republish,
                    },
                });
            }
            Err(reason) => {
                // Out of the etag map as well as out of the record list, or the
                // next pull records a landing that never happened.
                seen.remove(&key);
                report.quarantined.push(Quarantined {
                    name: object_name(keys, &kind, &id),
                    reason: format!("sync: the two copies could not be merged ({reason:?})"),
                });
            }
        }
    }

    for (key, local) in mine {
        if skipped.contains(&key) {
            continue;
        }
        // An UNSTAMPED local record is not stale. Absent means "written before
        // the field existed", which is the one record that has certainly never
        // been published - so it is pushed, not reported.
        //
        // NOTHING IS STALE ON AN EMPTY REMOTE. The rule reads absence as a
        // delete whose tombstone has expired, and that reading needs a remote
        // that once held the record.
        let stale = !remote_is_empty
            && local
                .updated_at
                .is_some_and(|u| now.saturating_sub(u) >= TOMBSTONE_TTL_MS);
        if !stale {
            report.pending += 1;
        }
        report.records.push(Reconciled {
            kind: local.kind,
            id: local.id,
            outcome: Outcome::LocalOnly { stale },
        });
    }

    report.etags = seen;
    Ok(report)
}

// ---------------------------------------------------------------------------
// The push
// ---------------------------------------------------------------------------

/// Publish `envelopes`, conditionally where the provider honours it.
///
/// `etags` supplies the `If-Match`. A record with no entry is a create, which
/// goes out with no condition at all: there is no etag to match. On a
/// compare-and-swap provider that is a create-if-absent race with another
/// device publishing the same id - the loser's write is overwritten and
/// recovered on the next pull, because the merge is content-ordered.
///
/// THE DEVICE ID IS STAMPED HERE, over whatever the caller sent. It is the one
/// field on an envelope that is not a fact about the record, and the prune
/// deletes remote objects on the strength of it - so the frontend is not
/// allowed a say in what it says.
pub async fn push(
    provider: &dyn SyncProvider,
    keys: &SyncKeys,
    prefix: &str,
    device: &str,
    envelopes: Vec<Envelope>,
    etags: BTreeMap<String, String>,
) -> PushReport {
    let cas = provider.capabilities().cas;
    let mut report = PushReport::default();
    for mut envelope in envelopes {
        envelope.device = device.to_string();
        let key = slot(&envelope.kind, &envelope.id);
        let object = object_key(prefix, &object_name(keys, &envelope.kind, &envelope.id));
        let condition = if cas { etags.get(&key).cloned() } else { None };
        match put_one(provider, keys, &object, &envelope, condition.as_deref()).await {
            Ok(etag) => {
                report.etags.insert(key, etag);
            }
            Err(reason) => report.failed.push(PushFailure {
                kind: envelope.kind.clone(),
                id: envelope.id.clone(),
                reason,
            }),
        }
    }
    report
}

async fn put_one(
    provider: &dyn SyncProvider,
    keys: &SyncKeys,
    object: &str,
    envelope: &Envelope,
    condition: Option<&str>,
) -> Result<String, String> {
    let bytes = seal_envelope(keys, envelope)?;
    match provider.put(object, bytes, condition).await {
        Ok(etag) => Ok(etag),
        Err(ProviderError::PreconditionFailed) => retry_merged(provider, keys, object, envelope)
            .await
            .map_err(|e| e.to_string()),
        Err(other) => Err(other.to_string()),
    }
}

/// ONE retry, and it goes back through the merge rather than forcing the write.
///
/// `PreconditionFailed` says the remote copy moved after the etag in hand was
/// read. A forced overwrite would discard whatever the other device just
/// published; the winner of the two copies is the only thing safe to write, and
/// computing it is what the merge is for. The failure is not surfaced to the
/// user: this is the ordinary shape of two devices editing at once.
async fn retry_merged(
    provider: &dyn SyncProvider,
    keys: &SyncKeys,
    object: &str,
    envelope: &Envelope,
) -> Result<String, ProviderError> {
    let Some(current) = provider.get(object).await? else {
        // Gone rather than moved - the prune removed it between the two calls.
        // This device's copy is then the only one left.
        let bytes = seal_envelope(keys, envelope).map_err(ProviderError::Config)?;
        return provider.put(object, bytes, None).await;
    };
    let remote = open_envelope(keys, &current.bytes).map_err(ProviderError::Malformed)?;
    let mut winner = merge(envelope, &remote)
        .map_err(|e| ProviderError::Malformed(format!("sync: the two copies disagree ({e:?})")))?
        .envelope;
    // Strip before the comparison: `merge` re-serializes the merged entry, and
    // the remote copy came off the wire already stripped.
    strip_device_local(&mut winner.record);
    // The other device already published what this merge resolves to. Writing
    // it again would only mint a fresh nonce for identical content.
    if !content_differs(&winner, &remote) {
        return Ok(current.etag);
    }
    let bytes = seal_envelope(keys, &winner).map_err(ProviderError::Config)?;
    provider.put(object, bytes, Some(&current.etag)).await
}

// ---------------------------------------------------------------------------
// Payload to envelopes
// ---------------------------------------------------------------------------

/// The record as it sits in the payload, device-local fields included.
///
/// NOT STRIPPED, and that is what the merge expects: `model::merge` builds its
/// result with `serde_json::to_value(merged)`, which puts `lastUsedAt` back on
/// an entry anyway, and `Merged::changed` compares the result against the local
/// envelope it was handed. Stripping here would make every agreed pair look
/// changed. The strip that matters happens on PUBLISH (`seal_envelope`) and on
/// a merged envelope before it is compared against a remote copy.
fn entry_envelope(entry: &Entry) -> Envelope {
    Envelope {
        v: WIRE_VERSION,
        kind: ENTRY_KIND.to_string(),
        id: entry.id.clone(),
        updated_at: Some(entry.updated_at),
        device: String::new(),
        deleted: false,
        record: serde_json::to_value(entry).expect("entry serialization"),
    }
}

fn group_envelope(group: &Group) -> Envelope {
    Envelope {
        v: WIRE_VERSION,
        kind: GROUP_KIND.to_string(),
        id: group.id.clone(),
        updated_at: Some(group.updated_at),
        device: String::new(),
        deleted: false,
        record: serde_json::to_value(group).expect("group serialization"),
    }
}

fn tombstone_envelope(tombstone: &Tombstone) -> Envelope {
    Envelope {
        v: WIRE_VERSION,
        kind: match tombstone.kind {
            TombstoneKind::Entry => ENTRY_KIND.to_string(),
            TombstoneKind::Group => GROUP_KIND.to_string(),
        },
        id: tombstone.id.clone(),
        updated_at: Some(tombstone.deleted_at),
        device: String::new(),
        deleted: true,
        record: Value::Null,
    }
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
pub fn local_envelope(payload: &VaultPayload, kind: &str, id: &str, now: u64) -> Option<Envelope> {
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

fn remove_entry(payload: &mut VaultPayload, id: &str) -> bool {
    let before = payload.entries.len();
    payload.entries.retain(|e| e.id != id);
    payload.entries.len() != before
}

fn remove_group(payload: &mut VaultPayload, id: &str) -> bool {
    let before = payload.groups.len();
    payload.groups.retain(|g| g.id != id);
    payload.groups.len() != before
}

/// Store one reconciled envelope into the payload.
///
/// `Err(reason)` = the envelope names a record this build refuses to store (a
/// live entry whose record does not deserialize, or an unknown kind); the
/// caller quarantines it. `Ok(false)` = a tombstone for a record that was not
/// there; `Ok(true)` = the stored form changed.
pub fn write_envelope(
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
            TombstoneKind::Entry => remove_entry(payload, &envelope.id),
            TombstoneKind::Group => remove_group(payload, &envelope.id),
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

// ---------------------------------------------------------------------------
// The apply
// ---------------------------------------------------------------------------

/// What a pull's landing left behind.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Applied {
    pub landed: usize,
    pub changed_ids: Vec<String>,
    pub stale: Vec<SlotRef>,
    pub quarantine: Vec<Quarantined>,
    /// `"kind:id"` of every slot whose landing was refused; their etags are
    /// dropped from the etag map so the next pull reads them again.
    pub failed_slots: Vec<String>,
}

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
        let to_store = match local_envelope(payload, kind, id, now) {
            Some(local) => match merge(&local, envelope) {
                Ok(merged) => merged.envelope,
                Err(reason) => {
                    let reason = format!("sync: the two copies could not be merged ({reason:?})");
                    applied.quarantine.push(quarantine_slot(kind, id, reason));
                    applied.failed_slots.push(slot(kind, id));
                    continue;
                }
            },
            None => envelope.clone(),
        };
        match write_envelope(payload, &to_store, now) {
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

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// One opened configuration.
///
/// CLONED OUT OF THE LOCK before anything is awaited. A `std::sync::MutexGuard`
/// is not `Send`, so an async command holding one across an await does not
/// compile - and a lock held across a network round trip would serialize every
/// other caller behind the slowest request anyway. Every field here is either
/// an `Arc` or a short string, so the clone is cheap.
#[derive(Clone)]
pub struct SyncSession {
    pub keys: Arc<SyncKeys>,
    pub provider: Arc<dyn SyncProvider>,
    pub prefix: String,
    pub device: String,
}

/// The configuration the commands below run against, or none.
///
/// EMPTY ON EVERY LAUNCH, and nothing persists it: [`sync_configure`] fills it
/// and the process losing it is the whole of "sync is off". Until a caller
/// configures, every other command answers [`NOT_CONFIGURED`].
#[derive(Default)]
pub struct SyncState {
    session: Mutex<Option<SyncSession>>,
}

impl SyncState {
    pub(crate) fn open(&self) -> Result<SyncSession, String> {
        self.session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| NOT_CONFIGURED.to_string())
    }

    /// ONE SPELLING OF THE WRITE for both the open and the close.
    pub(crate) fn set(&self, session: Option<SyncSession>) -> Result<(), String> {
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = session;
        Ok(())
    }

    pub(crate) fn clear(&self) {
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Arguments and small helpers
// ---------------------------------------------------------------------------

/// What [`sync_configure`] takes, mirrored by `SyncConfigureArgs` in
/// `src/modules/sync/types.ts`.
///
/// `deny_unknown_fields` turns a field renamed on one side into a loud refusal
/// at the first call instead of a silently defaulted field.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncConfigArg {
    pub provider: String,
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub cas: bool,
}

/// The provider credentials. Every field is optional because the caller sends
/// only what the user typed; the stored ones fill the rest.
#[derive(Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncCredentialsArg {
    #[serde(default)]
    pub access_key_id: Option<String>,
    #[serde(default)]
    pub secret_access_key: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncConfigureArgs {
    pub config: SyncConfigArg,
    #[serde(default)]
    pub credentials: SyncCredentialsArg,
    #[serde(default)]
    pub passphrase: Option<String>,
    #[serde(default)]
    pub create: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncJoinArgs {
    pub master_password: String,
    pub config: SyncConfigArg,
    pub credentials: SyncCredentialsArg,
    pub passphrase: String,
}

/// One slot, named the way the payload names it.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SlotRef {
    pub kind: String,
    pub id: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncConfigureResult {
    /// `"existing"`, `"fresh"` or `"created"`.
    pub remote: String,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncPullResult {
    pub pending: usize,
    pub landed: usize,
    pub quarantine: Vec<Quarantined>,
    pub stale: Vec<SlotRef>,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncPushResult {
    pub pushed: usize,
    pub failed: usize,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncJoinResult {
    /// `"existing"` or `"fresh"`.
    pub remote: String,
    pub landed: usize,
    pub quarantine: Vec<Quarantined>,
}

/// The identity a stored root key and etag map are valid for:
/// `"{provider}|{endpoint}|{bucket}|{prefix}"`.
fn remote_identity(cfg: &SyncConfigArg) -> String {
    format!(
        "{}|{}|{}|{}",
        cfg.provider, cfg.endpoint, cfg.bucket, cfg.prefix
    )
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value.filter(|v| !v.is_empty()).map(str::to_string)
}

/// The provider's own configuration shape, built from the argument and the
/// credentials actually in use.
fn provider_config(cfg: &SyncConfigArg, creds: &SyncCredentialsArg) -> Result<Value, String> {
    match cfg.provider.as_str() {
        "s3" => {
            let access_key_id = non_empty(creds.access_key_id.as_deref())
                .ok_or_else(|| "sync: the storage credentials are required".to_string())?;
            let secret_access_key = non_empty(creds.secret_access_key.as_deref())
                .ok_or_else(|| "sync: the storage credentials are required".to_string())?;
            Ok(serde_json::json!({
                "endpoint": cfg.endpoint,
                "region": cfg.region,
                "bucket": cfg.bucket,
                "cas": cfg.cas,
                "accessKeyId": access_key_id,
                "secretAccessKey": secret_access_key,
            }))
        }
        "webdav" => Ok(serde_json::json!({
            "endpoint": cfg.endpoint,
            "username": creds.username.clone().unwrap_or_default(),
            "password": creds.password.clone().unwrap_or_default(),
        })),
        // The unknown id is refused by `build` with its own message; this only
        // has to hand it something shaped like a configuration.
        _ => Ok(Value::Object(serde_json::Map::new())),
    }
}

/// Point `device` at `identity`, dropping the root key and etags that only the
/// old remote was valid for. `dirty` is kept, so local edits still follow the
/// vault to its new storage. Returns whether anything changed.
pub fn reset_for_identity(device: &mut SyncDevice, identity: &str) -> bool {
    if device.remote.as_deref() == Some(identity) {
        return false;
    }
    device.remote = Some(identity.to_string());
    device.root_key = None;
    device.etags.clear();
    true
}

/// Which remote state [`configure_keyfile`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteState {
    Existing,
    Fresh,
    Created,
}

/// The keyfile decision, and the root key a session has to be persisted under.
pub struct Configured {
    pub remote: RemoteState,
    pub root: Option<Zeroizing<[u8; 32]>>,
}

// ---------------------------------------------------------------------------
// Keyfiles
// ---------------------------------------------------------------------------

fn decode_root(stored: &str) -> Result<Zeroizing<[u8; 32]>, String> {
    let bytes = B64
        .decode(stored)
        .map_err(|_| "sync: the stored root key is corrupt".to_string())?;
    let root: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| "sync: the stored root key is corrupt".to_string())?;
    Ok(Zeroizing::new(root))
}

/// The keyfile decision for one configure.
///
/// Provider-injected so every branch is testable against a fake without a Tauri
/// app. `stored_root` is the base64 root key already in the vault, if any.
pub async fn configure_keyfile(
    provider: &dyn SyncProvider,
    prefix: &str,
    passphrase: Option<&str>,
    create: bool,
    stored_root: Option<&str>,
) -> Result<Configured, String> {
    let key = keyfile_key(prefix);
    let Some(object) = provider.get(&key).await.map_err(|e| e.to_string())? else {
        if !create {
            // A fresh prefix with no instruction to create one changes nothing:
            // no keyfile, no session, no write.
            return Ok(Configured {
                remote: RemoteState::Fresh,
                root: None,
            });
        }
        let pass = non_empty(passphrase)
            .ok_or_else(|| "sync: the sync passphrase is required".to_string())?;
        let pass_for_strength = pass.clone();
        let strength = tokio::task::spawn_blocking(move || strength_of(&pass_for_strength))
            .await
            .map_err(|e| format!("sync: task failed: {e}"))?;
        if strength.score < 3 {
            return Err(match strength.warning {
                Some(warning) => format!("sync: the sync passphrase is too weak: {warning}"),
                None => "sync: the sync passphrase is too weak".to_string(),
            });
        }
        let pass_for_mint = pass.clone();
        let (keyfile, root) =
            tokio::task::spawn_blocking(move || new_keyfile_with_root(&pass_for_mint))
                .await
                .map_err(|e| format!("sync: task failed: {e}"))??;
        let bytes = serde_json::to_vec(&keyfile)
            .map_err(|_| "sync: the keyfile could not be written".to_string())?;
        return match provider
            .put_if_absent(&key, bytes)
            .await
            .map_err(|e| e.to_string())?
        {
            // The write landed, so this device's root is the one the remote
            // holds and it is already in hand: re-opening the keyfile it just
            // wrote would spend a second KDF run for nothing.
            Some(_) => Ok(Configured {
                remote: RemoteState::Created,
                root: Some(root),
            }),
            None => {
                // Another device minted the keyfile first. Join its root key
                // instead of overwriting it.
                let Some(object) = provider.get(&key).await.map_err(|e| e.to_string())? else {
                    return Err("sync: the keyfile disappeared while it was being created".into());
                };
                let keyfile: SyncKeyfile =
                    serde_json::from_slice(&object.bytes).map_err(|_| NOT_A_KEYFILE.to_string())?;
                let root = tokio::task::spawn_blocking(move || open_keyfile_root(&keyfile, &pass))
                    .await
                    .map_err(|e| format!("sync: task failed: {e}"))??;
                Ok(Configured {
                    remote: RemoteState::Existing,
                    root: Some(root),
                })
            }
        };
    };

    // A keyfile is present. Bytes that do not parse one tell the user they are
    // pointed at the wrong place, not that their passphrase is wrong.
    let keyfile: SyncKeyfile =
        serde_json::from_slice(&object.bytes).map_err(|_| NOT_A_KEYFILE.to_string())?;
    if let Some(stored) = stored_root.filter(|s| !s.is_empty()) {
        return Ok(Configured {
            remote: RemoteState::Existing,
            root: Some(decode_root(stored)?),
        });
    }
    let pass =
        non_empty(passphrase).ok_or_else(|| "sync: the sync passphrase is required".to_string())?;
    let root = tokio::task::spawn_blocking(move || open_keyfile_root(&keyfile, &pass))
        .await
        .map_err(|e| format!("sync: task failed: {e}"))??;
    Ok(Configured {
        remote: RemoteState::Existing,
        root: Some(root),
    })
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Open or re-open a sync session: build the provider, settle the keyfile, and
/// hold both.
///
/// A `"fresh"` answer writes NOTHING, not even the credentials: creating the
/// keyfile is a separate, user-confirmed call (`create: true`), and until then
/// there is no remote to be configured for.
#[tauri::command]
pub async fn sync_configure(
    app: AppHandle,
    args: SyncConfigureArgs,
) -> Result<SyncConfigureResult, String> {
    let identity = remote_identity(&args.config);
    // The stored device state, with the guard dropped before anything awaits.
    let mut stored = {
        let vault_state = app.state::<VaultState>();
        let guard = vault_state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        unlocked.payload.device.sync.clone()
    };
    // A config that names another remote drops the root key and etags it cannot
    // use before anything reads them, including the `stored_root` lookup below.
    reset_for_identity(&mut stored, &identity);

    let credentials = SyncCredentialsArg {
        access_key_id: non_empty(args.credentials.access_key_id.as_deref())
            .or_else(|| stored.s3_access_key_id.clone()),
        secret_access_key: non_empty(args.credentials.secret_access_key.as_deref())
            .or_else(|| stored.s3_secret_access_key.clone()),
        username: non_empty(args.credentials.username.as_deref())
            .or_else(|| stored.webdav_username.clone()),
        password: non_empty(args.credentials.password.as_deref())
            .or_else(|| stored.webdav_password.clone()),
    };

    let provider = build(
        &args.config.provider,
        provider_config(&args.config, &credentials)?,
    )
    .map_err(|e| e.to_string())?;

    let configured = configure_keyfile(
        provider.as_ref(),
        &args.config.prefix,
        args.passphrase.as_deref(),
        args.create,
        stored.root_key.as_deref(),
    )
    .await?;

    if configured.remote == RemoteState::Fresh {
        return Ok(SyncConfigureResult {
            remote: "fresh".to_string(),
        });
    }
    let root = configured
        .root
        .ok_or_else(|| "sync: the keyfile could not be opened".to_string())?;
    let keys = Arc::new(expand_root(&root)?);

    let task_app = app.clone();
    let prefix = args.config.prefix.clone();
    let provider_for_task = Arc::clone(&provider);
    let keys_for_task = Arc::clone(&keys);
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let vault_state = task_app.state::<VaultState>();
        let sync_state = task_app.state::<SyncState>();
        let dir = crate::modules::vault::vault_dir(&task_app)?;
        let device = super::device_id(&task_app)?;
        {
            let mut guard = vault_state.access()?;
            let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
            let sync = &mut unlocked.payload.device.sync;
            reset_for_identity(sync, &identity);
            sync.root_key = Some(B64.encode(*root));
            sync.s3_access_key_id = credentials.access_key_id.clone();
            sync.s3_secret_access_key = credentials.secret_access_key.clone();
            sync.webdav_username = credentials.username.clone();
            sync.webdav_password = credentials.password.clone();
        }
        // `commit` re-enters `access()`; the guard above is already dropped.
        // A failed write leaves the persisted credentials behind the session
        // that was just opened, so the session goes with it: the caller sees an
        // error and nothing keeps syncing against a configuration the vault
        // does not hold.
        if let Err(e) = commit(&vault_state, &dir) {
            sync_state.clear();
            return Err(e);
        }
        sync_state.set(Some(SyncSession {
            keys: keys_for_task,
            provider: provider_for_task,
            prefix,
            device,
        }))?;
        Ok(())
    })
    .await
    .map_err(|e| format!("sync: task failed: {e}"))??;

    Ok(SyncConfigureResult {
        remote: match configured.remote {
            RemoteState::Created => "created",
            _ => "existing",
        }
        .to_string(),
    })
}

/// Close the session this process holds and forget the remote it was joined to.
///
/// While the vault is locked the payload is left alone: the scheduler calls
/// this again after the next unlock, which clears it then.
#[tauri::command]
pub async fn sync_disable(app: AppHandle) -> Result<(), String> {
    app.state::<SyncState>().clear();
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let vault_state = task_app.state::<VaultState>();
        let dir = crate::modules::vault::vault_dir(&task_app)?;
        {
            let mut guard = match vault_state.access() {
                Ok(guard) => guard,
                Err(_) => return Ok(()),
            };
            let Some(unlocked) = guard.as_mut() else {
                return Ok(());
            };
            // The scheduler calls this on every unlock while sync is off, and
            // a commit re-seals and rewrites the vault and its backup. A device
            // that never configured anything has nothing here, so the write is
            // skipped rather than paid on every launch.
            if unlocked.payload.device.sync == SyncDevice::default() {
                return Ok(());
            }
            unlocked.payload.device.sync.clear();
        }
        commit(&vault_state, &dir)
    })
    .await
    .map_err(|e| format!("sync: task failed: {e}"))??;
    Ok(())
}

/// Reconcile with the remote and land what it holds.
#[tauri::command]
pub async fn sync_pull(app: AppHandle) -> Result<SyncPullResult, String> {
    let session = app.state::<SyncState>().open()?;

    let (locals, etags, now) = {
        let vault_state = app.state::<VaultState>();
        let guard = vault_state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        let now = now_ms();
        (
            locals_from_payload(&unlocked.payload, now),
            unlocked.payload.device.sync.etags.clone(),
            now,
        )
    };

    let report = pull(
        session.provider.as_ref(),
        &session.keys,
        &session.prefix,
        &session.device,
        locals,
        etags,
        now,
    )
    .await
    .map_err(|e| e.to_string())?;

    let task_app = app.clone();
    let report_for_task = report.clone();
    let applied = tauri::async_runtime::spawn_blocking(move || -> Result<Applied, String> {
        let vault_state = task_app.state::<VaultState>();
        let dir = crate::modules::vault::vault_dir(&task_app)?;
        let applied = {
            let mut guard = vault_state.access()?;
            let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
            apply_pull(&mut unlocked.payload, &report_for_task, now)
        };
        commit(&vault_state, &dir)?;
        Ok(applied)
    })
    .await
    .map_err(|e| format!("sync: task failed: {e}"))??;

    emit_changed(&app, &applied.changed_ids, "sync");
    crate::modules::vault::drain_save_event(&app);
    crate::modules::vault::drain_auto_lock(&app);

    let mut quarantine = report.quarantined.clone();
    quarantine.extend(applied.quarantine.iter().cloned());
    Ok(SyncPullResult {
        pending: report.pending,
        landed: applied.landed,
        quarantine,
        stale: applied.stale.clone(),
    })
}

/// Publish the records the remote is missing.
#[tauri::command]
pub async fn sync_push(app: AppHandle) -> Result<SyncPushResult, String> {
    let session = app.state::<SyncState>().open()?;

    let (pushed, etags) = {
        let vault_state = app.state::<VaultState>();
        let mut guard = vault_state.access()?;
        let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
        take_dirty_envelopes(&mut unlocked.payload)
    };
    if pushed.is_empty() {
        return Ok(SyncPushResult {
            pushed: 0,
            failed: 0,
        });
    }

    let pushed_for_fold = pushed.clone();
    let report = push(
        session.provider.as_ref(),
        &session.keys,
        &session.prefix,
        &session.device,
        pushed,
        etags,
    )
    .await;

    let task_app = app.clone();
    let report_for_task = report.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let vault_state = task_app.state::<VaultState>();
        let dir = crate::modules::vault::vault_dir(&task_app)?;
        {
            let mut guard = vault_state.access()?;
            let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
            finish_push(&mut unlocked.payload, &pushed_for_fold, &report_for_task);
        }
        commit(&vault_state, &dir)
    })
    .await
    .map_err(|e| format!("sync: task failed: {e}"))??;

    crate::modules::vault::drain_save_event(&app);
    crate::modules::vault::drain_auto_lock(&app);
    Ok(SyncPushResult {
        pushed: report.etags.len(),
        failed: report.failed.len(),
    })
}

// ---------------------------------------------------------------------------
// Join
// ---------------------------------------------------------------------------

/// The records a join landed, before the vault file exists.
struct JoinedVault {
    keys: SyncKeys,
    payload: VaultPayload,
    landed: usize,
    quarantine: Vec<Quarantined>,
}

/// The keyfile and pull half of a join: no local file is touched here.
///
/// `Ok(None)` = no keyfile at the prefix, so a join cannot proceed and nothing
/// should be written.
#[allow(clippy::too_many_arguments)]
async fn join_pull(
    provider: &dyn SyncProvider,
    prefix: &str,
    passphrase: &str,
    device: &str,
    identity: &str,
    credentials: &SyncCredentialsArg,
    now: u64,
) -> Result<Option<JoinedVault>, String> {
    let key = keyfile_key(prefix);
    let Some(object) = provider.get(&key).await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let keyfile: SyncKeyfile =
        serde_json::from_slice(&object.bytes).map_err(|_| NOT_A_KEYFILE.to_string())?;
    let pass = passphrase.to_string();
    let root = tokio::task::spawn_blocking(move || open_keyfile_root(&keyfile, &pass))
        .await
        .map_err(|e| format!("sync: task failed: {e}"))??;
    let keys = expand_root(&root)?;

    let report = pull(
        provider,
        &keys,
        prefix,
        device,
        Vec::new(),
        BTreeMap::new(),
        now,
    )
    .await
    .map_err(|e| e.to_string())?;

    let mut payload = VaultPayload::default();
    let applied = apply_pull(&mut payload, &report, now);
    let seeded = seed_reserved_groups(&mut payload, now);
    for id in &seeded {
        payload.device.sync.mark_dirty(GROUP_KIND, id);
    }
    payload.device.sync.remote = Some(identity.to_string());
    payload.device.sync.root_key = Some(B64.encode(*root));
    payload.device.sync.s3_access_key_id = credentials.access_key_id.clone();
    payload.device.sync.s3_secret_access_key = credentials.secret_access_key.clone();
    payload.device.sync.webdav_username = credentials.username.clone();
    payload.device.sync.webdav_password = credentials.password.clone();

    let mut quarantine = report.quarantined.clone();
    quarantine.extend(applied.quarantine.iter().cloned());
    Ok(Some(JoinedVault {
        keys,
        payload,
        landed: applied.landed,
        quarantine,
    }))
}

/// Join an existing remote: pull it, install a new vault from what it holds,
/// and open a session.
///
/// NO LOCAL VAULT FILE IS WRITTEN UNTIL THE PULL SUCCEEDED, and none at all
/// when the prefix holds no keyfile.
#[tauri::command]
pub async fn sync_join(app: AppHandle, args: SyncJoinArgs) -> Result<SyncJoinResult, String> {
    let dir = crate::modules::vault::vault_dir(&app)?;
    if dir.join(VAULT_FILE_NAME).exists() || dir.join(format!("{VAULT_FILE_NAME}.bak")).exists() {
        return Err("vault: a vault file already exists".to_string());
    }
    {
        let vault_state = app.state::<VaultState>();
        let open = vault_state
            .access()
            .map(|guard| guard.is_some())
            .unwrap_or(false);
        if open {
            return Err("vault: a vault is already open".to_string());
        }
    }

    let identity = remote_identity(&args.config);
    let provider = build(
        &args.config.provider,
        provider_config(&args.config, &args.credentials)?,
    )
    .map_err(|e| e.to_string())?;
    let device = super::device_id(&app)?;
    let now = now_ms();

    let joined = join_pull(
        provider.as_ref(),
        &args.config.prefix,
        &args.passphrase,
        &device,
        &identity,
        &args.credentials,
        now,
    )
    .await?;
    let Some(joined) = joined else {
        return Ok(SyncJoinResult {
            remote: "fresh".to_string(),
            landed: 0,
            quarantine: Vec::new(),
        });
    };

    let JoinedVault {
        keys,
        payload,
        landed,
        quarantine,
    } = joined;
    let task_app = app.clone();
    let master_password = args.master_password.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let vault_state = task_app.state::<VaultState>();
        install_new_vault(&vault_state, &dir, &master_password, payload)
    })
    .await
    .map_err(|e| format!("sync: task failed: {e}"))??;

    app.state::<SyncState>().set(Some(SyncSession {
        keys: Arc::new(keys),
        provider: Arc::clone(&provider),
        prefix: args.config.prefix.clone(),
        device,
    }))?;

    Ok(SyncJoinResult {
        remote: "existing".to_string(),
        landed,
        quarantine,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::sync::crypto::{new_keyfile, new_keyfile_with_root};
    use crate::modules::sync::model::WIRE_VERSION;
    use crate::modules::sync::provider::{Caps, Entry as ProviderEntry, Object};
    use crate::modules::vault::file::{load_vault, open_file};
    use crate::modules::vault::model::{DeviceState, EntryDraft, BROWSER_ID, ROOT_ID, TRASH_ID};
    use serde_json::json;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Shadows the `std::sync::Mutex` the glob above brought in: the fake locks
    // and immediately unlocks, so `lock()` returning the guard directly is
    // exactly what these call sites want.
    use parking_lot::Mutex;

    const DAY: u64 = 24 * 60 * 60 * 1000;
    /// "Now" for every test here, far enough from the epoch that a stamp a
    /// hundred days before it is still positive.
    const NOW: u64 = 1_800_000_000_000;
    const PREFIX: &str = "subclave";
    const STRONG: &str = "Zx9!pQ2#vLm7@Wr4Tk1$";

    /// A provider that keeps its objects in memory and counts what it was
    /// asked to do.
    ///
    /// The counts are the point rather than a convenience: half the properties
    /// below are "this object was NOT fetched" and "no put was issued", and
    /// neither is visible from a return value.
    type Stored = (Vec<u8>, String, Option<u64>);

    #[derive(Default)]
    struct Fake {
        objects: Mutex<BTreeMap<String, Stored>>,
        gets: Mutex<Vec<String>>,
        puts: Mutex<Vec<String>>,
        deletes: Mutex<Vec<String>>,
        calls: Mutex<Vec<String>>,
        cas: bool,
        /// Keys whose next conditional put is rejected once, to reach the retry.
        reject_once: Mutex<BTreeSet<String>>,
        /// Keys the next `get` reports as absent, to reach the window between
        /// the read that found nothing and the write that follows it.
        hidden_once: Mutex<BTreeSet<String>>,
        /// What a read-only bucket, an object lock or a lifecycle policy does
        /// to every delete this provider is asked for.
        refuse_deletes: AtomicBool,
    }

    impl Fake {
        fn cas(cas: bool) -> Self {
            Self {
                cas,
                ..Default::default()
            }
        }

        fn seed(&self, key: &str, bytes: Vec<u8>, etag: &str, modified_at: Option<u64>) {
            self.objects
                .lock()
                .insert(key.into(), (bytes, etag.into(), modified_at));
        }

        fn gets(&self) -> Vec<String> {
            self.gets.lock().clone()
        }
        fn puts(&self) -> Vec<String> {
            self.puts.lock().clone()
        }
        fn deletes(&self) -> Vec<String> {
            self.deletes.lock().clone()
        }
        fn last_call(&self) -> Option<String> {
            self.calls.lock().last().cloned()
        }
        fn object(&self, key: &str) -> Option<Vec<u8>> {
            self.objects
                .lock()
                .get(key)
                .map(|(bytes, _, _)| bytes.clone())
        }
    }

    impl SyncProvider for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }

        fn capabilities(&self) -> Caps {
            Caps { cas: self.cas }
        }

        fn get<'a>(
            &'a self,
            key: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Option<Object>, ProviderError>> + Send + 'a>>
        {
            Box::pin(async move {
                self.calls.lock().push(format!("get:{key}"));
                self.gets.lock().push(key.to_string());
                if self.hidden_once.lock().remove(key) {
                    return Ok(None);
                }
                Ok(self.objects.lock().get(key).map(|(bytes, etag, _)| Object {
                    bytes: bytes.clone(),
                    etag: etag.clone(),
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
                if self.cas && if_match.is_some() && self.reject_once.lock().remove(key) {
                    return Err(ProviderError::PreconditionFailed);
                }
                self.calls.lock().push(format!("put:{key}"));
                self.puts.lock().push(key.to_string());
                let etag = format!("etag-{}", self.puts.lock().len());
                let modified = self.objects.lock().get(key).and_then(|(_, _, m)| *m);
                self.objects
                    .lock()
                    .insert(key.into(), (bytes, etag.clone(), modified));
                Ok(etag)
            })
        }

        fn put_if_absent<'a>(
            &'a self,
            key: &'a str,
            bytes: Vec<u8>,
        ) -> Pin<Box<dyn Future<Output = Result<Option<String>, ProviderError>> + Send + 'a>>
        {
            Box::pin(async move {
                self.calls.lock().push(format!("put_if_absent:{key}"));
                // A provider that cannot condition a write sends it anyway and
                // reports success; one that can reports the object it found.
                if self.cas && self.objects.lock().contains_key(key) {
                    return Ok(None);
                }
                self.puts.lock().push(key.to_string());
                let etag = format!("etag-if-{}", self.puts.lock().len());
                let modified = self.objects.lock().get(key).and_then(|(_, _, m)| *m);
                self.objects
                    .lock()
                    .insert(key.into(), (bytes, etag.clone(), modified));
                Ok(Some(etag))
            })
        }

        fn list<'a>(
            &'a self,
            prefix: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<ProviderEntry>, ProviderError>> + Send + 'a>>
        {
            Box::pin(async move {
                self.calls.lock().push(format!("list:{prefix}"));
                Ok(self
                    .objects
                    .lock()
                    .iter()
                    .filter(|(key, _)| key.starts_with(prefix))
                    .map(|(key, (_, etag, modified_at))| ProviderEntry {
                        key: key.clone(),
                        etag: etag.clone(),
                        modified_at: *modified_at,
                    })
                    .collect())
            })
        }

        fn delete<'a>(
            &'a self,
            key: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), ProviderError>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.lock().push(format!("delete:{key}"));
                self.deletes.lock().push(key.to_string());
                if self.refuse_deletes.load(Ordering::SeqCst) {
                    return Err(ProviderError::Remote {
                        status: 403,
                        code: Some("AccessDenied".into()),
                    });
                }
                self.objects.lock().remove(key);
                Ok(())
            })
        }
    }

    /// A private directory under the system temp dir, removed on drop.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subclave-sync-engine-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id(),
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn is_empty(&self) -> bool {
            std::fs::read_dir(&self.0)
                .expect("read temp dir")
                .next()
                .is_none()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn keys() -> SyncKeys {
        new_keyfile("correct horse").expect("new keyfile").1
    }

    fn env(kind: &str, id: &str, updated_at: u64, device: &str, record: Value) -> Envelope {
        Envelope {
            v: WIRE_VERSION,
            kind: kind.into(),
            id: id.into(),
            updated_at: Some(updated_at),
            device: device.into(),
            deleted: false,
            record,
        }
    }

    fn test_entry(id: &str, title: &str, updated_at: u64) -> Entry {
        Entry {
            id: id.into(),
            group_id: ROOT_ID.into(),
            title: title.into(),
            username: "user".into(),
            password: "pw".into(),
            urls: vec![],
            notes: String::new(),
            totp: None,
            custom_fields: vec![],
            tags: vec![],
            icon: None,
            color: None,
            favorite: false,
            expires_at: None,
            trashed_from: None,
            created_at: 0,
            updated_at,
            history: vec![],
            last_used_at: None,
        }
    }

    fn entry_env(id: &str, updated_at: u64, device: &str, title: &str) -> Envelope {
        let entry = test_entry(id, title, updated_at);
        env(
            ENTRY_KIND,
            id,
            updated_at,
            device,
            serde_json::to_value(&entry).expect("entry serialization"),
        )
    }

    fn grave(kind: &str, id: &str, deleted_at: u64, device: &str) -> Envelope {
        Envelope {
            deleted: true,
            record: Value::Null,
            ..env(kind, id, deleted_at, device, Value::Null)
        }
    }

    fn payload_of(entries: Vec<Entry>, tombstones: Vec<Tombstone>) -> VaultPayload {
        VaultPayload {
            entries,
            groups: vec![],
            tombstones,
            device: DeviceState::default(),
        }
    }

    /// Put one envelope on the fake as a real sealed object, at the key the
    /// engine will look for it under.
    fn publish(
        fake: &Fake,
        keys: &SyncKeys,
        envelope: &Envelope,
        etag: &str,
        modified: Option<u64>,
    ) {
        let key = object_key(PREFIX, &object_name(keys, &envelope.kind, &envelope.id));
        fake.seed(&key, seal_envelope(keys, envelope).unwrap(), etag, modified);
    }

    fn key_of(keys: &SyncKeys, envelope: &Envelope) -> String {
        object_key(PREFIX, &object_name(keys, &envelope.kind, &envelope.id))
    }

    fn outcome<'a>(report: &'a PullReport, id: &str) -> &'a Outcome {
        &report
            .records
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no disposition for {id}"))
            .outcome
    }

    async fn pull_with(
        fake: &Fake,
        keys: &SyncKeys,
        locals: Vec<Envelope>,
        etags: BTreeMap<String, String>,
    ) -> PullReport {
        pull(fake, keys, PREFIX, "this-device", locals, etags, NOW)
            .await
            .expect("pull")
    }

    fn config() -> SyncConfigArg {
        SyncConfigArg {
            provider: "s3".into(),
            endpoint: "https://storage.example".into(),
            region: "us-east-1".into(),
            bucket: "subclave".into(),
            prefix: PREFIX.into(),
            cas: true,
        }
    }

    #[test]
    fn the_minted_root_is_the_one_the_keyfile_opens_to() {
        // The persist-then-reopen path a configured device runs on every
        // launch: the root that goes into `SyncDevice::root_key` is the one
        // the keyfile on the remote wraps, or the device would publish objects
        // it can never read back.
        let (kf, root) = new_keyfile_with_root("correct horse").expect("keyfile");
        let reopened = open_keyfile_root(&kf, "correct horse").expect("open root");
        assert_eq!(&root[..], &reopened[..]);

        let keys = expand_root(&root).expect("expand");
        let recovered = expand_root(&reopened).expect("expand");
        assert_eq!(
            object_name(&keys, ENTRY_KIND, "e1"),
            object_name(&recovered, ENTRY_KIND, "e1")
        );
        let sealed = seal_record(&keys, b"hello").expect("seal");
        assert_eq!(&*open_record(&recovered, &sealed).expect("open"), b"hello");
        assert!(open_keyfile_root(&kf, "wrong").is_err());
    }

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

    #[tokio::test]
    async fn the_union_pairs_a_local_tombstone_with_its_remote_counterpart() {
        let keys = keys();
        let fake = Fake::cas(true);
        let remote_live = entry_env("h-1", NOW - 9000, "dev-b", "still here");
        let remote_new = entry_env("h-2", NOW - 8000, "dev-b", "new over there");
        publish(&fake, &keys, &remote_live, "e1", Some(NOW - 9000));
        publish(&fake, &keys, &remote_new, "e2", Some(NOW - 8000));

        let locals = vec![
            grave(ENTRY_KIND, "h-1", NOW - 1000, "this-device"),
            entry_env("h-3", NOW - 500, "this-device", "only here"),
        ];
        let report = pull_with(&fake, &keys, locals, BTreeMap::new()).await;

        // The local delete is newer, so the merge resolves to the tombstone -
        // and it is a `Merged`, not a `RemoteOnly`.
        match outcome(&report, "h-1") {
            Outcome::Merged {
                envelope,
                republish,
                ..
            } => {
                assert!(envelope.deleted, "the delete lost to the remote record");
                assert!(republish, "the remote still holds the deleted record");
            }
            other => panic!("h-1: {other:?}"),
        }
        assert!(matches!(
            outcome(&report, "h-2"),
            Outcome::RemoteOnly { .. }
        ));
        assert!(matches!(
            outcome(&report, "h-3"),
            Outcome::LocalOnly { stale: false }
        ));
        assert_eq!(report.pending, 2);
    }

    #[tokio::test]
    async fn a_local_only_record_older_than_the_window_is_reported_and_not_deleted() {
        let keys = keys();
        let fake = Fake::cas(true);
        // THE REMOTE HOLDS SOMETHING, and that is load-bearing: an empty
        // listing lifts the stale rule outright, so a version of this test with
        // nothing published would pass for the wrong reason.
        let theirs = entry_env("h-9", NOW - 9000, "dev-b", "over there");
        publish(&fake, &keys, &theirs, "e1", Some(NOW - 9000));
        let locals = vec![
            entry_env("h-1", NOW - 100 * DAY, "this-device", "long gone elsewhere"),
            entry_env("h-2", NOW - 1000, "this-device", "recent"),
        ];
        let report = pull_with(&fake, &keys, locals, BTreeMap::new()).await;

        assert!(matches!(
            outcome(&report, "h-1"),
            Outcome::LocalOnly { stale: true }
        ));
        assert!(matches!(
            outcome(&report, "h-2"),
            Outcome::LocalOnly { stale: false }
        ));
        // A stale record is not pending either: reporting it is the whole
        // disposition, and counting it would keep a badge lit forever.
        assert_eq!(report.pending, 1);
        assert!(fake.deletes().is_empty(), "a listing gap deleted a record");
        assert!(fake.puts().is_empty(), "the pull wrote objects itself");
    }

    #[tokio::test]
    async fn an_empty_remote_is_never_stale_so_the_whole_inventory_publishes() {
        // A prefix with no objects in it has never held any of these, so the
        // reading the stale rule makes is unavailable and every record is owed.
        let keys = keys();
        let fake = Fake::cas(true);
        let locals = vec![
            entry_env(
                "h-1",
                NOW - 100 * DAY,
                "this-device",
                "older than the window",
            ),
            entry_env("h-2", NOW - 1000, "this-device", "recent"),
        ];
        let report = pull_with(&fake, &keys, locals, BTreeMap::new()).await;

        assert!(matches!(
            outcome(&report, "h-1"),
            Outcome::LocalOnly { stale: false }
        ));
        assert!(matches!(
            outcome(&report, "h-2"),
            Outcome::LocalOnly { stale: false }
        ));
        assert_eq!(report.pending, 2);
        assert!(fake.puts().is_empty(), "the pull wrote objects itself");
        assert!(fake.deletes().is_empty());
    }

    #[tokio::test]
    async fn an_etag_that_matches_is_not_fetched_and_a_prune_candidate_still_is() {
        let keys = keys();
        let fake = Fake::cas(true);
        let unchanged = entry_env("h-1", NOW - 9000, "dev-b", "unchanged");
        let moved = entry_env("h-2", NOW - 8000, "dev-b", "moved");
        let old_grave = grave(ENTRY_KIND, "h-3", NOW - 100 * DAY, "this-device");
        publish(&fake, &keys, &unchanged, "same", Some(NOW - 9000));
        publish(&fake, &keys, &moved, "different", Some(NOW - 8000));
        publish(&fake, &keys, &old_grave, "ancient", Some(NOW - 100 * DAY));

        let etags = BTreeMap::from([
            ("entry:h-1".to_string(), "same".to_string()),
            ("entry:h-2".to_string(), "stale".to_string()),
            ("entry:h-3".to_string(), "ancient".to_string()),
        ]);
        let locals = vec![entry_env("h-1", NOW - 9000, "this-device", "unchanged")];
        let report = pull_with(&fake, &keys, locals, etags).await;

        let fetched = fake.gets();
        assert!(
            !fetched.contains(&key_of(&keys, &unchanged)),
            "an unchanged object was downloaded again"
        );
        assert!(fetched.contains(&key_of(&keys, &moved)));
        // The prune candidate's etag matched the map and it was fetched anyway.
        assert!(
            fetched.contains(&key_of(&keys, &old_grave)),
            "the prune candidate was etag-skipped"
        );
        assert_eq!(report.pruned, 1);
        assert!(
            !report.records.iter().any(|r| r.id == "h-1"),
            "a skipped object still produced a disposition"
        );
        assert_eq!(report.etags.get("entry:h-1"), Some(&"same".to_string()));
    }

    #[tokio::test]
    async fn the_prune_removes_only_this_devices_own_expired_tombstones() {
        let keys = keys();
        let fake = Fake::cas(true);
        let mine_old = grave(ENTRY_KIND, "h-1", NOW - 100 * DAY, "this-device");
        let mine_recent = grave(ENTRY_KIND, "h-2", NOW - DAY, "this-device");
        let theirs_old = grave(ENTRY_KIND, "h-3", NOW - 100 * DAY, "dev-b");
        publish(&fake, &keys, &mine_old, "a", Some(NOW - 100 * DAY));
        publish(&fake, &keys, &mine_recent, "b", Some(NOW - DAY));
        publish(&fake, &keys, &theirs_old, "c", Some(NOW - 100 * DAY));

        let etags = BTreeMap::from([
            ("entry:h-1".to_string(), "a".to_string()),
            ("entry:h-2".to_string(), "b".to_string()),
            ("entry:h-3".to_string(), "c".to_string()),
        ]);
        let report = pull_with(&fake, &keys, Vec::new(), etags).await;

        assert_eq!(
            fake.deletes(),
            vec![key_of(&keys, &mine_old)],
            "the prune deleted the wrong set"
        );
        assert_eq!(report.pruned, 1);
        assert!(!report.etags.contains_key("entry:h-1"));
        assert!(!report.records.iter().any(|r| r.id == "h-3"));
        assert_eq!(report.etags.get("entry:h-2"), Some(&"b".to_string()));
    }

    #[tokio::test]
    async fn an_object_that_does_not_open_is_quarantined_and_leaves_the_map_alone() {
        let keys = keys();
        let fake = Fake::cas(true);
        let good = entry_env("h-1", NOW - 1000, "dev-b", "fine");
        publish(&fake, &keys, &good, "e1", Some(NOW - 1000));
        fake.seed(
            &object_key(PREFIX, "deadbeef"),
            b"not a sealed record at all".to_vec(),
            "e2",
            Some(NOW - 1000),
        );

        let report = pull_with(&fake, &keys, Vec::new(), BTreeMap::new()).await;
        assert_eq!(report.quarantined.len(), 1);
        assert_eq!(report.quarantined[0].name, "deadbeef");
        assert!(matches!(
            outcome(&report, "h-1"),
            Outcome::RemoteOnly { .. }
        ));
        assert_eq!(report.etags.len(), 1);
    }

    #[tokio::test]
    async fn an_object_from_a_newer_build_is_quarantined_rather_than_landed() {
        let keys = keys();
        let fake = Fake::cas(true);
        let mut future = entry_env("h-1", NOW - 1000, "dev-b", "from a newer build");
        future.v = WIRE_VERSION + 1;
        let good = entry_env("h-2", NOW - 1000, "dev-b", "readable");
        publish(&fake, &keys, &future, "e1", Some(NOW - 1000));
        publish(&fake, &keys, &good, "e2", Some(NOW - 1000));

        let report = pull_with(&fake, &keys, Vec::new(), BTreeMap::new()).await;
        assert!(
            !report.records.iter().any(|r| r.id == "h-1"),
            "a newer-build object reached the apply path"
        );
        assert_eq!(report.quarantined.len(), 1);
        assert!(
            report.quarantined[0].reason.contains("newer build"),
            "unexpected: {}",
            report.quarantined[0].reason
        );
        assert!(!report.etags.contains_key("entry:h-1"));
        assert!(matches!(
            outcome(&report, "h-2"),
            Outcome::RemoteOnly { .. }
        ));
    }

    #[tokio::test]
    async fn unknown_kinds_are_quarantined() {
        let keys = keys();
        let fake = Fake::cas(true);
        let widget = env("widget", "w-1", NOW - 1000, "dev-b", json!({"id": "w-1"}));
        publish(&fake, &keys, &widget, "e1", Some(NOW - 1000));

        let report = pull_with(&fake, &keys, Vec::new(), BTreeMap::new()).await;
        assert!(report.records.is_empty(), "{:?}", report.records);
        assert_eq!(report.quarantined.len(), 1);
        assert!(!report.etags.contains_key("widget:w-1"));
    }

    #[tokio::test]
    async fn a_refused_prune_delete_does_not_abort_the_pull() {
        let keys = keys();
        let fake = Fake::cas(true);
        fake.refuse_deletes.store(true, Ordering::SeqCst);
        let mine_old = grave(ENTRY_KIND, "h-1", NOW - 100 * DAY, "this-device");
        let live = entry_env("h-2", NOW - 1000, "dev-b", "still wanted");
        publish(&fake, &keys, &mine_old, "a", Some(NOW - 100 * DAY));
        publish(&fake, &keys, &live, "b", Some(NOW - 1000));

        let report = pull_with(&fake, &keys, Vec::new(), BTreeMap::new()).await;
        assert_eq!(report.pruned, 0, "a refused delete was counted as pruned");
        assert!(matches!(
            outcome(&report, "h-2"),
            Outcome::RemoteOnly { .. }
        ));
        assert!(
            !report.records.iter().any(|r| r.id == "h-1"),
            "an expired tombstone was handed to the apply path"
        );
    }

    #[tokio::test]
    async fn another_devices_expired_tombstone_is_not_landed_either() {
        let keys = keys();
        let fake = Fake::cas(true);
        let theirs = grave(ENTRY_KIND, "h-3", NOW - 100 * DAY, "dev-b");
        publish(&fake, &keys, &theirs, "c", Some(NOW - 100 * DAY));

        let report = pull_with(&fake, &keys, Vec::new(), BTreeMap::new()).await;
        assert!(
            fake.deletes().is_empty(),
            "another device's object was pruned"
        );
        assert!(report.records.is_empty(), "{:?}", report.records);
    }

    #[tokio::test]
    async fn a_tombstone_inside_the_window_is_still_reconciled() {
        let keys = keys();
        let fake = Fake::cas(true);
        let recent = grave(ENTRY_KIND, "h-4", NOW - DAY, "dev-b");
        publish(&fake, &keys, &recent, "d", Some(NOW - DAY));

        let report = pull_with(&fake, &keys, Vec::new(), BTreeMap::new()).await;
        assert!(matches!(
            outcome(&report, "h-4"),
            Outcome::RemoteOnly { .. }
        ));
    }

    #[tokio::test]
    async fn a_rejected_conditional_write_pulls_merges_and_retries_without_an_error() {
        let keys = keys();
        let fake = Fake::cas(true);
        let theirs = entry_env("h-1", NOW - 1000, "dev-b", "theirs, newer");
        publish(&fake, &keys, &theirs, "current", Some(NOW - 1000));
        fake.reject_once.lock().insert(key_of(&keys, &theirs));

        let mine = entry_env("h-1", NOW - 5000, "this-device", "mine, older");
        let etags = BTreeMap::from([("entry:h-1".to_string(), "stale".to_string())]);
        let report = push(&fake, &keys, PREFIX, "this-device", vec![mine], etags).await;

        assert!(report.failed.is_empty(), "unexpected: {:?}", report.failed);
        // The retry recomputed the winner from the live remote copy. Entries
        // union their histories, so the loser survives as a `conflict` version
        // rather than being dropped - but the stored record is the NEWER
        // copy's, never the stale one this device tried to write.
        assert_eq!(fake.puts(), vec![key_of(&keys, &theirs)]);
        let stored = fake.object(&key_of(&keys, &theirs)).expect("stored");
        let published = open_envelope(&keys, &stored).expect("open");
        assert_eq!(published.record["title"], "theirs, newer");
        assert!(report.etags.contains_key("entry:h-1"));
    }

    #[tokio::test]
    async fn a_retry_that_wins_the_merge_writes_the_winner_conditionally() {
        let keys = keys();
        let fake = Fake::cas(true);
        let theirs = entry_env("h-1", NOW - 9000, "dev-b", "theirs, older");
        publish(&fake, &keys, &theirs, "current", Some(NOW - 9000));
        fake.reject_once.lock().insert(key_of(&keys, &theirs));

        let mine = entry_env("h-1", NOW - 1000, "this-device", "mine, newer");
        let etags = BTreeMap::from([("entry:h-1".to_string(), "stale".to_string())]);
        let report = push(&fake, &keys, PREFIX, "this-device", vec![mine], etags).await;

        assert!(report.failed.is_empty(), "unexpected: {:?}", report.failed);
        assert_eq!(fake.puts(), vec![key_of(&keys, &theirs)]);
        let stored = fake.object(&key_of(&keys, &theirs)).expect("stored");
        assert_eq!(
            open_envelope(&keys, &stored).unwrap().record["title"],
            "mine, newer"
        );
    }

    #[test]
    fn the_conditional_write_setting_reaches_the_provider() {
        // The Settings switch has to survive the whole path: `SyncConfigArg`
        // from the webview, through `provider_config`'s JSON, into the
        // provider's own `Caps`.
        let creds = SyncCredentialsArg {
            access_key_id: Some("ak".into()),
            secret_access_key: Some("sk".into()),
            ..Default::default()
        };
        for cas in [true, false] {
            let arg = SyncConfigArg { cas, ..config() };
            let built = build(
                &arg.provider,
                provider_config(&arg, &creds).expect("an s3 configuration"),
            )
            .expect("the provider builds");
            assert_eq!(built.capabilities().cas, cas);
        }
    }

    #[tokio::test]
    async fn the_conditional_write_setting_decides_whether_a_create_race_is_noticed() {
        // Seen from the side the switch protects. With it on, a device that
        // finds a keyfile already at the prefix joins the winner's root key.
        // With it off the write goes out unconditionally, so the loser of a
        // real race replaces the keyfile and loses every object it sealed
        // under the root it minted. This is what turning the switch on buys.
        let (winner, winner_root) = new_keyfile_with_root(STRONG).expect("keyfile");
        let seed = |fake: &Fake| {
            fake.seed(
                &keyfile_key(PREFIX),
                serde_json::to_vec(&winner).expect("keyfile json"),
                "kf",
                None,
            );
            // The object is there, but the read that decides whether to mint
            // reports it absent: the competitor landed it between this device's
            // read and its write, which is the whole window.
            fake.hidden_once.lock().insert(keyfile_key(PREFIX));
        };

        let conditioned = {
            let fake = Fake::cas(true);
            seed(&fake);
            configure_keyfile(&fake, PREFIX, Some(STRONG), true, None)
                .await
                .expect("configure")
        };
        assert!(matches!(conditioned.remote, RemoteState::Existing));
        assert_eq!(
            &conditioned.root.expect("root")[..],
            &winner_root[..],
            "a conditional create has to keep the keyfile that was already there"
        );

        let unconditioned = {
            let fake = Fake::cas(false);
            seed(&fake);
            configure_keyfile(&fake, PREFIX, Some(STRONG), true, None)
                .await
                .expect("configure")
        };
        assert!(matches!(unconditioned.remote, RemoteState::Created));
        assert_ne!(
            &unconditioned.root.expect("root")[..],
            &winner_root[..],
            "an unconditional create replaced a keyfile it never looked at"
        );
    }

    #[tokio::test]
    async fn a_create_goes_out_unconditionally_and_a_non_cas_provider_never_conditions() {
        // Two ways to reach an unconditional put, and only one of them is a
        // capability: a record with no etag has no condition to send.
        for cas in [true, false] {
            let keys = keys();
            let fake = Fake::cas(cas);
            let mine = entry_env("h-1", NOW - 1000, "this-device", "fresh");
            fake.reject_once.lock().insert(key_of(&keys, &mine));
            let report = push(
                &fake,
                &keys,
                PREFIX,
                "this-device",
                vec![mine.clone()],
                BTreeMap::new(),
            )
            .await;
            assert!(report.failed.is_empty(), "cas={cas}: {:?}", report.failed);
            assert_eq!(fake.puts().len(), 1, "cas={cas}");
            assert!(report.etags.contains_key("entry:h-1"), "cas={cas}");
        }
    }

    #[tokio::test]
    async fn the_push_stamps_this_device_over_whatever_it_was_handed() {
        let keys = keys();
        let fake = Fake::cas(false);
        let mut claimed = entry_env("h-1", NOW - 1000, "somebody-elses-id", "fresh");
        claimed.device = "somebody-elses-id".into();
        push(
            &fake,
            &keys,
            PREFIX,
            "this-device",
            vec![claimed.clone()],
            BTreeMap::new(),
        )
        .await;

        let stored = fake.object(&key_of(&keys, &claimed)).expect("stored");
        assert_eq!(open_envelope(&keys, &stored).unwrap().device, "this-device");
    }

    #[tokio::test]
    async fn a_published_record_carries_no_device_local_field() {
        // `lastUsedAt` describes this machine. A wholesale write of a pulled
        // record would delete the receiving device's own last-used stamp, so
        // the publishing side is where it has to stop.
        let keys = keys();
        let fake = Fake::cas(false);
        let mut mine = entry_env("h-1", NOW - 1000, "this-device", "used here");
        mine.record = serde_json::to_value(test_entry("h-1", "used here", NOW - 1000))
            .expect("entry serialization");
        mine.record["lastUsedAt"] = json!(1700);
        push(
            &fake,
            &keys,
            PREFIX,
            "this-device",
            vec![mine.clone()],
            BTreeMap::new(),
        )
        .await;

        let stored = fake.object(&key_of(&keys, &mine)).expect("stored");
        let published = open_envelope(&keys, &stored).unwrap();
        assert!(published.record.get("lastUsedAt").is_none());
        assert!(published.record.get("last_used_at").is_none());
        assert_eq!(published.record["title"], "used here");
    }

    #[tokio::test]
    async fn a_remote_win_that_needs_no_republish_is_not_counted_as_pending() {
        let keys = keys();
        let fake = Fake::cas(true);
        let shared = entry_env("h-1", NOW - 1000, "dev-b", "agreed");
        publish(&fake, &keys, &shared, "e1", Some(NOW - 1000));

        let mine = entry_env("h-1", NOW - 1000, "this-device", "agreed");
        let report = pull_with(&fake, &keys, vec![mine], BTreeMap::new()).await;

        match outcome(&report, "h-1") {
            Outcome::Merged {
                changed, republish, ..
            } => {
                assert!(!changed, "an agreed pair rewrote the store");
                assert!(!republish, "an agreed pair republished");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(report.pending, 0);
    }

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
    async fn a_stored_root_key_opens_the_remote_without_the_passphrase() {
        // Every session after the first one takes this path: the scheduler
        // re-configures with no passphrase and no credentials, and
        // `SyncDevice::root_key` is what stands in for them.
        let fake = Fake::cas(true);
        let (keyfile, root) = new_keyfile_with_root(STRONG).expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );
        let stored = B64.encode(&root[..]);
        let configured = configure_keyfile(&fake, PREFIX, None, false, Some(&stored))
            .await
            .expect("a stored root opens the keyfile");
        assert!(matches!(configured.remote, RemoteState::Existing));
        assert_eq!(&configured.root.expect("root")[..], &root[..]);
        assert!(fake.puts().is_empty(), "a re-open wrote something");
    }

    #[tokio::test]
    async fn a_corrupt_stored_root_key_is_refused() {
        let fake = Fake::cas(true);
        let (keyfile, _) = new_keyfile_with_root(STRONG).expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );
        let err = match configure_keyfile(&fake, PREFIX, None, false, Some("not base64")).await {
            Ok(_) => panic!("a corrupt stored root must not fall back to anything"),
            Err(e) => e,
        };
        assert_eq!(err, "sync: the stored root key is corrupt");
    }

    #[tokio::test]
    async fn a_weak_passphrase_is_refused_before_a_keyfile_is_minted() {
        let fake = Fake::cas(true);
        let err = match configure_keyfile(&fake, PREFIX, Some("password"), true, None).await {
            Ok(_) => panic!("a weak passphrase must not become a sync passphrase"),
            Err(e) => e,
        };
        assert!(
            err.starts_with("sync: the sync passphrase is too weak"),
            "{err}"
        );
        assert!(fake.puts().is_empty(), "a weak passphrase still minted one");
    }

    #[tokio::test]
    async fn a_stale_local_record_is_reported_and_not_pushed() {
        let keys = keys();
        let fake = Fake::cas(true);
        let theirs = entry_env("h-9", NOW - 9000, "dev-b", "over there");
        publish(&fake, &keys, &theirs, "e1", Some(NOW - 9000));
        let mut payload = payload_of(
            vec![test_entry("h-1", "long gone elsewhere", NOW - 100 * DAY)],
            vec![],
        );
        let report = pull_with(
            &fake,
            &keys,
            locals_from_payload(&payload, NOW),
            BTreeMap::new(),
        )
        .await;
        let applied = apply_pull(&mut payload, &report, NOW);
        let expected = SlotRef {
            kind: ENTRY_KIND.into(),
            id: "h-1".into(),
        };
        assert_eq!(applied.stale, vec![expected]);
        assert!(!payload.device.sync.dirty.contains("entry:h-1"));
        let (envelopes, _) = take_dirty_envelopes(&mut payload);
        assert!(!envelopes.iter().any(|e| e.id == "h-1"));
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

    #[test]
    fn a_config_change_drops_the_stored_root_key() {
        let mut device = SyncDevice {
            remote: Some("a".into()),
            root_key: Some("root".into()),
            etags: BTreeMap::from([("entry:e1".to_string(), "x".to_string())]),
            dirty: BTreeSet::from(["entry:e1".to_string()]),
            ..Default::default()
        };
        assert!(reset_for_identity(&mut device, "b"));
        assert_eq!(device.root_key, None);
        assert_eq!(device.remote.as_deref(), Some("b"));
        assert!(device.etags.is_empty());
        assert!(device.dirty.contains("entry:e1"));
        // The same identity is a no-op, so an unchanged remote keeps its root.
        device.root_key = Some("root".into());
        assert!(!reset_for_identity(&mut device, "b"));
        assert_eq!(device.root_key.as_deref(), Some("root"));
    }

    #[test]
    fn a_configured_but_disabled_session_makes_no_request() {
        // A device whose user configured sync last week answers this way until
        // a caller reopens the session, and an empty state issues no request.
        let state = SyncState::default();
        let err = state.open().err().expect("an empty state must refuse");
        assert_eq!(err, NOT_CONFIGURED);
        let fake = Fake::cas(true);
        assert!(fake.gets().is_empty());
        assert!(fake.puts().is_empty());
        assert!(fake.deletes().is_empty());
    }

    #[tokio::test]
    async fn configure_stores_the_root_key_and_not_the_passphrase() {
        let fake = Fake::cas(true);
        let configured = configure_keyfile(&fake, PREFIX, Some(STRONG), true, None)
            .await
            .expect("configure");
        assert_eq!(configured.remote, RemoteState::Created);
        let root = configured.root.expect("a minted keyfile has a root key");
        let payload = VaultPayload {
            device: DeviceState {
                sync: SyncDevice {
                    remote: Some(remote_identity(&config())),
                    root_key: Some(B64.encode(*root)),
                    ..Default::default()
                },
            },
            ..Default::default()
        };
        assert!(payload.device.sync.root_key.is_some());
        let serialized = serde_json::to_string(&payload).expect("payload json");
        assert!(
            !serialized.contains(STRONG),
            "the passphrase reached the payload"
        );
    }

    #[tokio::test]
    async fn the_minted_keyfile_carries_the_default_argon2_params() {
        let fake = Fake::cas(true);
        let configured = configure_keyfile(&fake, PREFIX, Some(STRONG), true, None)
            .await
            .expect("configure");
        assert_eq!(configured.remote, RemoteState::Created);
        let bytes = fake.object(&keyfile_key(PREFIX)).expect("keyfile written");
        let keyfile: SyncKeyfile = serde_json::from_slice(&bytes).expect("keyfile");
        assert_eq!(keyfile.format, "subclave-sync");
        assert_eq!(keyfile.kdf.memory_kib, 65536);
        assert_eq!(keyfile.kdf.iterations, 3);
        assert_eq!(keyfile.kdf.parallelism, 4);
    }

    #[tokio::test]
    async fn a_lost_create_race_joins_the_winner() {
        let fake = Fake::cas(true);
        let (winner_keyfile, winner_keys) = new_keyfile("correct horse").expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&winner_keyfile).expect("keyfile json"),
            "kf",
            None,
        );

        let configured = configure_keyfile(&fake, PREFIX, Some("correct horse"), true, None)
            .await
            .expect("configure");
        assert_eq!(configured.remote, RemoteState::Existing);
        assert!(fake.puts().is_empty(), "the loser overwrote the winner");
        let root = configured.root.expect("the winner's root");
        let recovered = expand_root(&root).expect("expand");

        let envelope = entry_env("h-1", NOW, "dev-b", "over there");
        let sealed = seal_envelope(&winner_keys, &envelope).expect("seal");
        assert_eq!(open_envelope(&recovered, &sealed).expect("open").id, "h-1");
    }

    #[tokio::test]
    async fn an_edit_saved_but_not_pushed_is_pushed_on_the_next_session() {
        // The executable form of "kill the app after an edit is saved and
        // before it is pushed; the next unlock pushes the edit". A real kill is
        // not automatable here, so the file the next unlock would read is what
        // this reopens.
        let dir = TempDir::new("crash");
        let state = VaultState::default();
        crate::modules::vault::vault_create_inner(&state, &dir.0, "master-password")
            .expect("create vault");
        let draft = EntryDraft {
            id: None,
            group_id: ROOT_ID.into(),
            title: "edited".into(),
            username: "user".into(),
            password: Some("pw".into()),
            urls: vec![],
            notes: String::new(),
            totp: None,
            custom_fields: vec![],
            tags: vec![],
            icon: None,
            color: None,
            favorite: false,
            expires_at: None,
        };
        let summary =
            crate::modules::vault::vault_entry_upsert_inner(&state, &dir.0, draft).expect("upsert");
        let entry_id = summary.id.clone();
        {
            let guard = state.access().expect("access");
            let unlocked = guard.as_ref().expect("unlocked");
            assert!(
                unlocked
                    .payload
                    .device
                    .sync
                    .dirty
                    .contains(&format!("entry:{entry_id}")),
                "the edit did not mark its slot dirty"
            );
        }

        // "Crash": drop the state and reopen the file the way
        // `vault_unlock_inner` does.
        drop(state);
        let (file, _from_bak) = load_vault(&dir.0).expect("load vault");
        let opened = open_file(&file, "master-password").expect("open vault");
        let mut payload = opened.payload;
        assert!(
            payload
                .device
                .sync
                .dirty
                .contains(&format!("entry:{entry_id}")),
            "the dirty mark did not survive the save"
        );

        // The next session, against an in-memory provider.
        let fake = Arc::new(Fake::cas(true));
        let (keyfile, keys) = new_keyfile("correct horse").expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );
        let keys = Arc::new(keys);
        payload.device.sync.root_key = Some("unused-here".into());
        payload.device.sync.remote = Some(remote_identity(&config()));

        let now = super::now_ms();
        let report = pull(
            fake.as_ref(),
            &keys,
            PREFIX,
            "this-device",
            locals_from_payload(&payload, now),
            payload.device.sync.etags.clone(),
            now,
        )
        .await
        .expect("pull");
        apply_pull(&mut payload, &report, now);
        let (envelopes, etags) = take_dirty_envelopes(&mut payload);
        assert!(!envelopes.is_empty(), "the edit was not queued for push");
        let push_report = push(
            fake.as_ref(),
            &keys,
            PREFIX,
            "this-device",
            envelopes.clone(),
            etags,
        )
        .await;
        assert!(push_report.failed.is_empty(), "{:?}", push_report.failed);
        finish_push(&mut payload, &envelopes, &push_report);

        let object = object_key(PREFIX, &object_name(&keys, ENTRY_KIND, &entry_id));
        let stored = fake.object(&object).expect("the edit was pushed");
        let published = open_envelope(&keys, &stored).expect("open");
        assert_eq!(published.record["title"], "edited");
    }

    #[tokio::test]
    async fn join_writes_nothing_on_a_fresh_remote() {
        let dir = TempDir::new("join-fresh");
        let fake = Fake::cas(true);
        let joined = join_pull(
            &fake,
            PREFIX,
            "correct horse",
            "this-device",
            "identity",
            &SyncCredentialsArg::default(),
            NOW,
        )
        .await
        .expect("join");
        assert!(joined.is_none());
        assert!(dir.is_empty(), "a fresh remote wrote a local file");
        // The GET of the keyfile is the last thing the provider saw.
        assert_eq!(
            fake.last_call(),
            Some(format!("get:{}", keyfile_key(PREFIX)))
        );
    }

    #[tokio::test]
    async fn join_lands_records_and_writes_the_vault() {
        let dir = TempDir::new("join-lands");
        let fake = Fake::cas(true);
        let (keyfile, keys) = new_keyfile("correct horse").expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );
        let remote = entry_env("h-1", NOW - 1000, "dev-b", "over there");
        publish(&fake, &keys, &remote, "e1", Some(NOW - 1000));

        let joined = join_pull(
            &fake,
            PREFIX,
            "correct horse",
            "this-device",
            &remote_identity(&config()),
            &SyncCredentialsArg::default(),
            NOW,
        )
        .await
        .expect("join")
        .expect("keyfile present");

        let state = VaultState::default();
        install_new_vault(&state, &dir.0, "master-password", joined.payload).expect("install");
        let (file, _from_bak) = load_vault(&dir.0).expect("load vault");
        let opened = open_file(&file, "master-password").expect("open vault");
        assert!(
            opened.payload.entries.iter().any(|e| e.id == "h-1"),
            "the pulled record did not land"
        );
    }

    #[tokio::test]
    async fn join_seeds_missing_reserved_groups() {
        let fake = Fake::cas(true);
        let (keyfile, keys) = new_keyfile("correct horse").expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );
        let remote = entry_env("h-1", NOW - 1000, "dev-b", "over there");
        publish(&fake, &keys, &remote, "e1", Some(NOW - 1000));

        let joined = join_pull(
            &fake,
            PREFIX,
            "correct horse",
            "this-device",
            &remote_identity(&config()),
            &SyncCredentialsArg::default(),
            NOW,
        )
        .await
        .expect("join")
        .expect("keyfile present");

        for id in [ROOT_ID, TRASH_ID, BROWSER_ID] {
            assert!(
                joined.payload.groups.iter().any(|g| g.id == id),
                "missing reserved group {id}"
            );
            assert!(
                joined
                    .payload
                    .device
                    .sync
                    .dirty
                    .contains(&format!("group:{id}")),
                "the seeded group {id} is not dirty"
            );
        }
    }

    #[tokio::test]
    async fn a_wrong_passphrase_writes_nothing() {
        let dir = TempDir::new("join-wrong");
        let fake = Fake::cas(true);
        let (keyfile, _keys) = new_keyfile("correct horse").expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );

        let result = join_pull(
            &fake,
            PREFIX,
            "wrong passphrase",
            "this-device",
            "identity",
            &SyncCredentialsArg::default(),
            NOW,
        )
        .await;
        assert_eq!(
            result.err().expect("a wrong passphrase must fail"),
            "sync: wrong sync passphrase, or the keyfile is corrupt"
        );
        assert!(dir.is_empty(), "a refused join wrote a local file");
        assert!(
            fake.puts().is_empty(),
            "a refused join wrote a remote object"
        );
    }
}
