//! Sealed records: what lives inside the ciphertext, and the payload
//! that wraps them.

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use super::device::DeviceState;

pub const ROOT_ID: &str = "root";
pub const TRASH_ID: &str = "trash";
pub const BROWSER_ID: &str = "browser";

/// serde camelCase for every struct is the on-disk and IPC contract; a rename
/// here changes the ciphertext.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub group_id: String,
    pub title: String,
    pub username: String,
    pub password: String,
    pub urls: Vec<EntryUrl>,
    pub notes: String,
    pub totp: Option<String>,
    pub custom_fields: Vec<CustomField>,
    pub tags: Vec<String>,
    pub icon: Option<String>,
    pub color: Option<EntryColor>,
    pub favorite: bool,
    pub expires_at: Option<u64>,
    pub trashed_from: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub history: Vec<EntryVersion>,
    pub last_used_at: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntryUrl {
    pub url: String,
    /// Serialized as `match` (a Rust keyword). Absent means [`MatchMode::Domain`].
    #[serde(rename = "match", default)]
    pub match_mode: MatchMode,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    #[default]
    Domain,
    Host,
    Exact,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomField {
    pub name: String,
    pub value: String,
    pub hidden: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntryVersion {
    pub updated_at: u64,
    pub reason: VersionReason,
    pub title: String,
    pub username: String,
    pub password: String,
    pub urls: Vec<EntryUrl>,
    pub notes: String,
    pub totp: Option<String>,
    pub custom_fields: Vec<CustomField>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VersionReason {
    Edit,
    Restore,
    Conflict,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub icon: Option<String>,
    pub color: Option<EntryColor>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tombstone {
    pub id: String,
    pub kind: TombstoneKind,
    pub deleted_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TombstoneKind {
    Entry,
    Group,
}

/// The decrypted payload. Device-local state rides in [`DeviceState`] with
/// `#[serde(default)]`, so files written before it landed open unchanged.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct VaultPayload {
    pub entries: Vec<Entry>,
    pub groups: Vec<Group>,
    pub tombstones: Vec<Tombstone>,
    #[serde(default)]
    pub device: DeviceState,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EntryColor {
    Red,
    Yellow,
    Green,
    Cyan,
    Blue,
    Magenta,
}

impl VaultPayload {
    /// Scrub every secret string. Entry metadata (title, username, urls) is
    /// not secret and is left to drop.
    pub fn wipe(&mut self) {
        for entry in &mut self.entries {
            entry.password.zeroize();
            entry.notes.zeroize();
            entry.totp.zeroize();
            for version in &mut entry.history {
                version.password.zeroize();
                version.notes.zeroize();
                version.totp.zeroize();
                for field in &mut version.custom_fields {
                    field.value.zeroize();
                }
            }
            for field in &mut entry.custom_fields {
                field.value.zeroize();
            }
        }
        // The sync credentials are secrets too: a locked process must not
        // keep the root key or the storage passwords in memory.
        self.device.sync.root_key.take();
        self.device.sync.s3_access_key_id.take();
        self.device.sync.s3_secret_access_key.take();
        self.device.sync.webdav_username.take();
        self.device.sync.webdav_password.take();
        self.device.sync.etags.clear();
        self.device.sync.dirty.clear();
    }
}

/// Add the reserved groups (`root`, `trash`, `browser`) that are absent, with
/// `browser` parented to `root`. Returns the ids it added, so a join can mark
/// exactly those dirty; the create path ignores the return.
pub(crate) fn seed_reserved_groups(payload: &mut VaultPayload, now: u64) -> Vec<String> {
    let reserved: [(&str, Option<&str>, &str); 3] = [
        (ROOT_ID, None, "Root"),
        (TRASH_ID, None, "Trash"),
        (BROWSER_ID, Some(ROOT_ID), "Browser"),
    ];
    let mut added = Vec::new();
    for (id, parent, name) in reserved {
        if payload.groups.iter().any(|g| g.id == id) {
            continue;
        }
        payload.groups.push(Group {
            id: id.into(),
            parent_id: parent.map(str::to_string),
            name: name.into(),
            icon: None,
            color: None,
            created_at: now,
            updated_at: now,
        });
        added.push(id.to_string());
    }
    added
}

#[cfg(test)]
pub(crate) fn entry() -> Entry {
    Entry {
        id: "e1".into(),
        group_id: ROOT_ID.into(),
        title: "Site".into(),
        username: "user".into(),
        password: "pw".into(),
        urls: vec![EntryUrl {
            url: "https://Example.COM/login".into(),
            match_mode: MatchMode::Domain,
        }],
        notes: "n".into(),
        totp: Some("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into()),
        custom_fields: vec![
            CustomField {
                name: "pin".into(),
                value: "1234".into(),
                hidden: true,
            },
            CustomField {
                name: "note".into(),
                value: "hi".into(),
                hidden: false,
            },
        ],
        tags: vec!["web".into()],
        icon: None,
        color: None,
        favorite: false,
        expires_at: None,
        trashed_from: None,
        created_at: 100,
        updated_at: 200,
        history: vec![],
        last_used_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::device::SyncDevice;
    use super::*;

    #[test]
    fn wipe_scrubs_every_secret_string() {
        let mut e = entry();
        e.history.push(EntryVersion {
            updated_at: 150,
            reason: VersionReason::Edit,
            title: "Old".into(),
            username: "user".into(),
            password: "old-pw".into(),
            urls: vec![],
            notes: "old-notes".into(),
            totp: None,
            custom_fields: vec![CustomField {
                name: "old-pin".into(),
                value: "5678".into(),
                hidden: true,
            }],
        });
        let mut payload = VaultPayload {
            entries: vec![e],
            groups: vec![],
            tombstones: vec![],
            device: DeviceState::default(),
        };
        payload.device.sync.root_key = Some("root-key".into());
        payload.device.sync.s3_access_key_id = Some("akid".into());
        payload.device.sync.s3_secret_access_key = Some("s3-secret".into());
        payload.device.sync.webdav_username = Some("dav-user".into());
        payload.device.sync.webdav_password = Some("dav-secret".into());
        payload
            .device
            .sync
            .etags
            .insert("entry:e1".into(), "etag".into());
        payload.device.sync.dirty.insert("entry:e1".into());
        payload.wipe();
        let e = &payload.entries[0];
        assert_eq!(e.password, "");
        assert_eq!(e.notes, "");
        assert_eq!(e.totp, None);
        assert_eq!(e.custom_fields[0].value, "");
        assert_eq!(e.history[0].password, "");
        assert_eq!(e.history[0].notes, "");
        assert_eq!(e.history[0].custom_fields[0].value, "");
        assert_eq!(
            payload.device.sync,
            SyncDevice::default(),
            "the sync credentials and maps are scrubbed"
        );
        assert_eq!(e.title, "Site", "metadata is not scrubbed");
    }
}
