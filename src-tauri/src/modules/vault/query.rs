//! Read-only vault queries: the field resolver behind reveal and clipboard
//! copy, plus entry listing, search, detail and reveal.

use serde::Serialize;
use tauri::AppHandle;

use crate::modules::vault::events::run_blocking;
use crate::modules::vault::model::{
    detail_of, summary_of, Entry, EntryDetail, EntrySummary, Group, VaultPayload,
};
use crate::modules::vault::state::{VaultState, LOCKED_ERR};

/// Every ancestor chain step of `id`, for the cycle refusal on reparent.
pub(crate) fn is_descendant(payload: &VaultPayload, ancestor: &str, candidate: &str) -> bool {
    let mut current = candidate.to_string();
    for _ in 0..payload.groups.len() {
        let parent = payload
            .groups
            .iter()
            .find(|g| g.id == current)
            .and_then(|g| g.parent_id.clone());
        match parent {
            Some(p) if p == ancestor => return true,
            Some(p) => current = p,
            None => return false,
        }
    }
    false
}

/// Shared lookup behind reveal and clipboard copy. `totp_as_code` turns the
/// stored otpauth URI into the current code; everything else is verbatim.
pub(crate) fn resolve_field(
    entry: &Entry,
    field: &str,
    totp_as_code: bool,
) -> Result<String, String> {
    // history:<stamp>:password first: the prefix parse must not be shadowed
    // by the custom: arm below.
    if let Some(rest) = field.strip_prefix("history:") {
        let mut parts = rest.splitn(2, ':');
        let stamp: u64 = parts
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| "vault: no such version".to_string())?;
        return match parts.next() {
            Some("password") => entry
                .history
                .iter()
                .find(|v| v.updated_at == stamp)
                .map(|v| v.password.clone())
                .ok_or_else(|| "vault: no such version".to_string()),
            _ => Err("vault: unknown field".to_string()),
        };
    }
    match field {
        "password" => Ok(entry.password.clone()),
        "username" => Ok(entry.username.clone()),
        "totp" => {
            let uri = entry
                .totp
                .clone()
                .ok_or_else(|| "vault: no TOTP on this entry".to_string())?;
            if totp_as_code {
                let parsed = crate::modules::totp::parse(&uri)?;
                Ok(crate::modules::totp::code(
                    &parsed,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                ))
            } else {
                Ok(uri)
            }
        }
        other => other.strip_prefix("custom:").map_or_else(
            || Err("vault: unknown field".to_string()),
            |name| {
                entry
                    .custom_fields
                    .iter()
                    .find(|f| f.name.eq_ignore_ascii_case(name))
                    .map(|f| f.value.clone())
                    .ok_or_else(|| "vault: no such custom field".to_string())
            },
        ),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultList {
    pub entries: Vec<EntrySummary>,
    pub groups: Vec<Group>,
}

#[tauri::command]
pub async fn vault_list(app: AppHandle) -> Result<VaultList, String> {
    run_blocking(&app, vault_list_inner).await
}

pub(crate) fn vault_list_inner(state: &VaultState) -> Result<VaultList, String> {
    let guard = state.access()?;
    let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
    // Unsorted: the locale compare is a UI concern.
    Ok(VaultList {
        entries: unlocked.payload.entries.iter().map(summary_of).collect(),
        groups: unlocked.payload.groups.clone(),
    })
}

/// Ids of the entries whose title, username, notes, any tag or any URL
/// contains `query` case-insensitively. Never the password. Trashed entries
/// are included; the UI applies its own scope. An empty or whitespace-only
/// query returns an empty list.
#[tauri::command]
pub async fn vault_search(app: AppHandle, query: String) -> Result<Vec<String>, String> {
    run_blocking(&app, move |state| vault_search_inner(state, &query)).await
}

pub(crate) fn vault_search_inner(state: &VaultState, query: &str) -> Result<Vec<String>, String> {
    let needle = query.trim().to_lowercase();
    let guard = state.access()?;
    let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
    if needle.is_empty() {
        return Ok(Vec::new());
    }
    Ok(unlocked
        .payload
        .entries
        .iter()
        .filter(|e| {
            e.title.to_lowercase().contains(&needle)
                || e.username.to_lowercase().contains(&needle)
                || e.notes.to_lowercase().contains(&needle)
                || e.tags.iter().any(|t| t.to_lowercase().contains(&needle))
                || e.urls
                    .iter()
                    .any(|u| u.url.to_lowercase().contains(&needle))
        })
        .map(|e| e.id.clone())
        .collect())
}

#[tauri::command]
pub async fn vault_entry_get(app: AppHandle, id: String) -> Result<EntryDetail, String> {
    run_blocking(&app, move |state| vault_entry_get_inner(state, &id)).await
}

pub(crate) fn vault_entry_get_inner(state: &VaultState, id: &str) -> Result<EntryDetail, String> {
    let guard = state.access()?;
    let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
    let entry = unlocked
        .payload
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| "vault: no such entry".to_string())?;
    Ok(detail_of(entry))
}

