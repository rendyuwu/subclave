//! Vault state and the Tauri command layer.
//!
//! [`VaultState`] holds the unlocked payload, the key, the KDF parameters, a
//! pending sealed write after a failed save, and the idle-lock deadline.
//!
//! Every command shell is `pub async fn` and does its blocking work inside
//! `tauri::async_runtime::spawn_blocking` (on Windows a sync command runs on
//! the WebView2 UI thread, so blocking there freezes the window; the
//! `no_new_sync_tauri_commands` test in `lib.rs` enforces this). `State<'_, _>`
//! is not `'static` and cannot move into that closure, so each shell clones
//! the `AppHandle`, resolves the managed state inside the closure, and drains
//! the auto-lock flag through the original handle afterwards. Inner functions
//! take `&VaultState` plus plain values, which is what keeps the whole core
//! testable without a Tauri runtime.

pub mod file;
pub mod kdf;
pub mod lock;
pub mod model;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::modules::lockext::LockExt as _;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use zeroize::Zeroizing;

use crate::modules::events;
use crate::modules::vault::file::{
    load_vault, open_file, save_vault, seal_payload, OpenVault, VaultFile, VAULT_FILE_NAME,
};
use crate::modules::vault::kdf::{derive_key, fresh_params, Argon2Params};
use crate::modules::vault::lock::{deadline_after, deadline_passed, DEFAULT_AUTO_LOCK_MINUTES};
use crate::modules::vault::model::{
    detail_of, normalize_tags, stamp_next, summary_of, version_changed_names, version_of, Entry,
    EntryDetail, EntryDraft, EntrySummary, EntryVersion, Group, GroupDraft, Tombstone,
    TombstoneKind, VaultPayload, BROWSER_ID, ROOT_ID, TRASH_ID,
};

pub const LOCKED_ERR: &str = "vault: locked";

/// Why the vault locked. The window-event reasons (minimize, tray) are added
/// when the window events that trigger them exist.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LockReason {
    Manual,
    Idle,
}

/// Everything held while the vault is unlocked.
pub struct Unlocked {
    pub(crate) payload: VaultPayload,
    key: Zeroizing<[u8; 32]>,
    kdf: Argon2Params,
}

/// Managed state. See the field docs for the invariants the save machinery
/// relies on.
pub struct VaultState {
    /// Payload, key and KDF while unlocked. Wiped, not merely dropped, on
    /// every lock path.
    inner: Mutex<Option<Unlocked>>,
    /// Sealed bytes parked after a failed write. Ciphertext only, so the
    /// retry works while locked.
    pending: Mutex<Option<VaultFile>>,
    /// Serializes write + pending updates, so a stale pending write can never
    /// land after a newer one: a newer `perform_save` always overwrites
    /// `pending`, and an older one cannot land after a newer write.
    save_lock: Mutex<()>,
    /// Idle deadline in boot-clock ms; `u64::MAX` = never.
    deadline_ms: AtomicU64,
    /// Set when `load_vault` had to fall back to the `.bak`: the primary is
    /// broken and must never be silently overwritten. Clearing this flag is
    /// the restore flow's job.
    save_blocked: AtomicBool,
    /// Set by an automatic lock (the idle tick, or an expired deadline hit
    /// inside [`VaultState::access`]); drained by the next command shell to
    /// emit the event.
    auto_lock_event: Mutex<Option<LockReason>>,
    /// The outcome of the last save attempt, recorded by [`perform_save`]
    /// and drained by the shells into `subclave:vault-save-failed`. Both the
    /// failure and the success are events: the banner shows the reason and
    /// clears on the next good write.
    save_event: Mutex<Option<SaveOutcome>>,
}

/// The outcome of one save attempt, for the `subclave:vault-save-failed`
/// event (`{ reason } | null`).
#[derive(Clone, Debug)]
pub(crate) enum SaveOutcome {
    Failed(String),
    Succeeded,
}

impl Default for VaultState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
            pending: Mutex::new(None),
            save_lock: Mutex::new(()),
            deadline_ms: AtomicU64::new(u64::MAX),
            save_blocked: AtomicBool::new(false),
            auto_lock_event: Mutex::new(None),
            save_event: Mutex::new(None),
        }
    }
}

