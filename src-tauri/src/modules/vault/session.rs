//! Vault session commands: status, create, unlock, lock, touch, the master
//! password change, the pending-save retry and the snapshot restore.
//!
//! Each registered command is a thin async shell over an inner function that
//! takes `&VaultState` plus plain values, which is what keeps the core
//! testable without a Tauri runtime.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::modules::lockext::LockExt as _;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::modules::prefs;
use crate::modules::vault::events::{
    drain_auto_lock, drain_save_event, emit_changed, emit_locked, run_blocking, LockReason,
};
use crate::modules::vault::file;
use crate::modules::vault::file::{
    load_vault, open_file, save_vault, seal_payload, VAULT_FILE_NAME,
};
use crate::modules::vault::kdf::{derive_key, fresh_params};
use crate::modules::vault::lock;
use crate::modules::vault::model::{seed_reserved_groups, VaultPayload};
use crate::modules::vault::state::{now_ms, perform_save_locked, Unlocked, VaultState, LOCKED_ERR};

/// The app data dir for a command. Tauri's resolver applies the `.dev`
/// suffix from `tauri.dev.conf.json`.
pub(crate) fn vault_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| format!("vault: {e}"))
}

// ---- Command shells ----
//
// Each shell resolves the dir when it needs one, then hands the inner function
// to `run_blocking` (see the events module), which runs it off the UI thread
// and drains the event flags. `vault_lock` and `vault_retry_save` keep their
// own bodies because their emit and drain order differs.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub exists: bool,
    pub locked: bool,
    pub save_pending: bool,
    /// True when the payload was opened from the `.bak`. While true every
    /// save is refused with "vault: restore the snapshot before saving".
    pub save_blocked: bool,
    /// Epoch ms mtime of the `.bak`, `None` when there is no `.bak`.
    pub backup_at: Option<u64>,
    /// Ms until the idle deadline, `None` while locked or when auto-lock is
    /// off. Both numbers are boot-clock ms, so the subtraction is meaningful.
    pub locks_in_ms: Option<u64>,
}

#[tauri::command]
pub async fn vault_status(app: AppHandle) -> Result<VaultStatus, String> {
    let dir = vault_dir(&app)?;
    run_blocking(&app, move |state| vault_status_inner(state, &dir)).await
}

/// Epoch ms mtime of the `.bak`, `None` when it is absent or unreadable.
fn bak_mtime_ms(dir: &Path) -> Option<u64> {
    let modified = std::fs::metadata(dir.join(format!("{VAULT_FILE_NAME}.bak")))
        .ok()?
        .modified()
        .ok()?;
    let since_epoch = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(since_epoch.as_millis() as u64)
}

pub(crate) fn vault_status_inner(state: &VaultState, dir: &Path) -> Result<VaultStatus, String> {
    // Through access so an expired deadline locks (and wipes) before the
    // report; the expired call itself answers Err(locked), which is exactly
    // the state to report.
    let locked = match state.access() {
        Ok(guard) => guard.is_none(),
        Err(e) if e == LOCKED_ERR => true,
        Err(e) => return Err(e),
    };
    let exists =
        dir.join(VAULT_FILE_NAME).is_file() || dir.join(format!("{VAULT_FILE_NAME}.bak")).is_file();
    let locks_in_ms = if locked {
        None
    } else {
        let deadline = state.deadline_ms.load(Ordering::SeqCst);
        (deadline != u64::MAX).then(|| deadline.saturating_sub(lock::boot_now_ms()))
    };
    Ok(VaultStatus {
        exists,
        locked,
        save_pending: state.has_pending(),
        save_blocked: state.save_blocked.load(Ordering::SeqCst),
        backup_at: bak_mtime_ms(dir),
        locks_in_ms,
    })
}

#[tauri::command]
pub async fn vault_create(app: AppHandle, master_password: String) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    run_blocking(&app, move |state| {
        vault_create_inner(state, &dir, &master_password)
    })
    .await
}

pub(crate) fn vault_create_inner(
    state: &VaultState,
    dir: &Path,
    master_password: &str,
) -> Result<(), String> {
    if dir.join(VAULT_FILE_NAME).exists() || dir.join(format!("{VAULT_FILE_NAME}.bak")).exists() {
        return Err("vault: a vault file already exists".to_string());
    }
    let now = now_ms();
    let mut payload = VaultPayload::default();
    seed_reserved_groups(&mut payload, now);
    install_new_vault(state, dir, master_password, payload)
}

