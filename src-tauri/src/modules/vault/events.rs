//! The vault's outbound events: the locked notification, the changed-id
//! broadcast, the save-failed banner, and the blocking-task helper every
//! command shell drains them through.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::modules::events;
use crate::modules::vault::state::{SaveOutcome, VaultState};

/// Why the vault locked. The minimize and tray reasons come from the window
/// and tray event handlers; both emit only when the lock actually dropped an
/// unlocked payload.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LockReason {
    Manual,
    Idle,
    Minimize,
    Tray,
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

/// Emit the locked event. `pub(crate)` so the tray and the window handlers can
/// reuse the same emission the command shells use.
pub(crate) fn emit_locked(app: &AppHandle, reason: LockReason) {
    // Every lock reason (idle, manual, minimize, tray) drops the in-memory
    // sync session; the next unlock reopens it from the stored root key.
    crate::modules::sync::session_closed(app);
    // The browser channel carries credentials inside the payload, so every
    // live connection is dropped with it; the extension reconnects and finds
    // the locked state on its next request.
    crate::modules::browser::close_all(app);
    let _ = app.emit(
        events::VAULT_LOCKED,
        serde_json::json!({ "reason": reason }),
    );
}

/// Only called on success; a failed mutation emits `vault-save-failed`
/// instead. `origin` is `"local"` for every vault-side mutation and `"sync"`
/// for a pull that landed records.
pub(crate) fn emit_changed(app: &AppHandle, ids: &[String], origin: &str) {
    let _ = app.emit(
        events::VAULT_CHANGED,
        serde_json::json!({ "ids": ids, "origin": origin }),
    );
}

/// Drain the last save outcome into `subclave:vault-save-failed`: `{ reason }`
/// on failure, `null` on success (the retry's clear signal). Every command
/// shell calls this after its inner result, and the tick after its retry, so
/// the banner tracks every save attempt whatever path made it.
pub(crate) fn drain_save_event(app: &AppHandle) {
    let state = app.state::<VaultState>();
    let reason = match state.take_save_event() {
        Some(SaveOutcome::Failed(reason)) => serde_json::json!(reason),
        Some(SaveOutcome::Succeeded) => serde_json::Value::Null,
        None => return,
    };
    let _ = app.emit(events::VAULT_SAVE_FAILED, reason);
}

/// Run `f` on the blocking pool with the managed [`VaultState`] resolved
/// inside the closure (`State<'_, _>` is not `'static` and cannot move into
/// it), then drain the save banner and the auto-lock flag. This is the shape
/// every command shell shares.
pub(crate) async fn run_blocking<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&VaultState) -> Result<T, String> + Send + 'static,
{
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || f(&task_app.state::<VaultState>()))
        .await
        .map_err(|e| format!("vault: task failed: {e}"))?;
    drain_save_event(app);
    drain_auto_lock(app);
    result
}