#[tauri::command]
pub async fn vault_entry_reveal(
    app: AppHandle,
    id: String,
    field: String,
) -> Result<String, String> {
    run_blocking(&app, move |state| {
        vault_entry_reveal_inner(state, &id, &field)
    })
    .await
}

pub(crate) fn vault_entry_reveal_inner(
    state: &VaultState,
    id: &str,
    field: &str,
) -> Result<String, String> {
    let guard = state.access()?;
    let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
    let entry = unlocked
        .payload
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| "vault: no such entry".to_string())?;
    resolve_field(entry, field, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::modules::vault::entry_commands::*;
    use crate::modules::vault::model::{self, EntryVersion, ROOT_ID};
    use crate::modules::vault::session::*;
    use crate::modules::vault::test_util::*;

    #[test]
    fn resolve_field_cases() {
        let entry = Entry {
            id: "e".into(),
            group_id: ROOT_ID.into(),
            title: "T".into(),
            username: "u".into(),
            password: "pw".into(),
            urls: vec![],
            notes: String::new(),
            totp: Some("otpauth://totp/T?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ".into()),
            custom_fields: vec![model::CustomField {
                name: "API Key".into(),
                value: "abc123".into(),
                hidden: false,
            }],
            tags: vec![],
            icon: None,
            color: None,
            favorite: false,
            expires_at: None,
            trashed_from: None,
            created_at: 1,
            updated_at: 1,
            history: vec![EntryVersion {
                updated_at: 42,
                reason: model::VersionReason::Edit,
                title: "T".into(),
                username: "u".into(),
                password: "old".into(),
                urls: vec![],
                notes: String::new(),
                totp: None,
                custom_fields: vec![],
            }],
            last_used_at: None,
        };
        assert_eq!(resolve_field(&entry, "password", false).unwrap(), "pw");
        assert_eq!(resolve_field(&entry, "username", false).unwrap(), "u");
        // totp as code: six digits, from the current unix second. The
        // exact value is pinned by the RFC vector tests; here only the
        // path is proven.
        let as_code = resolve_field(&entry, "totp", true).unwrap();
        assert_eq!(as_code.len(), 6);
        assert!(as_code.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(
            resolve_field(&entry, "totp", false).unwrap(),
            entry.totp.clone().unwrap()
        );
        assert_eq!(
            resolve_field(&entry, "custom:api KEY", false).unwrap(),
            "abc123"
        );
        assert_eq!(
            resolve_field(&entry, "history:42:password", false).unwrap(),
            "old"
        );
        assert_eq!(
            resolve_field(&entry, "history:999:password", false).unwrap_err(),
            "vault: no such version"
        );
        assert_eq!(
            resolve_field(&entry, "history:42:notes", false).unwrap_err(),
            "vault: unknown field"
        );
        assert_eq!(
            resolve_field(&entry, "custom:missing", false).unwrap_err(),
            "vault: no such custom field"
        );
    }

    /// Search matches title, username, notes, tags and URLs, never the
    /// password, and includes trashed entries.
    #[test]
    fn search_matches_every_field_but_the_password() {
        let dir = TempDir::new("search");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();

        let mut github = draft(None, "GitHub");
        github.username = "octocat".into();
        github.password = Some("SECRET-NEEDLE".into());
        github.notes = "work account".into();
        github.tags = vec!["dev".into()];
        github.urls = vec![model::EntryUrl {
            url: "https://github.com/login".into(),
            match_mode: model::MatchMode::Domain,
        }];
        let github = vault_entry_upsert_inner(&state, &dir.0, github).unwrap();

        let mut other = draft(None, "Mail");
        other.notes = "personal".into();
        let other = vault_entry_upsert_inner(&state, &dir.0, other).unwrap();

        let hits = |q: &str| vault_search_inner(&state, q).unwrap();

        assert_eq!(hits("github"), vec![github.id.clone()]);
        assert_eq!(hits("OCTO"), vec![github.id.clone()]);
        assert_eq!(hits("work account"), vec![github.id.clone()]);
        assert_eq!(hits("dev"), vec![github.id.clone()]);
        assert_eq!(hits("login"), vec![github.id.clone()]);
        assert!(hits("SECRET-NEEDLE").is_empty(), "never the password");
        assert!(hits("zzz").is_empty());
        assert!(hits("   ").is_empty(), "a blank query matches nothing");

        // Trashed entries are included; the UI applies its own scope.
        vault_entry_trash_inner(&state, &dir.0, vec![other.id.clone()]).unwrap();
        assert_eq!(hits("personal"), vec![other.id.clone()]);
    }
}
