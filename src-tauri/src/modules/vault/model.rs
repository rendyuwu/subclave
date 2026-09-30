//! The vault data model and its webview-facing projections.
//!
//! The sealed records ([`Entry`], [`Group`], [`Tombstone`]) live only inside
//! the ciphertext; the projections ([`EntrySummary`], [`EntryDetail`],
//! [`EntryDraft`]) are what crosses the IPC boundary. Timestamps are Unix
//! milliseconds everywhere; the LWW stamp rule is [`stamp_next`].

use serde::{de::Deserializer, Deserialize, Serialize};
use url::Url;
use zeroize::Zeroize;

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
pub struct GroupDraft {
    /// `None` = create; `Some(id)` = rename or reparent.
    pub id: Option<String>,
    pub parent_id: Option<String>,
    pub name: String,
    pub icon: Option<String>,
    pub color: Option<EntryColor>,
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

/// The decrypted payload. Device-local state (sync credentials, browser
/// pairings) is not carried here yet; when it lands it arrives with
/// `#[serde(default)]`, so the format v1 files written today open unchanged.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct VaultPayload {
    pub entries: Vec<Entry>,
    pub groups: Vec<Group>,
    pub tombstones: Vec<Tombstone>,
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

// ---- Webview-facing projections ----

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: String,
    pub group_id: String,
    pub title: String,
    pub username: String,
    pub primary_host: Option<String>,
    pub tags: Vec<String>,
    pub icon: Option<String>,
    pub color: Option<EntryColor>,
    pub favorite: bool,
    pub has_password: bool,
    pub has_totp: bool,
    pub expires_at: Option<u64>,
    pub updated_at: u64,
    pub last_used_at: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntryDetail {
    pub id: String,
    pub group_id: String,
    pub title: String,
    pub username: String,
    pub primary_host: Option<String>,
    pub tags: Vec<String>,
    pub icon: Option<String>,
    pub color: Option<EntryColor>,
    pub favorite: bool,
    pub has_password: bool,
    pub has_totp: bool,
    pub expires_at: Option<u64>,
    pub updated_at: u64,
    pub last_used_at: Option<u64>,
    pub urls: Vec<EntryUrl>,
    pub notes: String,
    /// Hidden values are `None`: the detail view never carries them.
    pub custom_fields: Vec<DetailCustomField>,
    pub history: Vec<DetailVersion>,
    pub created_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetailCustomField {
    pub name: String,
    pub hidden: bool,
    pub value: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DetailVersion {
    pub updated_at: u64,
    pub reason: VersionReason,
    pub changed: Vec<String>,
}

/// Editor save. An omitted secret means "stored, unchanged".
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntryDraft {
    pub id: Option<String>,
    pub group_id: String,
    pub title: String,
    pub username: String,
    /// Absent or null keeps the stored password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub urls: Vec<EntryUrl>,
    pub notes: String,
    /// `None` = unchanged, `Some(None)` = clear, `Some(Some(uri))` = set.
    /// A plain derive maps JSON `null` to the outer `None`, which would make
    /// "null clears" unreachable, hence [`double_option`].
    #[serde(default, deserialize_with = "double_option")]
    pub totp: Option<Option<String>>,
    pub custom_fields: Vec<DraftCustomField>,
    pub tags: Vec<String>,
    pub icon: Option<String>,
    pub color: Option<EntryColor>,
    pub favorite: bool,
    pub expires_at: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DraftCustomField {
    pub name: String,
    pub hidden: bool,
    /// Absent or null keeps the stored value of an existing same-name field.
    #[serde(default, deserialize_with = "double_option")]
    pub value: Option<Option<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorOptions {
    pub length: u32,
    pub lower: bool,
    pub upper: bool,
    pub digits: bool,
    pub symbols: bool,
    pub exclude_ambiguous: bool,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            length: 20,
            lower: true,
            upper: true,
            digits: true,
            symbols: true,
            exclude_ambiguous: false,
        }
    }
}

/// Deserialize `Option<Option<T>>`: JSON `null` becomes `Some(None)`, an
/// omitted field stays `None` via the `#[serde(default)]` beside it.
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

/// The stamp rule behind `updatedAt = max(now, previous.updatedAt + 1)`: a
/// local edit always lands strictly after the version it replaces, even when
/// the wall clock is behind the one that stamped that version.
pub fn stamp_next(now: u64, previous_updated_at: u64) -> u64 {
    now.max(previous_updated_at.saturating_add(1))
}

/// Trim, drop empties, dedupe case-insensitively keeping the first spelling.
/// Case folding is ASCII, matching the identifier comparisons elsewhere in
/// the vault. No length or count cap.
pub fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for raw in tags {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(trimmed.to_string());
        }
    }
    out
}

/// Host of a URL, lowercased. Full public-suffix matching belongs to the
/// browser-integration matcher; this is only the summary's `primaryHost`.
pub fn host_of(url: &str) -> Option<String> {
    Url::parse(url).ok()?.host_str().map(|h| h.to_lowercase())
}

pub fn summary_of(entry: &Entry) -> EntrySummary {
    EntrySummary {
        id: entry.id.clone(),
        group_id: entry.group_id.clone(),
        title: entry.title.clone(),
        username: entry.username.clone(),
        primary_host: entry.urls.first().and_then(|u| host_of(&u.url)),
        tags: entry.tags.clone(),
        icon: entry.icon.clone(),
        color: entry.color.clone(),
        favorite: entry.favorite,
        has_password: !entry.password.is_empty(),
        has_totp: entry.totp.is_some(),
        expires_at: entry.expires_at,
        updated_at: entry.updated_at,
        last_used_at: entry.last_used_at,
    }
}

/// Content fields a history version carries. Order is the report order too:
/// [`detail_of`] reports changed names sorted by this list's position.
const VERSION_FIELDS: [&str; 7] = [
    "title",
    "username",
    "password",
    "urls",
    "notes",
    "totp",
    "customFields",
];

/// The content snapshot of one entry, for a history version or for the
/// current state the newest version compares against.
pub(crate) fn version_of(e: &Entry, reason: VersionReason) -> EntryVersion {
    EntryVersion {
        updated_at: e.updated_at,
        reason,
        title: e.title.clone(),
        username: e.username.clone(),
        password: e.password.clone(),
        urls: e.urls.clone(),
        notes: e.notes.clone(),
        totp: e.totp.clone(),
        custom_fields: e.custom_fields.clone(),
    }
}

pub(crate) fn version_changed_names(
    current: &EntryVersion,
    previous: &EntryVersion,
) -> Vec<String> {
    let differs = [
        current.title != previous.title,
        current.username != previous.username,
        current.password != previous.password,
        current.urls != previous.urls,
        current.notes != previous.notes,
        current.totp != previous.totp,
        current.custom_fields != previous.custom_fields,
    ];
    VERSION_FIELDS
        .iter()
        .zip(differs)
        .filter(|(_, d)| *d)
        .map(|(name, _)| name.to_string())
        .collect()
}

pub fn detail_of(entry: &Entry) -> EntryDetail {
    // history is newest first; the comparison target of the newest version is
    // the current entry, every older version compares against its newer
    // neighbour.
    let mut newer = version_of(entry, VersionReason::Edit);
    let history = entry
        .history
        .iter()
        .map(|version| {
            let changed = version_changed_names(&newer, version);
            newer = version.clone();
            DetailVersion {
                updated_at: version.updated_at,
                reason: version.reason.clone(),
                changed,
            }
        })
        .collect();
    EntryDetail {
        id: entry.id.clone(),
        group_id: entry.group_id.clone(),
        title: entry.title.clone(),
        username: entry.username.clone(),
        primary_host: entry.urls.first().and_then(|u| host_of(&u.url)),
        tags: entry.tags.clone(),
        icon: entry.icon.clone(),
        color: entry.color.clone(),
        favorite: entry.favorite,
        has_password: !entry.password.is_empty(),
        has_totp: entry.totp.is_some(),
        expires_at: entry.expires_at,
        updated_at: entry.updated_at,
        last_used_at: entry.last_used_at,
        urls: entry.urls.clone(),
        notes: entry.notes.clone(),
        custom_fields: entry
            .custom_fields
            .iter()
            .map(|f| DetailCustomField {
                name: f.name.clone(),
                hidden: f.hidden,
                value: if f.hidden {
                    None
                } else {
                    Some(f.value.clone())
                },
            })
            .collect(),
        history,
        created_at: entry.created_at,
    }
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Entry {
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

    #[test]
    fn stamp_next_is_monotonic_past_a_equal_stamp() {
        assert_eq!(stamp_next(500, 400), 500);
        assert_eq!(stamp_next(500, 500), 501);
        assert_eq!(stamp_next(100, 900), 901);
    }

    #[test]
    fn normalize_tags_trims_dedupes_and_drops_empties() {
        assert_eq!(
            normalize_tags(vec![
                " Web ".into(),
                "web".into(),
                "".into(),
                "  ".into(),
                "WEB".into(),
                "ssh".into()
            ]),
            vec!["Web".to_string(), "ssh".to_string()]
        );
    }

    #[test]
    fn host_of_lowercases_the_host() {
        assert_eq!(
            host_of("https://Example.COM:8443/login").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_of("not a url"), None);
    }

    #[test]
    fn summary_reduces_secrets_to_flags() {
        let summary = summary_of(&entry());
        assert!(summary.has_password);
        assert!(summary.has_totp);
        assert_eq!(summary.primary_host.as_deref(), Some("example.com"));
        assert_eq!(summary.updated_at, 200);
    }

    #[test]
    fn detail_hides_hidden_values_and_reports_changed_fields() {
        let mut e = entry();
        e.history.push(EntryVersion {
            updated_at: 150,
            reason: VersionReason::Edit,
            title: "Old".into(),
            username: "user".into(),
            password: "oldpw".into(),
            urls: e.urls.clone(),
            notes: "n".into(),
            totp: None,
            custom_fields: vec![],
        });
        let detail = detail_of(&e);
        assert_eq!(detail.custom_fields[0].value, None);
        assert_eq!(detail.custom_fields[1].value, Some("hi".into()));
        // Newest version first; its comparison target is the current entry,
        // so every content field it differs in is reported.
        assert_eq!(
            detail.history[0].changed,
            vec!["title", "password", "totp", "customFields"]
        );
    }

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
        };
        payload.wipe();
        let e = &payload.entries[0];
        assert_eq!(e.password, "");
        assert_eq!(e.notes, "");
        assert_eq!(e.totp, None);
        assert_eq!(e.custom_fields[0].value, "");
        assert_eq!(e.history[0].password, "");
        assert_eq!(e.history[0].notes, "");
        assert_eq!(e.history[0].custom_fields[0].value, "");
        assert_eq!(e.title, "Site", "metadata is not scrubbed");
    }
}