/// Seal `payload` under `master_password`, write it fresh, and install it as
/// the unlocked session. This is the lower half of [`vault_create_inner`],
/// shared with the sync join path so a payload pulled from the remote can be
/// installed without reaching into [`VaultState`] internals.
pub(crate) fn install_new_vault(
    state: &VaultState,
    dir: &Path,
    master_password: &str,
    payload: VaultPayload,
) -> Result<(), String> {
    if master_password.chars().count() < 8 {
        return Err("vault: master password must be at least 8 characters".to_string());
    }
    let kdf = fresh_params()?;
    let key = derive_key(master_password, &kdf)?;
    let sealed = seal_payload(&payload, &key, &kdf)?;
    // Write WITHOUT parking: a failed create must not leave a vault the user
    // was told does not exist for the retry to land later. A successful
    // write also drops any parked seal.
    {
        let _held = state.save_lock.lock_or_recover();
        if state.save_blocked.load(Ordering::SeqCst) {
            return Err("vault: restore the snapshot before saving".to_string());
        }
        save_vault(dir, &sealed).map_err(|e| format!("vault: {e}"))?;
        state.drop_pending();
    }
    // Refresh BEFORE installing: a concurrent access() must not see the
    // previous session's expired deadline while the new one is installed.
    state.refresh_deadline(prefs::read(dir).auto_lock_minutes);
    *state.inner.lock_or_recover() = Some(Unlocked { payload, key, kdf });
    Ok(())
}

#[tauri::command]
pub async fn vault_unlock(app: AppHandle, master_password: String) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    run_blocking(&app, move |state| {
        vault_unlock_inner(state, &dir, &master_password)
    })
    .await
}

pub(crate) fn vault_unlock_inner(
    state: &VaultState,
    dir: &Path,
    master_password: &str,
) -> Result<(), String> {
    let (file, from_bak) = load_vault(dir)?;
    // A structurally broken primary already fell back to the `.bak`. When the
    // primary parses but fails to OPEN (a GCM tag failure from bit rot or
    // tampering, which `load_vault` cannot see without the key), retry the
    // `.bak` before reporting: a good backup beside a rotted primary is
    // recoverable, and the failure would otherwise be an opaque message. A
    // wrong password fails on both files and keeps that same message.
    let (opened, from_bak) = match open_file(&file, master_password) {
        Ok(opened) => (opened, from_bak),
        // `load_vault` already handed back the `.bak` (the primary was
        // unreadable), so there is nothing else to try: re-opening the same
        // file would only spend a second key derivation.
        Err(primary_err) if from_bak => return Err(primary_err),
        Err(primary_err) => match file::read_bak(dir) {
            Some(bak) => match open_file(&bak, master_password) {
                Ok(opened) => (opened, true),
                Err(_) => return Err(primary_err),
            },
            None => return Err(primary_err),
        },
    };
    state.save_blocked.store(from_bak, Ordering::SeqCst);
    // A parked seal is newer than the disk copy it failed to replace (a
    // successful save clears it, so it can only still exist while the write
    // keeps failing). It is sealed under the same key and kdf this file just
    // derived, so opening it shows the user their edits. A stale disk copy is
    // not a broken primary, so this does not set save_blocked: the next
    // commit simply lands.
    let payload = match state.pending.lock_or_recover().clone() {
        Some(pending) => open_file(&pending, master_password)
            .map(|o| o.payload)
            .unwrap_or_else(|_| opened.payload),
        None => opened.payload,
    };
    state.refresh_deadline(prefs::read(dir).auto_lock_minutes);
    *state.inner.lock_or_recover() = Some(Unlocked {
        payload,
        key: opened.key,
        kdf: opened.kdf,
    });
    Ok(())
}

