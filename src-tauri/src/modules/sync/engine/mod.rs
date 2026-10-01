//! The two round trips, the apply path, and the commands.
//!
//! WHAT THIS MODULE OWNS. The pull and the push, and the sealing either side of
//! them. The object layout, the wire types, the landing of a pull into the vault
//! payload, the push bookkeeping, the in-process session and the five commands
//! the webview calls live in the submodules beside this file, and every public
//! name is re-exported here, so the paths into `engine` are the ones they always
//! were.
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
//! `no_new_sync_tauri_commands` test in `src-tauri/src/commands.rs` enforces the
//! async half of that.

mod commands;
mod layout;
mod payload;
mod types;

use std::collections::{BTreeMap, BTreeSet};

use super::crypto::{object_name, open_record, seal_record, SealedRecord, SyncKeys};
use super::model::{
    content_differs, known_kind, merge, strip_device_local, Envelope, WIRE_VERSION,
};
use super::provider::{ProviderError, SyncProvider};

use layout::{name_of, object_key, object_prefix, slot, TOMBSTONE_TTL_MS};

pub use commands::*;
pub use payload::{
    apply_pull, finish_push, local_envelope, locals_from_payload, take_dirty_envelopes,
    write_envelope,
};
pub use types::*;

// ---------------------------------------------------------------------------
// What a command answers before it is configured
// ---------------------------------------------------------------------------

/// What a command answers when no configuration has been opened.
///
/// Reachable on every launch, not only on a device that never configured:
/// [`SyncState`] starts empty and [`sync_configure`] is what fills it, so a
/// stored configuration is worth nothing until the caller has opened it again.
pub const NOT_CONFIGURED: &str = "sync: no sync configuration is open on this device";

// ---------------------------------------------------------------------------
// Sealing
// ---------------------------------------------------------------------------

