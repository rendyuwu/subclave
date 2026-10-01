//! Webview-facing projections of the sealed records.

use serde::{Deserialize, Serialize};
use url::Url;

use super::records::{Entry, EntryColor, EntryUrl, EntryVersion, VersionReason};

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

#[cfg(test)]
mod tests {
    use super::super::records::entry;
    use super::*;

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
}