impl VaultState {
    /// The only way an inner function touches `inner`. Checks the idle
    /// deadline first: a machine waking past the deadline locks before the
    /// first command, and `vault_touch` cannot resurrect an expired session.
    /// A deadline hit here is the one place the auto-lock flag is recorded;
    /// the tick thread reuses this path, so the event fires exactly once.
    pub fn access(&self) -> Result<std::sync::MutexGuard<'_, Option<Unlocked>>, String> {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some()
            && deadline_passed(lock::boot_now_ms(), self.deadline_ms.load(Ordering::SeqCst))
        {
            if let Some(unlocked) = guard.as_mut() {
                unlocked.payload.wipe();
            }
            *guard = None;
            // Refresh before the guard is released: the next reader must not
            // see the expired deadline.
            self.deadline_ms.store(
                deadline_after(DEFAULT_AUTO_LOCK_MINUTES, lock::boot_now_ms()),
                Ordering::SeqCst,
            );
            *self
                .auto_lock_event
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(LockReason::Idle);
            return Err(LOCKED_ERR.to_string());
        }
        Ok(guard)
    }

    /// Wipe the payload and drop the key. Records nothing: the caller owns
    /// the event (`vault_lock` emits Manual directly; the tick's expiry goes
    /// through `access`, which records Idle).
    pub(crate) fn lock_inner(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(unlocked) = guard.as_mut() {
            unlocked.payload.wipe();
        }
        *guard = None;
    }

    pub(crate) fn refresh_deadline(&self) {
        self.deadline_ms.store(
            deadline_after(DEFAULT_AUTO_LOCK_MINUTES, lock::boot_now_ms()),
            Ordering::SeqCst,
        );
    }

    /// The reason an automatic lock happened, if one has not been drained
    /// yet. Command shells call this after every result.
    pub(crate) fn take_auto_lock(&self) -> Option<LockReason> {
        self.auto_lock_event
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    /// The outcome of the last save attempt, if no shell has drained it yet.
    pub(crate) fn take_save_event(&self) -> Option<SaveOutcome> {
        self.save_event
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}

// ---- Save machinery ----
//
// Lock order, everywhere: save_lock, then inner, then pending. Nothing takes
// them in reverse, so no deadlock.
//
// `pending` holds the newest seal whose write failed. A SUCCESSFUL write
// clears it: a parked seal must never outlive a save that landed, or the
// retry would overwrite newer data (including a master password change) with
// older ciphertext.

/// Seal the current payload and write it, after any payload mutation. On
/// failure the seal is parked in `pending` and the error propagates; the
/// in-memory payload is already mutated, which is the contract the retry
/// path relies on.
fn commit(state: &VaultState, dir: &Path) -> Result<(), String> {
    // save_lock is taken BEFORE inner so the seal and the write are one
    // ordered unit: an older seal can never land after a newer one. Sealed
    // first, then perform_save re-takes the lock with the fresh seal.
    let sealed = {
        let _held = state.save_lock.lock_or_recover();
        let guard = state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        seal_payload(&unlocked.payload, &unlocked.key, &unlocked.kdf)?
    };
    perform_save(state, dir, sealed)
}

/// Write `file` under save_lock. On success any parked seal is dropped (the
/// disk now holds something at least as new); on failure this file is parked
/// as the newest seal. Used by the retry and the shells that do not seal
/// under the lock themselves.
fn perform_save(state: &VaultState, dir: &Path, file: VaultFile) -> Result<(), String> {
    let _held = state.save_lock.lock_or_recover();
    if state.save_blocked.load(Ordering::SeqCst) {
        return Err("vault: restore the snapshot before saving".to_string());
    }
    match save_vault(dir, &file) {
        Ok(()) => {
            *state.pending.lock_or_recover() = None;
            *state.save_event.lock_or_recover() = Some(SaveOutcome::Succeeded);
            Ok(())
        }
        Err(e) => {
            *state.pending.lock_or_recover() = Some(file);
            *state.save_event.lock_or_recover() = Some(SaveOutcome::Failed(e.clone()));
            Err(e)
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The app data dir for a command. Tauri's resolver applies the `.dev`
/// suffix from `tauri.dev.conf.json`.
fn vault_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|e| format!("vault: {e}"))
}

/// Every ancestor chain step of `id`, for the cycle refusal on reparent.
fn is_descendant(payload: &VaultPayload, ancestor: &str, candidate: &str) -> bool {
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

// ---- Events ----

/// Drain the auto-lock flag into the `subclave:vault-locked` event. Called by
/// every command shell after the inner result is in.
pub(crate) fn drain_auto_lock(app: &AppHandle) {
    let state = app.state::<VaultState>();
    if let Some(reason) = state.take_auto_lock() {
        emit_locked(app, reason);
    }
}

fn emit_locked(app: &AppHandle, reason: LockReason) {
    let _ = app.emit(
        events::VAULT_LOCKED,
        serde_json::json!({ "reason": reason }),
    );
}

/// Only called on success; a failed mutation emits `vault-save-failed`
/// instead.
fn emit_changed(app: &AppHandle, ids: &[String]) {
    let _ = app.emit(
        events::VAULT_CHANGED,
        serde_json::json!({ "ids": ids, "origin": "local" }),
    );
}

/// `{ reason } | null`, per the event table: `None` is the success signal a
/// retry sends to clear the banner.
fn emit_save_failed(app: &AppHandle, reason: Option<&str>) {
    let _ = app.emit(
        events::VAULT_SAVE_FAILED,
        match reason {
            Some(r) => serde_json::json!(r),
            None => serde_json::Value::Null,
        },
    );
}

/// Drain the last save outcome into `subclave:vault-save-failed`. Every
/// command shell calls this after its inner result, and the tick after its
/// retry, so the banner tracks every save attempt whatever path made it.
pub(crate) fn drain_save_event(app: &AppHandle) {
    let state = app.state::<VaultState>();
    match state.take_save_event() {
        Some(SaveOutcome::Failed(reason)) => emit_save_failed(app, Some(&reason)),
        Some(SaveOutcome::Succeeded) => emit_save_failed(app, None),
        None => {}
    }
}

// ---- Command shells ----
//
// Shape of every shell: resolve the dir, clone the handle for the blocking
// task, run the inner function, drain the auto-lock flag, return.

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    pub exists: bool,
    pub locked: bool,
    pub save_pending: bool,
}

#[tauri::command]
pub async fn vault_status(app: AppHandle) -> Result<VaultStatus, String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_status_inner(&task_app.state::<VaultState>(), &dir)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    result
}

fn vault_status_inner(state: &VaultState, dir: &Path) -> Result<VaultStatus, String> {
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
    Ok(VaultStatus {
        exists,
        locked,
        save_pending: state.pending.lock_or_recover().is_some(),
    })
}

#[tauri::command]
pub async fn vault_create(app: AppHandle, master_password: String) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_create_inner(&task_app.state::<VaultState>(), &dir, &master_password)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    result
}

