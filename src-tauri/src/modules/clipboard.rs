//! Host-process clipboard for secret copies.
//!
//! The webview API covers ordinary writes; only secret copies route through
//! here, because two things belong together: the copy is marked concealed so
//! host clipboard history tools skip it, and the auto-clear timer can later
//! compare-and-clear the same value.
//!
//! The handle is process-global and load-bearing on Linux: an X11 selection
//! is owned by a live connection, not stored in a server, so a per-call
//! handle that writes and then drops loses the data outright. A long-lived
//! handle serves the selection for as long as the app runs.
//!
//! Every function here is a synchronous round trip to whichever process owns
//! the selection, so every one must run on a blocking thread.

use std::sync::{LazyLock, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::modules::lockext::LockExt as _;
use crate::modules::vault::{resolve_field, VaultState};

// The initializer is fallible (no X display yet, a Wayland compositor
// mid-restart), so this stays an Option inside the LazyLock: a failed init
// must not poison every later call with the same error.
static CLIPBOARD: LazyLock<Mutex<Option<arboard::Clipboard>>> =
    LazyLock::new(|| Mutex::new(arboard::Clipboard::new().ok()));

/// How long a copied secret stays on the clipboard until the settings file
/// carries `clipboardClearSeconds` (its writer is the Settings UI).
pub(crate) const DEFAULT_CLIPBOARD_CLEAR_SECONDS: u64 = 30;

/// The one handle for the app's lifetime.
///
/// BLOCKING - call only from a blocking thread.
fn handle() -> Result<&'static Mutex<Option<arboard::Clipboard>>, String> {
    if CLIPBOARD.lock_or_recover().is_none() {
        // Retry once per call: a transient failure (no display yet) must not
        // stick for the rest of the process.
        let fresh = arboard::Clipboard::new().map_err(|e| e.to_string())?;
        *CLIPBOARD.lock_or_recover() = Some(fresh);
    }
    Ok(&CLIPBOARD)
}

/// BLOCKING - call only from a blocking thread.
pub(crate) fn read_text() -> Result<String, String> {
    let mut guard = handle()?.lock_or_recover();
    let clipboard = guard
        .as_mut()
        .ok_or_else(|| "clipboard: no clipboard handle".to_string())?;
    clipboard.get_text().map_err(|e| e.to_string())
}

/// BLOCKING - call only from a blocking thread.
///
/// The data is marked so host clipboard managers skip it. On Windows this is
/// `exclude_from_monitoring` alone: arboard's own docs say not to pair it
/// with `exclude_from_cloud` or `exclude_from_history`, and it already
/// subsumes both. macOS has no OS-level flag for this, only the
/// `org.nspasteboard.ConcealedType` community convention, which is what
/// arboard sets there; the Linux flag is what KDE reads as
/// `x-kde-passwordManagerHint=secret`.
pub(crate) fn write_text(text: &str) -> Result<(), String> {
    let clipboard = handle()?;
    let mut clipboard = clipboard.lock_or_recover();
    let clipboard = clipboard
        .as_mut()
        .ok_or_else(|| "clipboard: no clipboard handle".to_string())?;
    let set = conceal(clipboard.set());
    set.text(text).map_err(|e| e.to_string())
}

fn conceal(set: arboard::Set<'_>) -> arboard::Set<'_> {
    #[cfg(target_os = "linux")]
    {
        use arboard::SetExtLinux as _;
        set.exclude_from_history()
    }
    #[cfg(target_os = "windows")]
    {
        use arboard::SetExtWindows as _;
        set.exclude_from_monitoring()
    }
    #[cfg(target_os = "macos")]
    {
        use arboard::SetExtApple as _;
        set.exclude_from_history()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    set
}

/// A copied secret waiting for its timer.
struct PendingClear {
    value: String,
    /// Boot-clock ms; the same clock the idle lock uses, so a suspended
    /// machine does not skip the clear.
    clears_at: u64,
}

static PENDING_CLEAR: LazyLock<Mutex<Option<PendingClear>>> = LazyLock::new(|| Mutex::new(None));

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearResult {
    /// Epoch ms when the clipboard clears; `None` = never.
    pub clears_at: Option<u64>,
}

/// Copy one entry field, concealed, with the auto-clear timer armed. `totp`
/// copies the current CODE; every other field matches `vault_entry_reveal`.
#[tauri::command]
pub async fn clip_copy_field(
    app: AppHandle,
    id: String,
    field: String,
) -> Result<ClearResult, String> {
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = task_app.state::<VaultState>();
        clip_copy_field_inner(&state, &id, &field)
    })
    .await
    .map_err(|e| format!("clipboard: task failed: {e}"))?;
    crate::modules::vault::drain_auto_lock(&app);
    result
}

pub(crate) fn clip_copy_field_inner(
    state: &VaultState,
    id: &str,
    field: &str,
) -> Result<ClearResult, String> {
    // The entry is cloned out so the state lock is never held across the
    // blocking clipboard round trip.
    let entry = {
        let guard = state.access()?;
        let unlocked = guard
            .as_ref()
            .ok_or_else(|| crate::modules::vault::LOCKED_ERR.to_string())?;
        unlocked
            .payload
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| "vault: no such entry".to_string())?
            .clone()
    };
    let value = resolve_field(&entry, field, true)?;
    write_text(&value)?;
    let secs = DEFAULT_CLIPBOARD_CLEAR_SECONDS;
    if secs == 0 {
        return Ok(ClearResult { clears_at: None });
    }
    let clears_at = crate::modules::vault::lock::boot_now_ms() + secs * 1000;
    // A copy that arrived while a timer was pending replaces the slot: the
    // newest copy is the one the clear must match.
    *PENDING_CLEAR.lock_or_recover() = Some(PendingClear {
        value: value.clone(),
        clears_at,
    });
    // The returned epoch ms comes from the wall clock, fresh here: the timer
    // comparison itself stays on the boot clock.
    let now_epoch_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok(ClearResult {
        clears_at: Some(now_epoch_ms + secs * 1000),
    })
}

/// Called every second by the tick thread. When the timer has run out, the
/// clipboard is cleared ONLY if it still holds the copied value.
pub(crate) fn clear_tick() {
    let now = crate::modules::vault::lock::boot_now_ms();
    // The due check and the take are ONE lock acquisition: a copy landing
    // between a separate check and take would otherwise be cleared the
    // instant it was written, without its own timer ever running.
    let mut slot = PENDING_CLEAR.lock_or_recover();
    let due = matches!(slot.as_ref(), Some(pending) if pending.clears_at <= now);
    let pending = if due { slot.take() } else { None };
    drop(slot);
    if let Some(pending) = pending {
        compare_and_clear(&pending.value);
    }
}

/// Same compare-then-clear, at exit. Called from `RunEvent::Exit`.
pub(crate) fn clear_on_exit() {
    let slot = PENDING_CLEAR.lock_or_recover().take();
    if let Some(pending) = slot {
        compare_and_clear(&pending.value);
    }
}

/// BLOCKING - call only from a blocking thread. Clear only when the clipboard
/// still holds the copied value: the user may have copied something else in
/// the meantime, and that must survive.
fn compare_and_clear(value: &str) {
    if read_text().ok().as_deref() != Some(value) {
        return;
    }
    let Ok(clipboard) = handle() else { return };
    let mut guard = clipboard.lock_or_recover();
    if let Some(clipboard) = guard.as_mut() {
        let _ = clipboard.clear();
    }
}
