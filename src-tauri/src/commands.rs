//! Tauri commands owned by the app shell rather than by a feature module:
//! opening the Settings window and leaving the process.

use tauri::{Emitter, Manager};

use crate::modules::vault::VaultState;
use crate::windows::open_or_reveal_child;

#[tauri::command]
pub(crate) async fn open_settings_window(
    app: tauri::AppHandle,
    tab: Option<String>,
) -> Result<(), String> {
    let url_path = match tab.as_deref() {
        Some(t) if !t.is_empty() => format!("settings.html?tab={}", t),
        _ => "settings.html".to_string(),
    };

    // Freshly built windows carry the tab in `url_path`; a revealed existing
    // window won't re-read the URL, so it gets the tab pushed via event.
    if open_or_reveal_child(
        &app,
        "settings",
        url_path,
        "Settings",
        (880.0, 620.0),
        (600.0, 480.0),
    )?
    .is_none()
    {
        if let Some(t) = tab.as_deref().filter(|s| !s.is_empty()) {
            if let Some(window) = app.get_webview_window("settings") {
                // emit() serializes via JSON, so no string-escape footgun.
                let _ = window.emit(crate::modules::events::SETTINGS_TAB, t);
            }
        }
    }
    Ok(())
}

/// Leave the process after the user confirmed the quit prompt. The parked seal
/// is dropped first: it would otherwise be retried by the save tick and the
/// exit request would be vetoed, so the confirmation would come back forever.
/// Anything unsaved is lost, which is exactly what "Quit anyway" agreed to.
#[tauri::command]
pub(crate) async fn quit_subclave(app: tauri::AppHandle) -> Result<(), String> {
    app.state::<VaultState>().drop_pending();
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod ui_thread_guard {
    use std::collections::BTreeSet;
    use std::path::Path;

    /// Sync `#[tauri::command]`s that are allowed to exist, each because it does
    /// no blocking work.
    ///
    /// Every remaining command is `async`, so the list is empty. A new sync
    /// command needs an entry here with the reason it cannot block - adding one
    /// should be a deliberate act, not a way around `spawn_blocking`.
    const ALLOWED_SYNC_COMMANDS: &[&str] = &[];

    fn rs_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                rs_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    /// Names of every `#[tauri::command]` declared as `pub fn` rather than
    /// `pub async fn`. A `BTreeSet` because `#[cfg]`-gated commands are declared
    /// once per platform and would otherwise count twice.
    fn sync_command_names() -> BTreeSet<String> {
        let mut files = Vec::new();
        rs_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut found = BTreeSet::new();
        for path in files {
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            let lines: Vec<&str> = src.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if line.trim() != "#[tauri::command]" {
                    continue;
                }
                // Attribute macros may sit between the marker and the fn, and a
                // long signature can push `pub fn` a few lines down.
                for probe in lines.iter().skip(i + 1).take(8) {
                    let t = probe.trim_start();
                    if t.starts_with("pub async fn ") {
                        break;
                    }
                    if let Some(rest) = t.strip_prefix("pub fn ") {
                        let name = rest.split('(').next().unwrap_or("").trim();
                        if !name.is_empty() {
                            found.insert(name.to_string());
                        }
                        break;
                    }
                }
            }
        }
        found
    }

    /// On Windows a sync `#[tauri::command]` runs on the WebView2 UI thread, so
    /// blocking inside one freezes the whole window. That has shipped THREE
    /// times: git decorations (v0.3.50), `pty_write` (v0.3.98), and `fs_read_dir`
    /// on the lock-screen resume path. Each time the fix was to move that one
    /// command off the thread, and each time the next one was added without
    /// anybody noticing the rule.
    ///
    /// So the list is pinned. A new sync command fails here, which is the point:
    /// adding one should be a deliberate act with a reason, and the default for
    /// anything touching the filesystem, a subprocess, a socket or a pipe is
    /// `pub async fn` plus `spawn_blocking`.
    #[test]
    fn no_new_sync_tauri_commands() {
        let found = sync_command_names();
        let allowed: BTreeSet<String> = ALLOWED_SYNC_COMMANDS
            .iter()
            .map(|s| s.to_string())
            .collect();

        let added: Vec<&String> = found.difference(&allowed).collect();
        assert!(
            added.is_empty(),
            "new sync #[tauri::command]s found: {added:?}\n\
             On Windows these run on the WebView2 UI thread and will freeze the \
             window if they block. Make them `pub async fn` + \
             `tauri::async_runtime::spawn_blocking`, or add them to \
             ALLOWED_SYNC_COMMANDS with a reason why they cannot block."
        );

        // The other direction matters too: a command that got fixed should be
        // struck off, so the list keeps describing reality instead of rotting.
        let stale: Vec<&String> = allowed.difference(&found).collect();
        assert!(
            stale.is_empty(),
            "ALLOWED_SYNC_COMMANDS lists commands that are no longer sync: {stale:?}\n\
             Remove them from the list."
        );
    }
}