fn vault_create_inner(state: &VaultState, dir: &Path, master_password: &str) -> Result<(), String> {
    if dir.join(VAULT_FILE_NAME).exists() || dir.join(format!("{VAULT_FILE_NAME}.bak")).exists() {
        return Err("vault: a vault file already exists".to_string());
    }
    if master_password.chars().count() < 8 {
        return Err("vault: master password must be at least 8 characters".to_string());
    }
    let now = now_ms();
    let payload = VaultPayload {
        entries: vec![],
        groups: vec![
            Group {
                id: ROOT_ID.into(),
                parent_id: None,
                name: "Root".into(),
                icon: None,
                color: None,
                created_at: now,
                updated_at: now,
            },
            Group {
                id: TRASH_ID.into(),
                parent_id: None,
                name: "Trash".into(),
                icon: None,
                color: None,
                created_at: now,
                updated_at: now,
            },
            Group {
                id: BROWSER_ID.into(),
                parent_id: Some(ROOT_ID.into()),
                name: "Browser".into(),
                icon: None,
                color: None,
                created_at: now,
                updated_at: now,
            },
        ],
        tombstones: vec![],
    };
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
        *state.pending.lock_or_recover() = None;
    }
    // Refresh BEFORE installing: a concurrent access() must not see the
    // previous session's expired deadline while the new one is installed.
    state.refresh_deadline();
    *state.inner.lock_or_recover() = Some(Unlocked { payload, key, kdf });
    Ok(())
}