/// The registered command takes no args and locks for the manual reason.
#[tauri::command]
pub async fn vault_lock(app: AppHandle) -> Result<(), String> {
    let task_app = app.clone();
    let result: Result<(), String> = tauri::async_runtime::spawn_blocking(move || {
        task_app.state::<VaultState>().lock_inner();
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"));
    // lock_inner records no flag, so the event is emitted exactly once.
    emit_locked(&app, LockReason::Manual);
    result
}

#[tauri::command]
pub async fn vault_touch(app: AppHandle) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    run_blocking(&app, move |state| vault_touch_inner(state, &dir)).await
}

fn vault_touch_inner(state: &VaultState, dir: &Path) -> Result<(), String> {
    // Through access so an expired deadline locks instead of extending. The
    // guard is dropped immediately: touch only extends the deadline. Reading
    // the preference here is also how a Settings change takes effect without
    // waiting for the next unlock.
    drop(state.access()?);
    state.refresh_deadline(prefs::read(dir).auto_lock_minutes);
    Ok(())
}

#[tauri::command]
pub async fn vault_change_master(
    app: AppHandle,
    current: String,
    next: String,
) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    run_blocking(&app, move |state| {
        vault_change_master_inner(state, &dir, &current, &next)
    })
    .await
}

pub(crate) fn vault_change_master_inner(
    state: &VaultState,
    dir: &Path,
    current: &str,
    next: &str,
) -> Result<(), String> {
    // Verify against the on-disk file, not the in-memory key: a wrong
    // `current` must fail the same way a fresh unlock does.
    let (file, _) = load_vault(dir)?;
    let opened = open_file(&file, current)?;
    if next.chars().count() < 8 {
        return Err("vault: master password must be at least 8 characters".to_string());
    }
    // save_lock first, per the save-machinery lock order: the re-seal and
    // its write are one ordered unit against concurrent commits.
    let _held = state.save_lock.lock_or_recover();
    // The in-memory payload is the source of truth for the re-seal: it holds
    // every edit since unlock, and the sync state and browser pairings inside
    // it carry over untouched.
    let (payload, _) = {
        let guard = state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        (unlocked.payload.clone(), unlocked.kdf.clone())
    };
    let new_kdf = fresh_params()?;
    let new_key = derive_key(next, &new_kdf)?;
    let sealed = seal_payload(&payload, &new_key, &new_kdf)?;
    // Write WITHOUT parking: a "failed" change must not leave the disk
    // opening with the new password while the user believes it failed. A
    // parked pre-change seal under the old key is superseded either way: on
    // success it is dropped, on failure the next commit re-seals under the
    // old key and overwrites it.
    if state.save_blocked.load(Ordering::SeqCst) {
        return Err("vault: restore the snapshot before saving".to_string());
    }
    save_vault(dir, &sealed).map_err(|e| format!("vault: {e}"))?;
    state.drop_pending();
    // Only after the write lands: the disk opens with the new password from
    // this point, so the in-memory key must follow.
    {
        let mut guard = state.access()?;
        if let Some(unlocked) = guard.as_mut() {
            unlocked.key = new_key;
            unlocked.kdf = new_kdf;
        }
    }
    // Dropping `opened` scrubs its derived copy of the old key; the payload
    // clone drops with this scope.
    drop(opened);
    Ok(())
}

#[tauri::command]
pub async fn vault_retry_save(app: AppHandle) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_retry_save_inner(&task_app.state::<VaultState>(), &dir)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))
    .and_then(|r| r);
    drain_save_event(&app);
    drain_auto_lock(&app);
    result
}

/// Also what the background tick calls every 10 s. Works while locked:
/// `pending` holds ciphertext only. Success clears `pending`; failure leaves
/// it, because it already holds the newest seal.
///
/// The seal is COPIED out, not taken, and `save_lock` is held across the whole
/// attempt: `pending` therefore never reads empty while a retry is in flight,
/// which is what [`VaultState::has_pending`] needs to answer the quit prompt
/// (a taken seal left a window where a close would exit and lose the write).
pub(crate) fn vault_retry_save_inner(state: &VaultState, dir: &Path) -> Result<(), String> {
    let _held = state.save_lock.lock_or_recover();
    // Bound in its own statement, not read inside the `match` scrutinee: a
    // temporary guard there lives until the end of the match and
    // `perform_save_locked` locks `pending` itself, which deadlocks.
    let parked = state.pending.lock_or_recover().clone();
    match parked {
        // perform_save_locked re-parks it on failure, so the retry state is kept.
        Some(file) => perform_save_locked(state, dir, file),
        None => Ok(()),
    }
}

/// Recover a vault whose payload was opened from the `.bak`. The broken
/// primary is moved to `subclave-vault.json.corrupt` so the commit that
/// follows cannot overwrite something the user might still want, then the
/// in-memory payload is sealed and written fresh.
#[tauri::command]
pub async fn vault_restore_snapshot(app: AppHandle) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let result = run_blocking(&app, move |state| vault_restore_snapshot_inner(state, &dir)).await;
    if result.is_ok() {
        emit_changed(&app, &[], "local");
    }
    result
}

