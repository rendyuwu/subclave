//! Group mutation commands: create, rename, reparent and delete.

use std::path::Path;

use tauri::AppHandle;

use crate::modules::vault::events::{emit_changed, run_blocking};
use crate::modules::vault::model::{
    stamp_next, Group, GroupDraft, Tombstone, TombstoneKind, BROWSER_ID, ROOT_ID, TRASH_ID,
};
use crate::modules::vault::query::is_descendant;
use crate::modules::vault::session::vault_dir;
use crate::modules::vault::state::{commit, now_ms, VaultState, LOCKED_ERR};

#[tauri::command]
pub async fn vault_group_upsert(app: AppHandle, group: GroupDraft) -> Result<Group, String> {
    let dir = vault_dir(&app)?;
    let result = run_blocking(&app, move |state| {
        vault_group_upsert_inner(state, &dir, group)
    })
    .await;
    if result.is_ok() {
        emit_changed(&app, &[], "local");
    }
    result
}

pub(crate) fn vault_group_upsert_inner(
    state: &VaultState,
    dir: &Path,
    draft: GroupDraft,
) -> Result<Group, String> {
    state.ensure_writable()?;
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let payload = &mut unlocked.payload;
    let name = draft.name.trim().to_string();
    if name.is_empty() {
        return Err("vault: group name is required".to_string());
    }
    let now = now_ms();
    let group = match &draft.id {
        None => {
            let parent = draft
                .parent_id
                .clone()
                .unwrap_or_else(|| ROOT_ID.to_string());
            if parent != ROOT_ID && !payload.groups.iter().any(|g| g.id == parent) {
                return Err("vault: no such group".to_string());
            }
            if payload.groups.iter().any(|g| {
                g.parent_id.as_deref().unwrap_or(ROOT_ID) == parent
                    && g.name.eq_ignore_ascii_case(&name)
            }) {
                return Err("vault: a sibling group already has that name".to_string());
            }
            let group = Group {
                id: uuid::Uuid::new_v4().to_string(),
                parent_id: Some(parent),
                name,
                icon: draft.icon.clone(),
                color: draft.color.clone(),
                created_at: now,
                updated_at: now,
            };
            payload.groups.push(group.clone());
            group
        }
        Some(id) => {
            if [ROOT_ID, TRASH_ID, BROWSER_ID].contains(&id.as_str()) {
                return Err("vault: reserved group".to_string());
            }
            let existing = payload
                .groups
                .iter()
                .find(|g| &g.id == id)
                .ok_or_else(|| "vault: no such group".to_string())?
                .clone();
            let parent = draft.parent_id.clone().unwrap_or_else(|| {
                existing
                    .parent_id
                    .clone()
                    .unwrap_or_else(|| ROOT_ID.to_string())
            });
            if parent != ROOT_ID && !payload.groups.iter().any(|g| g.id == parent) {
                return Err("vault: no such group".to_string());
            }
            if parent == *id || is_descendant(payload, id, &parent) {
                return Err("vault: cannot move a group into its own subtree".to_string());
            }
            if payload.groups.iter().any(|g| {
                &g.id != id
                    && g.parent_id.as_deref().unwrap_or(ROOT_ID) == parent
                    && g.name.eq_ignore_ascii_case(&name)
            }) {
                return Err("vault: a sibling group already has that name".to_string());
            }
            let group = payload
                .groups
                .iter_mut()
                .find(|g| &g.id == id)
                .ok_or_else(|| "vault: no such group".to_string())?;
            group.name = name;
            group.parent_id = Some(parent);
            group.icon = draft.icon.clone();
            group.color = draft.color.clone();
            group.updated_at = stamp_next(now, group.updated_at);
            group.clone()
        }
    };
    payload.device.sync.mark_dirty("group", &group.id);
    drop(guard);
    commit(state, dir)?;
    Ok(group)
}

#[tauri::command]
pub async fn vault_group_delete(app: AppHandle, id: String) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let result = run_blocking(&app, move |state| vault_group_delete_inner(state, &dir, id)).await;
    if result.is_ok() {
        emit_changed(&app, &[], "local");
    }
    result
}

pub(crate) fn vault_group_delete_inner(
    state: &VaultState,
    dir: &Path,
    id: String,
) -> Result<(), String> {
    state.ensure_writable()?;
    // The reserved groups are load-bearing: trash is the restore source,
    // browser is the sync-stable save-login target, root is the fallback
    // parent. Entries point at them by id, and M3 sync would propagate a
    // tombstone for a shared reserved id.
    if [ROOT_ID, TRASH_ID, BROWSER_ID].contains(&id.as_str()) {
        return Err("vault: reserved group".to_string());
    }
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let payload = &mut unlocked.payload;
    let group = payload
        .groups
        .iter()
        .find(|g| g.id == id)
        .ok_or_else(|| "vault: no such group".to_string())?
        .clone();
    let name = group.name.clone();
    let now = now_ms();
    let entry_count = payload.entries.iter().filter(|e| e.group_id == id).count();
    let child_count = payload
        .groups
        .iter()
        .filter(|g| g.parent_id.as_deref() == Some(&id))
        .count();
    if entry_count + child_count > 0 {
        return Err(format!(
            "vault: move or delete the {} items in {} first",
            entry_count + child_count,
            name
        ));
    }
    payload.groups.retain(|g| g.id != id);
    payload.device.sync.mark_dirty("group", &id);
    payload.tombstones.push(Tombstone {
        id,
        kind: TombstoneKind::Group,
        deleted_at: now,
    });
    drop(guard);
    commit(state, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::modules::vault::session::*;
    use crate::modules::vault::test_util::*;

    /// Reserved groups are refused on delete, whatever they currently hold.
    #[test]
    fn reserved_groups_cannot_be_deleted() {
        let dir = TempDir::new("reserved");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        for id in [ROOT_ID, TRASH_ID, BROWSER_ID] {
            assert_eq!(
                vault_group_delete_inner(&state, &dir.0, id.into()).unwrap_err(),
                "vault: reserved group",
                "{id} must be undeletable"
            );
        }
    }
}
