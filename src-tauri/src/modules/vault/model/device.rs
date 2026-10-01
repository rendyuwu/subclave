//! Device-local vault state and the sync bookkeeping it owns.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// This installation's device-local state. It rides the vault file (not the
/// sync objects), arrives with `#[serde(default)]`, and is never shared, so
/// the fields here never join a wire envelope.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DeviceState {
    #[serde(default)]
    pub sync: SyncDevice,
}

/// Sync state owned by this device: what remote it is joined to, the root key
/// that opens that remote, the credentials, and the bookkeeping maps the
/// engine needs to skip unchanged objects and re-push local edits.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncDevice {
    /// The remote this device is joined to, `"{provider}|{endpoint}|{bucket}|{prefix}"`.
    /// The stored root key is only valid for this identity; a config that
    /// names another remote drops both before a session opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// Base64, 32 bytes: the unwrapped root key. The passphrase is never stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_access_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_secret_access_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webdav_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webdav_password: Option<String>,
    /// `"kind:id"` to the etag the remote gave that object.
    #[serde(default)]
    pub etags: BTreeMap<String, String>,
    /// `"kind:id"` slots with local changes the remote has not seen.
    #[serde(default)]
    pub dirty: BTreeSet<String>,
}

impl SyncDevice {
    /// Record that `kind:id` has a local change the remote has not seen.
    pub fn mark_dirty(&mut self, kind: &str, id: &str) {
        self.dirty.insert(format!("{kind}:{id}"));
    }

    /// Forget the remote, the root key, the credentials and the tracking maps.
    pub fn clear(&mut self) {
        *self = SyncDevice::default();
    }
}
