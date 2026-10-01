//! Editor drafts and the helpers that normalize them before they land.

use serde::{de::Deserializer, Deserialize, Serialize};

use super::records::{EntryColor, EntryUrl};

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
pub struct GroupDraft {
    /// `None` = create; `Some(id)` = rename or reparent.
    pub id: Option<String>,
    pub parent_id: Option<String>,
    pub name: String,
    pub icon: Option<String>,
    pub color: Option<EntryColor>,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
