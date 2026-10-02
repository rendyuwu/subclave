//! The wire-facing shapes: what a pull or a push reports, what the webview hands
//! in, and what it gets back.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::modules::sync::model::Envelope;

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

// ---------------------------------------------------------------------------
// Arguments and answers
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
    /// S3's alone. `#[serde(default)]` so a WebDAV caller does not have to send
    /// an S3 field `provider_config` ignores. The S3 form still requires it
    /// (`connectionFieldsReady` in `src/modules/sync/types.ts`) and
    /// `provider_config` still puts it into the JSON `S3Config` deserializes.
    #[serde(default)]
    pub region: String,
    /// S3's alone, by the same default as the `region` field.
    #[serde(default)]
    pub bucket: String,
    pub prefix: String,
    /// A stored user toggle, and S3's alone; `false` is the safe default for a
    /// backend with no conditional write to turn on.
    #[serde(default)]
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