#[tauri::command]
pub async fn vault_unlock(app: AppHandle, master_password: String) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_unlock_inner(&task_app.state::<VaultState>(), &dir, &master_password)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    result
}

fn vault_unlock_inner(state: &VaultState, dir: &Path, master_password: &str) -> Result<(), String> {
    let (file, from_bak) = load_vault(dir)?;
    state.save_blocked.store(from_bak, Ordering::SeqCst);
    let opened: OpenVault = open_file(&file, master_password)?;
    // A parked seal is newer than the disk copy it failed to replace (the
    // P0 fix clears it on every successful save, so it can only still exist
    // if the write keeps failing). It is sealed under the same key and kdf
    // this file just derived, so opening it shows the user their edits.
    let payload = match state.pending.lock_or_recover().clone() {
        Some(pending) => open_file(&pending, master_password)
            .map(|o| {
                // The parked seal wins, and save_blocked says the disk
                // copy is the broken one.
                state.save_blocked.store(true, Ordering::SeqCst);
                o.payload
            })
            .unwrap_or_else(|_| opened.payload),
        None => opened.payload,
    };
    state.refresh_deadline();
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
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_touch_inner(&task_app.state::<VaultState>())
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    result
}

fn vault_touch_inner(state: &VaultState) -> Result<(), String> {
    // Through access so an expired deadline locks instead of extending. The
    // guard is dropped immediately: touch only extends the deadline.
    drop(state.access()?);
    state.refresh_deadline();
    Ok(())
}

#[tauri::command]
pub async fn vault_change_master(
    app: AppHandle,
    current: String,
    next: String,
) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_change_master_inner(&task_app.state::<VaultState>(), &dir, &current, &next)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    result
}

fn vault_change_master_inner(
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
    *state.pending.lock_or_recover() = None;
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
/// it, because it already holds the newest seal. The read, write and clear
/// all happen under save_lock, so a commit racing in between cannot be
/// overwritten by the retry or lost by the clear.
pub(crate) fn vault_retry_save_inner(state: &VaultState, dir: &Path) -> Result<(), String> {
    let file = {
        let _held = state.save_lock.lock_or_recover();
        state.pending.lock_or_recover().take()
    };
    if let Some(file) = file {
        // perform_save re-parks it on failure, so the retry state is kept.
        perform_save(state, dir, file)
    } else {
        Ok(())
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
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_list_inner(&task_app.state::<VaultState>())
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    result
}

fn vault_list_inner(state: &VaultState) -> Result<VaultList, String> {
    let guard = state.access()?;
    let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
    // Unsorted: the locale compare is a UI concern.
    Ok(VaultList {
        entries: unlocked.payload.entries.iter().map(summary_of).collect(),
        groups: unlocked.payload.groups.clone(),
    })
}

#[tauri::command]
pub async fn vault_entry_get(app: AppHandle, id: String) -> Result<EntryDetail, String> {
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_entry_get_inner(&task_app.state::<VaultState>(), &id)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    result
}

fn vault_entry_get_inner(state: &VaultState, id: &str) -> Result<EntryDetail, String> {
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
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_entry_reveal_inner(&task_app.state::<VaultState>(), &id, &field)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    result
}

fn vault_entry_reveal_inner(state: &VaultState, id: &str, field: &str) -> Result<String, String> {
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

#[tauri::command]
pub async fn vault_entry_upsert(app: AppHandle, draft: EntryDraft) -> Result<EntrySummary, String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_entry_upsert_inner(&task_app.state::<VaultState>(), &dir, draft)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    if let Ok(summary) = &result {
        emit_changed(&app, std::slice::from_ref(&summary.id));
    }
    result
}

fn vault_entry_upsert_inner(
    state: &VaultState,
    dir: &Path,
    draft: EntryDraft,
) -> Result<EntrySummary, String> {
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
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_entry_move_inner(&task_app.state::<VaultState>(), &dir, ids, group_id)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    if result.is_ok() {
        emit_changed(&app, &[]);
    }
    result
}

fn vault_entry_move_inner(
    state: &VaultState,
    dir: &Path,
    ids: Vec<String>,
    group_id: String,
) -> Result<(), String> {
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
    }
    drop(guard);
    commit(state, dir)
}

#[tauri::command]
pub async fn vault_entry_trash(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_entry_trash_inner(&task_app.state::<VaultState>(), &dir, ids)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    if result.is_ok() {
        emit_changed(&app, &[]);
    }
    result
}

fn vault_entry_trash_inner(state: &VaultState, dir: &Path, ids: Vec<String>) -> Result<(), String> {
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
    }
    drop(guard);
    commit(state, dir)
}