fn vault_restore_snapshot_inner(state: &VaultState, dir: &Path) -> Result<(), String> {
    // Held across the whole restore, so two clicks cannot interleave: without
    // it the second call could pass the flag check before the first clears it
    // and rename the freshly written good primary over the broken copy.
    let _held = state.save_lock.lock_or_recover();
    if !state.save_blocked.load(Ordering::SeqCst) {
        return Err("vault: no snapshot to restore".to_string());
    }
    // Refuse before moving anything when locked: the seal below needs the
    // in-memory payload and key, and the broken primary must not be moved
    // aside for a restore that cannot run.
    if state.access()?.is_none() {
        return Err(LOCKED_ERR.to_string());
    }
    let primary = dir.join(VAULT_FILE_NAME);
    if primary.exists() {
        let corrupt = dir.join(format!("{VAULT_FILE_NAME}.corrupt"));
        // The newest broken bytes are the ones worth keeping: `fs::rename`
        // replaces the target, so drop the previous copy explicitly rather
        // than pretending the first one survives.
        if corrupt.exists() {
            std::fs::remove_file(&corrupt).map_err(|e| format!("vault: {e}"))?;
        }
        std::fs::rename(&primary, &corrupt).map_err(|e| format!("vault: {e}"))?;
    }
    state.save_blocked.store(false, Ordering::SeqCst);
    // Seals the payload already in memory with the held key and writes primary
    // plus `.bak` fresh, which also clears any parked seal. On failure the
    // seal is parked, the flag stays false, and the next retry lands (the
    // broken file is already out of the way).
    let sealed = {
        let guard = state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        seal_payload(&unlocked.payload, &unlocked.key, &unlocked.kdf)?
    };
    perform_save_locked(state, dir, sealed)
}

#[cfg(test)]
mod tests {
    use super::*;

    use base64::{engine::general_purpose::STANDARD as B64, Engine};

    use crate::modules::vault::entry_commands::*;
    use crate::modules::vault::group_commands::*;
    use crate::modules::vault::model::{TombstoneKind, ROOT_ID};
    use crate::modules::vault::query::*;
    use crate::modules::vault::test_util::*;