pub(super) fn open_envelope(keys: &SyncKeys, bytes: &[u8]) -> Result<Envelope, String> {
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
pub(super) fn seal_envelope(keys: &SyncKeys, envelope: &Envelope) -> Result<Vec<u8>, String> {
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

#[cfg(test)]
mod test_support {
    use std::collections::{BTreeMap, BTreeSet};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};

    // The fake locks and immediately unlocks, so `lock()` returning the guard
    // directly is exactly what these call sites want.
    use parking_lot::Mutex;

    use serde_json::Value;

    use super::layout::object_key;
    use super::types::{Outcome, PullReport, SyncConfigArg};
    use super::{pull, seal_envelope};
    use crate::modules::sync::crypto::{new_keyfile, object_name, SyncKeys};
    use crate::modules::sync::model::{Envelope, ENTRY_KIND, WIRE_VERSION};
    use crate::modules::sync::provider::{
        Caps, Entry as ProviderEntry, Object, ProviderError, SyncProvider,
    };
    use crate::modules::vault::model::{DeviceState, Entry, Tombstone, VaultPayload, ROOT_ID};

    pub(crate) const DAY: u64 = 24 * 60 * 60 * 1000;
    /// "Now" for every test here, far enough from the epoch that a stamp a
    /// hundred days before it is still positive.
    pub(crate) const NOW: u64 = 1_800_000_000_000;
    pub(crate) const PREFIX: &str = "subclave";
    pub(crate) const STRONG: &str = "Zx9!pQ2#vLm7@Wr4Tk1$";

    /// A provider that keeps its objects in memory and counts what it was
    /// asked to do.
    ///
    /// The counts are the point rather than a convenience: half the properties
    /// below are "this object was NOT fetched" and "no put was issued", and
    /// neither is visible from a return value.
    pub(crate) type Stored = (Vec<u8>, String, Option<u64>);

    #[derive(Default)]
    pub(crate) struct Fake {
        pub(crate) objects: Mutex<BTreeMap<String, Stored>>,
        pub(crate) gets: Mutex<Vec<String>>,
        pub(crate) puts: Mutex<Vec<String>>,
        pub(crate) deletes: Mutex<Vec<String>>,
        pub(crate) calls: Mutex<Vec<String>>,
        pub(crate) cas: bool,
        /// Keys whose next conditional put is rejected once, to reach the retry.
        pub(crate) reject_once: Mutex<BTreeSet<String>>,
        /// Keys the next `get` reports as absent, to reach the window between
        /// the read that found nothing and the write that follows it.
        pub(crate) hidden_once: Mutex<BTreeSet<String>>,
        /// What a read-only bucket, an object lock or a lifecycle policy does
        /// to every delete this provider is asked for.
        pub(crate) refuse_deletes: AtomicBool,
    }

    impl Fake {
        pub(crate) fn cas(cas: bool) -> Self {
            Self {
                cas,
                ..Default::default()
            }
        }

        pub(crate) fn seed(&self, key: &str, bytes: Vec<u8>, etag: &str, modified_at: Option<u64>) {
            self.objects
                .lock()
                .insert(key.into(), (bytes, etag.into(), modified_at));
        }

        pub(crate) fn gets(&self) -> Vec<String> {
            self.gets.lock().clone()
        }
        pub(crate) fn puts(&self) -> Vec<String> {
            self.puts.lock().clone()
        }
        pub(crate) fn deletes(&self) -> Vec<String> {
            self.deletes.lock().clone()
        }
        pub(crate) fn last_call(&self) -> Option<String> {
            self.calls.lock().last().cloned()
        }
        pub(crate) fn object(&self, key: &str) -> Option<Vec<u8>> {
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
    pub(crate) struct TempDir(pub(crate) std::path::PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subclave-sync-engine-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id(),
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        pub(crate) fn is_empty(&self) -> bool {
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

    pub(crate) fn keys() -> SyncKeys {
        new_keyfile("correct horse").expect("new keyfile").1
    }

    pub(crate) fn env(
        kind: &str,
        id: &str,
        updated_at: u64,
        device: &str,
        record: Value,
    ) -> Envelope {
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

    pub(crate) fn test_entry(id: &str, title: &str, updated_at: u64) -> Entry {
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

    pub(crate) fn entry_env(id: &str, updated_at: u64, device: &str, title: &str) -> Envelope {
        let entry = test_entry(id, title, updated_at);
        env(
            ENTRY_KIND,
            id,
            updated_at,
            device,
            serde_json::to_value(&entry).expect("entry serialization"),
        )
    }

    pub(crate) fn grave(kind: &str, id: &str, deleted_at: u64, device: &str) -> Envelope {
        Envelope {
            deleted: true,
            record: Value::Null,
            ..env(kind, id, deleted_at, device, Value::Null)
        }
    }

    pub(crate) fn payload_of(entries: Vec<Entry>, tombstones: Vec<Tombstone>) -> VaultPayload {
        VaultPayload {
            entries,
            groups: vec![],
            tombstones,
            device: DeviceState::default(),
        }
    }

    /// Put one envelope on the fake as a real sealed object, at the key the
    /// engine will look for it under.
    pub(crate) fn publish(
        fake: &Fake,
        keys: &SyncKeys,
        envelope: &Envelope,
        etag: &str,
        modified: Option<u64>,
    ) {
        let key = object_key(PREFIX, &object_name(keys, &envelope.kind, &envelope.id));
        fake.seed(&key, seal_envelope(keys, envelope).unwrap(), etag, modified);
    }

    pub(crate) fn key_of(keys: &SyncKeys, envelope: &Envelope) -> String {
        object_key(PREFIX, &object_name(keys, &envelope.kind, &envelope.id))
    }

    pub(crate) fn outcome<'a>(report: &'a PullReport, id: &str) -> &'a Outcome {
        &report
            .records
            .iter()
            .find(|r| r.id == id)
            .unwrap_or_else(|| panic!("no disposition for {id}"))
            .outcome
    }

    pub(crate) async fn pull_with(
        fake: &Fake,
        keys: &SyncKeys,
        locals: Vec<Envelope>,
        etags: BTreeMap<String, String>,
    ) -> PullReport {
        pull(fake, keys, PREFIX, "this-device", locals, etags, NOW)
            .await
            .expect("pull")
    }

    pub(crate) fn config() -> SyncConfigArg {
        SyncConfigArg {
            provider: "s3".into(),
            endpoint: "https://storage.example".into(),
            region: "us-east-1".into(),
            bucket: "subclave".into(),
            prefix: PREFIX.into(),
            cas: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::sync::engine::test_support::*;
    use crate::modules::sync::model::ENTRY_KIND;
    use serde_json::json;
    use std::sync::atomic::Ordering;

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
}