#[tauri::command]
pub async fn vault_entry_restore(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_entry_restore_inner(&task_app.state::<VaultState>(), &dir, ids)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    if result.is_ok() {
        emit_changed(&app, &[]);
    }
    result
}

fn vault_entry_restore_inner(
    state: &VaultState,
    dir: &Path,
    ids: Vec<String>,
) -> Result<(), String> {
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
    }
    drop(guard);
    commit(state, dir)
}

#[tauri::command]
pub async fn vault_entry_delete(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_entry_delete_inner(&task_app.state::<VaultState>(), &dir, ids)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    if result.is_ok() {
        emit_changed(&app, &[]);
    }
    result
}

fn vault_entry_delete_inner(
    state: &VaultState,
    dir: &Path,
    ids: Vec<String>,
) -> Result<(), String> {
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
    let task_app = app.clone();
    let emit_id = id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_entry_restore_version_inner(&task_app.state::<VaultState>(), &dir, &id, updated_at)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    if result.is_ok() {
        emit_changed(&app, std::slice::from_ref(&emit_id));
    }
    result
}

fn vault_entry_restore_version_inner(
    state: &VaultState,
    dir: &Path,
    id: &str,
    updated_at: u64,
) -> Result<EntrySummary, String> {
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
    drop(guard);
    commit(state, dir)?;
    Ok(summary)
}

#[tauri::command]
pub async fn vault_group_upsert(app: AppHandle, group: GroupDraft) -> Result<Group, String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_group_upsert_inner(&task_app.state::<VaultState>(), &dir, group)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    if result.is_ok() {
        emit_changed(&app, &[]);
    }
    result
}

fn vault_group_upsert_inner(
    state: &VaultState,
    dir: &Path,
    draft: GroupDraft,
) -> Result<Group, String> {
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
    drop(guard);
    commit(state, dir)?;
    Ok(group)
}

#[tauri::command]
pub async fn vault_group_delete(app: AppHandle, id: String) -> Result<(), String> {
    let dir = vault_dir(&app)?;
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        vault_group_delete_inner(&task_app.state::<VaultState>(), &dir, id)
    })
    .await
    .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(&app);
    drain_auto_lock(&app);
    if result.is_ok() {
        emit_changed(&app, &[]);
    }
    result
}