    /// The full core flow: create -> mutate -> save -> reload -> lock ->
    /// unlock, plus the refusals each step carries.
    #[test]
    fn create_mutate_reload_lock_unlock_flow() {
        let dir = TempDir::new("full");
        let state = VaultState::default();

        // Create. A short password and a second vault are refused.
        assert_eq!(
            vault_create_inner(&state, &dir.0, "short").unwrap_err(),
            "vault: master password must be at least 8 characters"
        );
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        assert_eq!(
            vault_create_inner(&state, &dir.0, "master-pw").unwrap_err(),
            "vault: a vault file already exists"
        );

        // Upsert, with tag normalization visible in the summary.
        let summary = vault_entry_upsert_inner(&state, &dir.0, draft(None, "Site")).unwrap();
        assert_eq!(summary.title, "Site");
        assert_eq!(summary.tags, vec!["tag".to_string()]);

        // Reveal stores the secret; an unknown id and field are refused.
        assert_eq!(
            vault_entry_reveal_inner(&state, &summary.id, "password").unwrap(),
            "pw-1"
        );
        assert_eq!(
            vault_entry_reveal_inner(&state, "nope", "password").unwrap_err(),
            "vault: no such entry"
        );
        assert_eq!(
            vault_entry_reveal_inner(&state, &summary.id, "nope").unwrap_err(),
            "vault: unknown field"
        );

        // A content edit pushes a history version with the previous stamp.
        let detail_before = vault_entry_get_inner(&state, &summary.id).unwrap();
        let mut edited = draft(Some(summary.id.clone()), "Site 2");
        edited.password = None; // absent password keeps the stored one
        let edited_summary = vault_entry_upsert_inner(&state, &dir.0, edited).unwrap();
        let detail_after = vault_entry_get_inner(&state, &summary.id).unwrap();
        assert_eq!(detail_after.title, "Site 2");
        assert_eq!(detail_after.history.len(), 1);
        assert_eq!(detail_after.history[0].updated_at, detail_before.updated_at);
        assert!(detail_after.history[0]
            .changed
            .contains(&"title".to_string()));
        assert!(edited_summary.updated_at > detail_before.updated_at);

        // Group refusals: sibling clash, cycle, non-empty delete.
        let child = vault_group_upsert_inner(
            &state,
            &dir.0,
            group_draft(None, Some(ROOT_ID.into()), "Work"),
        )
        .unwrap();
        assert_eq!(
            vault_group_upsert_inner(
                &state,
                &dir.0,
                group_draft(None, Some(ROOT_ID.into()), "work")
            )
            .unwrap_err(),
            "vault: a sibling group already has that name"
        );
        assert_eq!(
            vault_group_upsert_inner(
                &state,
                &dir.0,
                group_draft(Some(child.id.clone()), Some(child.id.clone()), "Work2")
            )
            .unwrap_err(),
            "vault: cannot move a group into its own subtree"
        );
        vault_entry_move_inner(&state, &dir.0, vec![summary.id.clone()], child.id.clone()).unwrap();
        assert_eq!(
            vault_group_delete_inner(&state, &dir.0, child.id.clone()).unwrap_err(),
            "vault: move or delete the 1 items in Work first"
        );

        // Delete demands Trash; then trash -> restore lands back in the group.
        assert_eq!(
            vault_entry_delete_inner(&state, &dir.0, vec![summary.id.clone()]).unwrap_err(),
            "vault: only entries in Trash can be deleted permanently"
        );
        vault_entry_trash_inner(&state, &dir.0, vec![summary.id.clone()]).unwrap();
        vault_entry_restore_inner(&state, &dir.0, vec![summary.id.clone()]).unwrap();
        let restored = vault_entry_get_inner(&state, &summary.id).unwrap();
        assert_eq!(restored.group_id, child.id);

        // Restore to root when trashed_from is gone.
        vault_group_delete_inner(&state, &dir.0, child.id.clone()).unwrap_err();
        vault_entry_trash_inner(&state, &dir.0, vec![summary.id.clone()]).unwrap();
        let mut child_empty = child.clone();
        child_empty.name = "Work".into();
        // Delete is still refused while the entry is in Trash inside it.
        // Move it out of the group first, then the group delete succeeds.
        vault_entry_restore_inner(&state, &dir.0, vec![summary.id.clone()]).unwrap();
        vault_entry_move_inner(&state, &dir.0, vec![summary.id.clone()], ROOT_ID.into()).unwrap();
        vault_group_delete_inner(&state, &dir.0, child.id.clone()).unwrap();
        vault_entry_trash_inner(&state, &dir.0, vec![summary.id.clone()]).unwrap();
        // The trashed_from group no longer exists, so restore lands in root.
        vault_entry_restore_inner(&state, &dir.0, vec![summary.id.clone()]).unwrap();
        let after = vault_entry_get_inner(&state, &summary.id).unwrap();
        assert_eq!(after.group_id, ROOT_ID);

        // Permanent delete writes a tombstone.
        vault_entry_trash_inner(&state, &dir.0, vec![summary.id.clone()]).unwrap();
        vault_entry_delete_inner(&state, &dir.0, vec![summary.id.clone()]).unwrap();
        let guard = state.access().unwrap();
        let payload = &guard.as_ref().unwrap().payload;
        assert!(payload.entries.is_empty());
        // One entry tombstone plus the group tombstone from the delete above.
        assert_eq!(payload.tombstones.len(), 2);
        assert!(payload
            .tombstones
            .iter()
            .any(|t| t.id == summary.id && t.kind == TombstoneKind::Entry));
        assert!(payload
            .tombstones
            .iter()
            .any(|t| t.id == child.id && t.kind == TombstoneKind::Group));
        drop(guard);

        // Reload from disk with the same password.
        let state2 = VaultState::default();
        vault_unlock_inner(&state2, &dir.0, "master-pw").unwrap();

        // Change master: the old password fails on disk, the new one opens,
        // the .bak opens with the new password, and a wrong current fails
        // leaving the old key in place.
        vault_change_master_inner(&state2, &dir.0, "wrong-current", "new-master-pw").unwrap_err();
        vault_change_master_inner(&state2, &dir.0, "master-pw", "new-master-pw").unwrap();
        let state3 = VaultState::default();
        assert_eq!(
            vault_unlock_inner(&state3, &dir.0, "master-pw").unwrap_err(),
            "vault: wrong master password, or the vault file is corrupt"
        );
        vault_unlock_inner(&state3, &dir.0, "new-master-pw").unwrap();
        let (primary, _) = load_vault(&dir.0).unwrap();
        let salt = primary.kdf.salt.clone();
        // The .bak must have been rewritten from the new file, so it shares
        // the new salt.
        let bak_bytes =
            std::fs::read(dir.0.join(format!("{}.bak", file::VAULT_FILE_NAME))).unwrap();
        let bak_file: file::VaultFile = serde_json::from_slice(&bak_bytes).unwrap();
        assert_eq!(
            bak_file.kdf.salt, salt,
            "the .bak was rewritten from the new file"
        );
        let state4 = VaultState::default();
        // Unlock from the .bak path by hiding the primary.
        std::fs::rename(
            dir.0.join(file::VAULT_FILE_NAME),
            dir.0.join("subclave-vault-hidden.json"),
        )
        .unwrap();
        vault_unlock_inner(&state4, &dir.0, "new-master-pw").unwrap();
        assert!(state4.save_blocked.load(Ordering::SeqCst));
        std::fs::rename(
            dir.0.join("subclave-vault-hidden.json"),
            dir.0.join(file::VAULT_FILE_NAME),
        )
        .unwrap();

        // Expiry: force the deadline into the past; access locks and wipes.
        let state5 = VaultState::default();
        vault_unlock_inner(&state5, &dir.0, "new-master-pw").unwrap();
        state5
            .deadline_ms
            .store(1, std::sync::atomic::Ordering::SeqCst);
        // vault_touch locks instead of extending: it goes through access
        // first, so the expired deadline fires and the session is gone.
        assert_eq!(vault_touch_inner(&state5, &dir.0).unwrap_err(), LOCKED_ERR);
        {
            let guard = state5.inner.lock().unwrap();
            assert!(guard.is_none(), "the expired access must have locked");
        }
        // Every later entry command answers locked; a later touch only
        // refreshes the deadline and cannot unlock anything.
        assert_eq!(
            vault_entry_reveal_inner(&state5, "any", "password").unwrap_err(),
            LOCKED_ERR
        );
        assert!(vault_touch_inner(&state5, &dir.0).is_ok());

        // Retry path: a directory at the vault path makes the write fail,
        // pending holds the seal, the retry succeeds once it is gone.
        let state6 = VaultState::default();
        vault_unlock_inner(&state6, &dir.0, "new-master-pw").unwrap();
        std::fs::create_dir_all(dir.0.join("blocker")).unwrap();
        std::fs::rename(
            dir.0.join(file::VAULT_FILE_NAME),
            dir.0.join("blocker/hold.json"),
        )
        .unwrap();
        std::fs::create_dir_all(dir.0.join(file::VAULT_FILE_NAME)).unwrap();
        assert!(vault_entry_upsert_inner(&state6, &dir.0, draft(None, "During")).is_err());
        assert!(state6.pending.lock().unwrap().is_some());
        std::fs::remove_dir_all(dir.0.join(file::VAULT_FILE_NAME)).unwrap();
        vault_retry_save_inner(&state6, &dir.0).unwrap();
        assert!(state6.pending.lock().unwrap().is_none());
        // The retried write landed and the entry is on disk.
        let state7 = VaultState::default();
        vault_unlock_inner(&state7, &dir.0, "new-master-pw").unwrap();
        let list = vault_list_inner(&state7).unwrap();
        assert!(list.entries.iter().any(|e| e.title == "During"));

        // While save_blocked, commit refuses.
        state6.save_blocked.store(true, Ordering::SeqCst);
        assert_eq!(
            vault_entry_upsert_inner(&state6, &dir.0, draft(None, "Blocked")).unwrap_err(),
            "vault: restore the snapshot before saving"
        );
    }

