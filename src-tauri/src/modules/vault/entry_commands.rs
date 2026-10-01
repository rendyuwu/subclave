//! Entry mutation commands: upsert, move, trash, restore, permanent delete and
//! history restore.

use std::path::Path;

use tauri::AppHandle;

use crate::modules::vault::events::{emit_changed, run_blocking};
use crate::modules::vault::model;
use crate::modules::vault::model::{
    normalize_tags, stamp_next, summary_of, version_changed_names, version_of, Entry, EntryDraft,
    EntrySummary, EntryVersion, Tombstone, TombstoneKind, ROOT_ID, TRASH_ID,
};
use crate::modules::vault::session::vault_dir;
use crate::modules::vault::state::{commit, now_ms, VaultState, LOCKED_ERR};

#[tauri::command]
pub async fn vault_entry_upsert(app: AppHandle, draft: EntryDraft) -> Result<EntrySummary, String> {
    let dir = vault_dir(&app)?;
    let result = run_blocking(&app, move |state| {
        vault_entry_upsert_inner(state, &dir, draft)
    })
    .await;
    if let Ok(summary) = &result {
        emit_changed(&app, std::slice::from_ref(&summary.id), "local");
    }
    result
}

pub(crate) fn vault_entry_upsert_inner(
    state: &VaultState,
    dir: &Path,
    draft: EntryDraft,
) -> Result<EntrySummary, String> {
    state.ensure_writable()?;
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let payload = &mut unlocked.payload;
    let now = now_ms();
    let tags = normalize_tags(draft.tags.clone());

    // Custom field names are unique case-insensitively on both paths.
    {
        let mut seen = std::collections::HashSet::new();
        for f in &draft.custom_fields {
            if !seen.insert(f.name.to_ascii_lowercase()) {
                return Err("vault: duplicate custom field name".to_string());
            }
        }
    }

    // An ABSENT totp means unchanged on the update path (the detail view
    // carries no URI, so the editor cannot send it back); only an explicit
    // null clears. A set URI is validated through the parser, whose message
    // the editor shows verbatim.
    let update_totp = match (&draft.id, &draft.totp) {
        (_, Some(Some(uri))) => Some(Some(crate::modules::totp::parse(uri).map(|_| uri.clone())?)),
        (Some(_), None) => None, // resolved to the stored URI below
        (_, Some(None)) => Some(None),
        (None, None) => Some(None),
    };

    let id = match &draft.id {
        None => {
            // Create.
            if draft.group_id != ROOT_ID && !payload.groups.iter().any(|g| g.id == draft.group_id) {
                return Err("vault: no such group".to_string());
            }
            // A create into Trash would have no `trashed_from` to restore to,
            // so entry creation and trashing stay separate paths.
            if draft.group_id == TRASH_ID {
                return Err("vault: use trash instead".to_string());
            }
            let totp = match &update_totp {
                Some(totp) => totp.clone(),
                None => unreachable!("create always resolves totp to Some"),
            };
            let id = uuid::Uuid::new_v4().to_string();
            payload.entries.push(Entry {
                id: id.clone(),
                group_id: draft.group_id.clone(),
                title: draft.title.clone(),
                username: draft.username.clone(),
                password: draft.password.clone().unwrap_or_default(),
                urls: draft.urls.clone(),
                notes: draft.notes.clone(),
                totp,
                custom_fields: draft
                    .custom_fields
                    .iter()
                    .map(|f| {
                        Ok(model::CustomField {
                            name: f.name.clone(),
                            // A brand-new field must carry a value.
                            value: f.value.clone().flatten().ok_or_else(|| {
                                format!("vault: custom field {} has no value", f.name)
                            })?,
                            hidden: f.hidden,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                tags,
                icon: draft.icon.clone(),
                color: draft.color.clone(),
                favorite: draft.favorite,
                expires_at: draft.expires_at,
                trashed_from: None,
                created_at: now,
                updated_at: now,
                history: vec![],
                last_used_at: None,
            });
            id
        }
        Some(id) => {
            let entry = payload
                .entries
                .iter_mut()
                .find(|e| &e.id == id)
                .ok_or_else(|| "vault: no such entry".to_string())?;
            if draft.group_id != entry.group_id
                && draft.group_id != ROOT_ID
                && !payload.groups.iter().any(|g| g.id == draft.group_id)
            {
                return Err("vault: no such group".to_string());
            }
            // A draft must not move entries into or out of Trash: those are
            // vault_entry_trash and vault_entry_restore, which maintain
            // trashed_from.
            if draft.group_id == TRASH_ID || entry.group_id == TRASH_ID {
                return Err("vault: use trash instead".to_string());
            }
            // Absent password keeps the stored one; absent OR null custom
            // field value keeps the stored value of an existing same-name
            // field (EntryDetail hands hidden values out as null, so a
            // round-tripped editor draft keeps them); a field not on the
            // entry must carry a value.
            let new_password = draft
                .password
                .clone()
                .unwrap_or_else(|| entry.password.clone());
            let new_totp = match &update_totp {
                Some(totp) => totp.clone(),
                None => entry.totp.clone(),
            };
            let new_custom: Vec<model::CustomField> = draft
                .custom_fields
                .iter()
                .map(|f| {
                    let stored = entry
                        .custom_fields
                        .iter()
                        .find(|stored| stored.name.eq_ignore_ascii_case(&f.name));
                    let value = match (&f.value, stored) {
                        (Some(Some(v)), _) => v.clone(),
                        (_, Some(stored)) => stored.value.clone(),
                        (_, None) => {
                            return Err(format!("vault: custom field {} has no value", f.name))
                        }
                    };
                    Ok(model::CustomField {
                        name: f.name.clone(),
                        value,
                        hidden: f.hidden,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;

            let next = EntryVersion {
                updated_at: stamp_next(now, entry.updated_at),
                reason: model::VersionReason::Edit,
                title: draft.title.clone(),
                username: draft.username.clone(),
                password: new_password,
                urls: draft.urls.clone(),
                notes: draft.notes.clone(),
                totp: new_totp,
                custom_fields: new_custom,
            };
            // Tags, icon, colour, favourite, expiry and group are not content:
            // they add no history version.
            let old = version_of(entry, model::VersionReason::Edit);
            if !version_changed_names(&next, &old).is_empty() {
                entry.history.insert(0, old);
                entry.history.truncate(10);
            }
            entry.title = next.title;
            entry.username = next.username;
            entry.password = next.password;
            entry.urls = next.urls;
            entry.notes = next.notes;
            entry.totp = next.totp;
            entry.custom_fields = next.custom_fields;
            entry.tags = tags;
            entry.icon = draft.icon.clone();
            entry.color = draft.color.clone();
            entry.favorite = draft.favorite;
            entry.expires_at = draft.expires_at;
            entry.group_id = draft.group_id.clone();
            entry.updated_at = next.updated_at;
            id.clone()
        }
    };
    payload.device.sync.mark_dirty("entry", &id);
    drop(guard);
    commit(state, dir)?;
    let guard = state.access()?;
    let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
    let entry = unlocked
        .payload
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| "vault: no such entry".to_string())?;
    Ok(summary_of(entry))
}

#[tauri::command]
pub async fn vault_entry_move(
    app: AppHandle,
    ids: Vec<String>,
    group_id: String,
) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let result = run_blocking(&app, move |state| {
        vault_entry_move_inner(state, &dir, ids, group_id)
    })
    .await;
    if result.is_ok() {
        emit_changed(&app, &[], "local");
    }
    result
}

pub(crate) fn vault_entry_move_inner(
    state: &VaultState,
    dir: &Path,
    ids: Vec<String>,
    group_id: String,
) -> Result<(), String> {
    state.ensure_writable()?;
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let payload = &mut unlocked.payload;
    if group_id != ROOT_ID && !payload.groups.iter().any(|g| g.id == group_id) {
        return Err("vault: no such group".to_string());
    }
    if group_id == TRASH_ID {
        return Err("vault: use trash instead".to_string());
    }
    // Validate every id BEFORE mutating, so a refusal leaves no half-applied
    // edits behind in memory.
    for id in &ids {
        let entry = payload
            .entries
            .iter()
            .find(|e| &e.id == id)
            .ok_or_else(|| "vault: no such entry".to_string())?;
        if entry.group_id == TRASH_ID {
            return Err("vault: restore from Trash first".to_string());
        }
    }
    let now = now_ms();
    for id in &ids {
        let entry = payload
            .entries
            .iter_mut()
            .find(|e| &e.id == id)
            .expect("validated above");
        entry.group_id = group_id.clone();
        entry.updated_at = stamp_next(now, entry.updated_at);
        payload.device.sync.mark_dirty("entry", id);
    }
    drop(guard);
    commit(state, dir)
}

#[tauri::command]
pub async fn vault_entry_trash(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let result = run_blocking(&app, move |state| vault_entry_trash_inner(state, &dir, ids)).await;
    if result.is_ok() {
        emit_changed(&app, &[], "local");
    }
    result
}

pub(crate) fn vault_entry_trash_inner(
    state: &VaultState,
    dir: &Path,
    ids: Vec<String>,
) -> Result<(), String> {
    state.ensure_writable()?;
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let payload = &mut unlocked.payload;
    // Validate every id BEFORE mutating.
    for id in &ids {
        payload
            .entries
            .iter()
            .find(|e| &e.id == id)
            .ok_or_else(|| "vault: no such entry".to_string())?;
    }
    let now = now_ms();
    for id in &ids {
        let entry = payload
            .entries
            .iter_mut()
            .find(|e| &e.id == id)
            .expect("validated above");
        if entry.group_id == TRASH_ID {
            continue;
        }
        entry.trashed_from = Some(entry.group_id.clone());
        entry.group_id = TRASH_ID.into();
        entry.updated_at = stamp_next(now, entry.updated_at);
        payload.device.sync.mark_dirty("entry", id);
    }
    drop(guard);
    commit(state, dir)
}

#[tauri::command]
pub async fn vault_entry_restore(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let result = run_blocking(&app, move |state| {
        vault_entry_restore_inner(state, &dir, ids)
    })
    .await;
    if result.is_ok() {
        emit_changed(&app, &[], "local");
    }
    result
}

pub(crate) fn vault_entry_restore_inner(
    state: &VaultState,
    dir: &Path,
    ids: Vec<String>,
) -> Result<(), String> {
    state.ensure_writable()?;
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let payload = &mut unlocked.payload;
    // Validate every id BEFORE mutating; only entries in Trash restore.
    for id in &ids {
        let entry = payload
            .entries
            .iter()
            .find(|e| &e.id == id)
            .ok_or_else(|| "vault: no such entry".to_string())?;
        if entry.group_id != TRASH_ID {
            return Err("vault: only entries in Trash can be restored".to_string());
        }
    }
    let now = now_ms();
    for id in &ids {
        let entry = payload
            .entries
            .iter_mut()
            .find(|e| &e.id == id)
            .expect("validated above");
        // Back to trashed_from when that group still exists, else root.
        let target = entry
            .trashed_from
            .clone()
            .filter(|g| payload.groups.iter().any(|grp| &grp.id == g))
            .unwrap_or_else(|| ROOT_ID.to_string());
        entry.trashed_from = None;
        entry.group_id = target;
        entry.updated_at = stamp_next(now, entry.updated_at);
        payload.device.sync.mark_dirty("entry", id);
    }
    drop(guard);
    commit(state, dir)
}

#[tauri::command]
pub async fn vault_entry_delete(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let result = run_blocking(&app, move |state| {
        vault_entry_delete_inner(state, &dir, ids)
    })
    .await;
    if result.is_ok() {
        emit_changed(&app, &[], "local");
    }
    result
}

pub(crate) fn vault_entry_delete_inner(
    state: &VaultState,
    dir: &Path,
    ids: Vec<String>,
) -> Result<(), String> {
    state.ensure_writable()?;
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let payload = &mut unlocked.payload;
    let now = now_ms();
    for id in &ids {
        let entry = payload
            .entries
            .iter()
            .find(|e| &e.id == id)
            .ok_or_else(|| "vault: no such entry".to_string())?;
        if entry.group_id != TRASH_ID {
            return Err("vault: only entries in Trash can be deleted permanently".to_string());
        }
    }
    for id in &ids {
        payload.entries.retain(|e| &e.id != id);
        payload.device.sync.mark_dirty("entry", id);
        payload.tombstones.push(Tombstone {
            id: id.clone(),
            kind: TombstoneKind::Entry,
            deleted_at: now,
        });
    }
    drop(guard);
    commit(state, dir)
}

#[tauri::command]
pub async fn vault_entry_restore_version(
    app: AppHandle,
    id: String,
    updated_at: u64,
) -> Result<EntrySummary, String> {
    let dir = vault_dir(&app)?;
    let emit_id = id.clone();
    let result = run_blocking(&app, move |state| {
        vault_entry_restore_version_inner(state, &dir, &id, updated_at)
    })
    .await;
    if result.is_ok() {
        emit_changed(&app, std::slice::from_ref(&emit_id), "local");
    }
    result
}

pub(crate) fn vault_entry_restore_version_inner(
    state: &VaultState,
    dir: &Path,
    id: &str,
    updated_at: u64,
) -> Result<EntrySummary, String> {
    state.ensure_writable()?;
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let payload = &mut unlocked.payload;
    let now = now_ms();
    let entry = payload
        .entries
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| "vault: no such entry".to_string())?;
    let index = entry
        .history
        .iter()
        .position(|v| v.updated_at == updated_at)
        .ok_or_else(|| "vault: no such version".to_string())?;
    let version = entry.history.remove(index);
    // The replaced current state goes back to history as a restore, keeping
    // its own stamp.
    let replaced = version_of(entry, model::VersionReason::Restore);
    entry.title = version.title;
    entry.username = version.username;
    entry.password = version.password;
    entry.urls = version.urls;
    entry.notes = version.notes;
    entry.totp = version.totp;
    entry.custom_fields = version.custom_fields;
    entry.updated_at = stamp_next(now, entry.updated_at);
    entry.history.insert(0, replaced);
    entry.history.truncate(10);
    let summary = summary_of(entry);
    payload.device.sync.mark_dirty("entry", id);
    drop(guard);
    commit(state, dir)?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::Ordering;

    use crate::modules::vault::group_commands::*;
    use crate::modules::vault::model::BROWSER_ID;
    use crate::modules::vault::query::*;
    use crate::modules::vault::session::*;
    use crate::modules::vault::test_util::*;

    /// A local edit rides the same save as the dirty mark, so the sync engine
    /// can find it after a restart.
    #[test]
    fn an_edit_marks_the_slot_dirty() {
        let dir = TempDir::new("dirty");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let summary = vault_entry_upsert_inner(&state, &dir.0, draft(None, "Site")).unwrap();
        let guard = state.access().unwrap();
        let payload = &guard.as_ref().unwrap().payload;
        assert!(payload
            .device
            .sync
            .dirty
            .contains(&format!("entry:{}", summary.id)));
    }

    /// Restore-version semantics and the two move refusals the flow test's
    /// happy path does not reach.
    #[test]
    fn restore_version_and_move_refusals() {
        let dir = TempDir::new("restver");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let summary = vault_entry_upsert_inner(&state, &dir.0, draft(None, "V1")).unwrap();
        let mut edited = draft(Some(summary.id.clone()), "V2");
        edited.password = Some("pw-2".into());
        let edited = vault_entry_upsert_inner(&state, &dir.0, edited).unwrap();
        let detail = vault_entry_get_inner(&state, &summary.id).unwrap();
        assert_eq!(detail.history.len(), 1);
        let old_stamp = detail.history[0].updated_at;

        // Restore the old version: it becomes current with a fresh stamp,
        // and the replaced state goes back to history as a restore.
        let restored =
            vault_entry_restore_version_inner(&state, &dir.0, &summary.id, old_stamp).unwrap();
        assert_eq!(restored.title, "V1");
        let detail = vault_entry_get_inner(&state, &summary.id).unwrap();
        assert_eq!(detail.title, "V1");
        assert_eq!(detail.history.len(), 1);
        assert_eq!(detail.history[0].reason, model::VersionReason::Restore);
        assert!(detail.history[0].updated_at > old_stamp);
        assert!(restored.updated_at > edited.updated_at);
        // Unknown stamps are refused on the restore path.
        assert_eq!(
            vault_entry_restore_version_inner(&state, &dir.0, &summary.id, 1).unwrap_err(),
            "vault: no such version"
        );

        // Move into Trash goes through vault_entry_trash, never vault_entry_move.
        assert_eq!(
            vault_entry_move_inner(&state, &dir.0, vec![summary.id.clone()], TRASH_ID.into())
                .unwrap_err(),
            "vault: use trash instead"
        );
        // An unknown group id is refused.
        assert_eq!(
            vault_entry_move_inner(&state, &dir.0, vec![summary.id.clone()], "nope".into())
                .unwrap_err(),
            "vault: no such group"
        );
        vault_entry_trash_inner(&state, &dir.0, vec![summary.id.clone()]).unwrap();
        // And an entry already in trash cannot be moved out except by restore.
        assert_eq!(
            vault_entry_move_inner(&state, &dir.0, vec![summary.id.clone()], ROOT_ID.into())
                .unwrap_err(),
            "vault: restore from Trash first"
        );
    }

    /// Draft semantics: an absent totp keeps the stored URI (the detail view
    /// carries no URI, so the editor cannot send it back), an explicit null
    /// clears it, and a round-tripped custom field (value null, as
    /// EntryDetail hands hidden values out) keeps its value.
    #[test]
    fn upsert_draft_secret_semantics() {
        let dir = TempDir::new("drafts");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let uri = "otpauth://totp/T?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let mut d = draft(None, "Site");
        d.totp = Some(Some(uri.to_string()));
        d.custom_fields = vec![model::DraftCustomField {
            name: "pin".into(),
            hidden: true,
            value: Some(Some("1234".into())),
        }];
        let summary = vault_entry_upsert_inner(&state, &dir.0, d).unwrap();

        // Edit WITHOUT touching totp or the custom field value.
        let mut edit = draft(Some(summary.id.clone()), "Site 2");
        edit.totp = None;
        edit.custom_fields = vec![model::DraftCustomField {
            name: "pin".into(),
            hidden: true,
            value: Some(None),
        }];
        vault_entry_upsert_inner(&state, &dir.0, edit).unwrap();
        assert_eq!(
            vault_entry_reveal_inner(&state, &summary.id, "totp").unwrap(),
            uri,
            "an absent totp must keep the stored URI"
        );
        assert_eq!(
            vault_entry_reveal_inner(&state, &summary.id, "custom:pin").unwrap(),
            "1234",
            "a null custom value must keep the stored value"
        );

        // An explicit null clears the totp.
        let mut clear = draft(Some(summary.id.clone()), "Site 3");
        clear.totp = Some(None);
        vault_entry_upsert_inner(&state, &dir.0, clear).unwrap();
        assert_eq!(
            vault_entry_reveal_inner(&state, &summary.id, "totp").unwrap_err(),
            "vault: no TOTP on this entry"
        );
    }

    /// A create into Trash has no `trashed_from` to restore to, so it is
    /// refused the same way an update that moves an entry there is.
    #[test]
    fn creating_into_trash_is_refused() {
        let dir = TempDir::new("createtrash");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let mut d = draft(None, "Nope");
        d.group_id = TRASH_ID.into();
        assert_eq!(
            vault_entry_upsert_inner(&state, &dir.0, d).unwrap_err(),
            "vault: use trash instead"
        );
    }

    /// A blocked save refuses BEFORE the payload is touched. An edit that only
    /// ever lived in memory would be lost by a quit that never prompted for it,
    /// because no seal was parked either.
    #[test]
    fn a_blocked_vault_refuses_a_mutation_before_it_lands() {
        let dir = TempDir::new("blockedmut");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let kept = vault_entry_upsert_inner(&state, &dir.0, draft(None, "Kept")).unwrap();
        state.save_blocked.store(true, Ordering::SeqCst);

        let mut edit = draft(Some(kept.id.clone()), "Renamed");
        edit.password = Some("new-pw".into());
        assert_eq!(
            vault_entry_upsert_inner(&state, &dir.0, edit).unwrap_err(),
            "vault: restore the snapshot before saving"
        );
        // Nothing changed in memory, so nothing can be lost.
        let after = vault_entry_get_inner(&state, &kept.id).unwrap();
        assert_eq!(after.title, "Kept");
        assert_eq!(
            vault_entry_reveal_inner(&state, &kept.id, "password").unwrap(),
            "pw-1"
        );

        for refused in [
            vault_entry_move_inner(&state, &dir.0, vec![kept.id.clone()], ROOT_ID.into()).err(),
            vault_entry_trash_inner(&state, &dir.0, vec![kept.id.clone()]).err(),
            vault_entry_delete_inner(&state, &dir.0, vec![kept.id.clone()]).err(),
            vault_group_upsert_inner(
                &state,
                &dir.0,
                group_draft(None, Some(ROOT_ID.into()), "Work"),
            )
            .err(),
            vault_group_delete_inner(&state, &dir.0, BROWSER_ID.into()).err(),
        ] {
            assert_eq!(
                refused.as_deref(),
                Some("vault: restore the snapshot before saving")
            );
        }
        assert_eq!(
            vault_entry_restore_version_inner(&state, &dir.0, &kept.id, 1).unwrap_err(),
            "vault: restore the snapshot before saving"
        );
    }
}