fn vault_group_delete_inner(state: &VaultState, dir: &Path, id: String) -> Result<(), String> {
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

    /// A private directory under the system temp dir, removed on drop.
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subclave-vault-flow-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id(),
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn draft(id: Option<String>, title: &str) -> EntryDraft {
        EntryDraft {
            id,
            group_id: ROOT_ID.into(),
            title: title.into(),
            username: "user".into(),
            password: Some("pw-1".into()),
            urls: vec![],
            notes: String::new(),
            totp: None,
            custom_fields: vec![],
            tags: vec![" tag ".into(), "TAG".into()],
            icon: None,
            color: None,
            favorite: false,
            expires_at: None,
        }
    }

    fn group_draft(id: Option<String>, parent: Option<String>, name: &str) -> GroupDraft {
        GroupDraft {
            id,
            parent_id: parent,
            name: name.into(),
            icon: None,
            color: None,
        }
    }

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
        assert_eq!(vault_touch_inner(&state5).unwrap_err(), LOCKED_ERR);
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
        assert!(vault_touch_inner(&state5).is_ok());

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

    /// A locked vault refuses the copy before any clipboard round trip, so
    /// this is testable headless; the round trip itself needs a display.
    #[test]
    fn clip_copy_field_lock_path() {
        let state = VaultState::default();
        assert_eq!(
            crate::modules::clipboard::clip_copy_field_inner(&state, "any", "password")
                .err()
                .unwrap(),
            LOCKED_ERR
        );
    }

    /// The stale-seal regression: a successful save must clear a parked
    /// seal, or the retry lands an older ciphertext over newer data (a lost
    /// edit, or a reverted master password change).
    #[test]
    fn a_successful_save_clears_a_parked_seal() {
        let dir = TempDir::new("pending");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let summary = vault_entry_upsert_inner(&state, &dir.0, draft(None, "One")).unwrap();
        let _ = summary;

        // Park a seal by pointing the vault PRIMARY at a directory (the .bak
        // keeps the pre-failure state, so the primary is what fails).
        let primary = dir.0.join(file::VAULT_FILE_NAME);
        let stash = dir.0.join("stash.json");
        std::fs::rename(&primary, &stash).unwrap();
        std::fs::create_dir_all(&primary).unwrap();
        // The .bak is the last complete copy: check what the in-memory list
        // looks like right after the failure.
        let titles_now: Vec<String> = vault_list_inner(&state)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.title)
            .collect();
        eprintln!("titles after failed save: {titles_now:?}");
        let err = vault_entry_upsert_inner(&state, &dir.0, draft(None, "Two")).unwrap_err();
        assert!(
            state.pending.lock_or_recover().is_some(),
            "upsert must park: {err}"
        );

        // Remove the blocker, edit again: this save succeeds and must clear
        // the parked seal.
        std::fs::remove_dir_all(&primary).unwrap();
        std::fs::rename(&stash, &primary).unwrap();
        vault_entry_upsert_inner(&state, &dir.0, draft(None, "Three")).unwrap();
        assert!(
            state.pending.lock_or_recover().is_none(),
            "a successful save must clear the parked seal"
        );

        // The retry is then a no-op: nothing is parked, the disk keeps
        // One+Three, and "Two" exists only in the in-memory payload.
        vault_retry_save_inner(&state, &dir.0).unwrap();
        let probe = VaultState::default();
        vault_unlock_inner(&probe, &dir.0, "master-pw").unwrap();
        let titles: Vec<String> = vault_list_inner(&probe)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.title)
            .collect();
        // "Three" on disk proves the newest in-memory state landed; the
        // parked One+Two seal was cleared by that same save, and the retry
        // above was a no-op (nothing was parked). "Two" is in the list only
        // because the successful save sealed the whole in-memory payload.
        assert!(titles.contains(&"Three".to_string()));
        assert!(titles.contains(&"Two".to_string()));
    }

    /// A failed change-master must not park the new-password seal for the
    /// retry to land behind the user's back.
    #[test]
    fn a_failed_change_master_parks_nothing() {
        let dir = TempDir::new("changefail");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        // Block the write.
        let primary = dir.0.join(file::VAULT_FILE_NAME);
        let stash = dir.0.join("stash.json");
        std::fs::rename(&primary, &stash).unwrap();
        std::fs::create_dir_all(&primary).unwrap();
        let err =
            vault_change_master_inner(&state, &dir.0, "master-pw", "new-master-pw").unwrap_err();
        assert!(
            !err.contains("at least 8"),
            "the failure must come from the write, not validation: {err}"
        );
        assert!(
            state.pending.lock_or_recover().is_none(),
            "a failed change must park no seal"
        );
        // Un-block: the disk must still open with the OLD password.
        std::fs::remove_dir_all(&primary).unwrap();
        std::fs::rename(&stash, &primary).unwrap();
        let probe = VaultState::default();
        vault_unlock_inner(&probe, &dir.0, "master-pw").unwrap();
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

    /// A manual lock emits exactly one vault-locked: the flag records only
    /// idle locks, which go through access().
    #[test]
    fn manual_lock_records_no_auto_lock_flag() {
        let state = VaultState::default();
        state.lock_inner();
        assert_eq!(state.take_auto_lock(), None);
        assert_eq!(state.take_auto_lock(), None, "the flag must not re-fire");
    }
}