    /// A structurally broken primary falls back to the `.bak`, the status
    /// reports the block and the backup time, and the restore moves the broken
    /// bytes aside and writes fresh.
    #[test]
    fn restore_snapshot_recovers_a_structurally_broken_primary() {
        let dir = TempDir::new("restore");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let kept = vault_entry_upsert_inner(&state, &dir.0, draft(None, "Keep me")).unwrap();

        // Break the primary so load_vault falls back to the good .bak.
        let primary = dir.0.join(file::VAULT_FILE_NAME);
        let mut broken: file::VaultFile =
            serde_json::from_slice(&std::fs::read(&primary).unwrap()).unwrap();
        broken.ciphertext = "!".into();
        let broken_bytes = file::vault_file_bytes(&broken).unwrap();
        std::fs::write(&primary, &broken_bytes).unwrap();

        let reopened = VaultState::default();
        vault_unlock_inner(&reopened, &dir.0, "master-pw").unwrap();
        let status = vault_status_inner(&reopened, &dir.0).unwrap();
        assert!(status.save_blocked);
        assert!(status.backup_at.is_some());
        assert!(status.exists);
        assert!(!status.locked);
        assert!(!status.save_pending);

        // Saving is refused until the snapshot is restored.
        assert_eq!(
            vault_entry_upsert_inner(&reopened, &dir.0, draft(None, "Nope")).unwrap_err(),
            "vault: restore the snapshot before saving"
        );

        // A restore while locked refuses before moving anything.
        let broken_before = std::fs::read(&primary).unwrap();
        assert!(reopened.lock_inner());
        assert_eq!(
            vault_restore_snapshot_inner(&reopened, &dir.0).unwrap_err(),
            LOCKED_ERR
        );
        assert_eq!(std::fs::read(&primary).unwrap(), broken_before);
        vault_unlock_inner(&reopened, &dir.0, "master-pw").unwrap();

        vault_restore_snapshot_inner(&reopened, &dir.0).unwrap();
        assert!(!reopened.save_blocked.load(Ordering::SeqCst));
        let corrupt = dir.0.join(format!("{VAULT_FILE_NAME}.corrupt"));
        assert_eq!(std::fs::read(&corrupt).unwrap(), broken_bytes);
        assert!(!vault_status_inner(&reopened, &dir.0).unwrap().save_blocked);

        // The next commit lands, and both the restored and the new entry are
        // on disk.
        vault_entry_upsert_inner(&reopened, &dir.0, draft(None, "After restore")).unwrap();
        let probe = VaultState::default();
        vault_unlock_inner(&probe, &dir.0, "master-pw").unwrap();
        let titles: Vec<String> = vault_list_inner(&probe)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.title)
            .collect();
        assert!(titles.contains(&"Keep me".to_string()));
        assert!(titles.contains(&"After restore".to_string()));
        assert!(kept.updated_at > 0);
        // A restore with nothing blocked is a no-op error.
        assert_eq!(
            vault_restore_snapshot_inner(&probe, &dir.0).unwrap_err(),
            "vault: no snapshot to restore"
        );
    }

    /// A primary that parses and is structurally readable but whose GCM tag
    /// fails (bit rot, tampering) is unopenable, so unlock retries the `.bak`;
    /// with no `.bak` the same call keeps the one opaque message.
    #[test]
    fn unlock_falls_back_to_the_bak_when_the_primary_fails_to_open() {
        let dir = TempDir::new("gcm");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();

        // Valid base64 decoding to 32 bytes: the structural check passes and
        // load_vault hands back the primary, so only the GCM open fails.
        let primary = dir.0.join(file::VAULT_FILE_NAME);
        let mut tampered: file::VaultFile =
            serde_json::from_slice(&std::fs::read(&primary).unwrap()).unwrap();
        tampered.ciphertext = B64.encode([0u8; 32]);
        std::fs::write(&primary, file::vault_file_bytes(&tampered).unwrap()).unwrap();

        let recovered = VaultState::default();
        vault_unlock_inner(&recovered, &dir.0, "master-pw").unwrap();
        assert!(recovered.save_blocked.load(Ordering::SeqCst));
        assert!(vault_status_inner(&recovered, &dir.0).unwrap().save_blocked);

        // Without the .bak there is nothing to fall back to.
        std::fs::remove_file(dir.0.join(format!("{VAULT_FILE_NAME}.bak"))).unwrap();
        let no_bak = VaultState::default();
        assert_eq!(
            vault_unlock_inner(&no_bak, &dir.0, "master-pw").unwrap_err(),
            "vault: wrong master password, or the vault file is corrupt"
        );
    }

    /// The status fields: existence, lock state, a `.bak` mtime, and the
    /// countdown, which follows the `autoLockMinutes` preference and is absent
    /// while locked or when auto-lock is off.
    #[test]
    fn status_reports_backup_and_lock_fields() {
        let dir = TempDir::new("status");
        let state = VaultState::default();
        let status = vault_status_inner(&state, &dir.0).unwrap();
        assert!(!status.exists);
        assert!(status.locked);
        assert!(!status.save_pending);
        assert!(!status.save_blocked);
        assert!(status.backup_at.is_none());
        assert!(status.locks_in_ms.is_none());

        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let status = vault_status_inner(&state, &dir.0).unwrap();
        assert!(status.exists);
        assert!(!status.locked);
        assert!(status.backup_at.is_some(), "create writes a .bak");
        assert!(status.locks_in_ms.unwrap() > 0);

        // Auto-lock off (0 = never) comes from the settings file.
        std::fs::write(
            dir.0.join(prefs::SETTINGS_FILE_NAME),
            r#"{"autoLockMinutes":0}"#,
        )
        .unwrap();
        vault_touch_inner(&state, &dir.0).unwrap();
        let status = vault_status_inner(&state, &dir.0).unwrap();
        assert!(status.locks_in_ms.is_none(), "0 minutes means never");

        // Locked reports no countdown.
        assert!(state.lock_inner());
        let status = vault_status_inner(&state, &dir.0).unwrap();
        assert!(status.locked);
        assert!(status.locks_in_ms.is_none());
    }

    /// A second restore supersedes the first forensic copy instead of keeping
    /// two, and a wrong password with a `.bak` present keeps the one opaque
    /// message and leaves the block flag alone.
    #[test]
    fn restore_supersedes_the_corrupt_copy_and_a_wrong_password_stays_opaque() {
        let dir = TempDir::new("restore2");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let primary = dir.0.join(file::VAULT_FILE_NAME);
        let corrupt = dir.0.join(format!("{VAULT_FILE_NAME}.corrupt"));

        // First break and restore: the corrupt copy is the old broken bytes.
        std::fs::write(&corrupt, b"first").unwrap();
        let mut broken: file::VaultFile =
            serde_json::from_slice(&std::fs::read(&primary).unwrap()).unwrap();
        broken.ciphertext = "!".into();
        std::fs::write(&primary, file::vault_file_bytes(&broken).unwrap()).unwrap();
        vault_unlock_inner(&state, &dir.0, "master-pw").unwrap();
        assert!(state.save_blocked.load(Ordering::SeqCst));
        vault_restore_snapshot_inner(&state, &dir.0).unwrap();
        assert_eq!(
            std::fs::read(&corrupt).unwrap(),
            file::vault_file_bytes(&broken).unwrap()
        );

        // A wrong password against a readable primary with a `.bak` beside it
        // still answers the one message and must not arm the block flag.
        let wrong = VaultState::default();
        assert_eq!(
            vault_unlock_inner(&wrong, &dir.0, "not-the-password").unwrap_err(),
            "vault: wrong master password, or the vault file is corrupt"
        );
        assert!(!wrong.save_blocked.load(Ordering::SeqCst));
    }

    /// A restore whose rename fails leaves the block flag set, so the broken
    /// file is never overwritten by the write that would have followed.
    #[test]
    fn a_failed_restore_rename_keeps_the_block() {
        let dir = TempDir::new("restorefail");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let primary = dir.0.join(file::VAULT_FILE_NAME);
        let mut broken: file::VaultFile =
            serde_json::from_slice(&std::fs::read(&primary).unwrap()).unwrap();
        broken.ciphertext = "!".into();
        std::fs::write(&primary, file::vault_file_bytes(&broken).unwrap()).unwrap();
        vault_unlock_inner(&state, &dir.0, "master-pw").unwrap();
        assert!(state.save_blocked.load(Ordering::SeqCst));

        // A directory where the `.corrupt` file belongs makes the rename fail.
        let corrupt = dir.0.join(format!("{VAULT_FILE_NAME}.corrupt"));
        std::fs::create_dir_all(&corrupt).unwrap();
        assert!(vault_restore_snapshot_inner(&state, &dir.0).is_err());
        assert!(
            state.save_blocked.load(Ordering::SeqCst),
            "a failed restore must keep the block"
        );
        assert_eq!(
            std::fs::read(&primary).unwrap(),
            file::vault_file_bytes(&broken).unwrap()
        );
        std::fs::remove_dir_all(&corrupt).unwrap();
        vault_restore_snapshot_inner(&state, &dir.0).unwrap();
    }
}
