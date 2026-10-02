//! Tauri commands owned by the app shell rather than by a feature module:
//! opening the Settings window and leaving the process.

use tauri::{Emitter, Manager};

use crate::modules::vault::VaultState;
use crate::windows::{recenter_over_main, reveal};

#[tauri::command]
pub(crate) async fn open_settings_window(
    app: tauri::AppHandle,
    tab: Option<String>,
) -> Result<(), String> {
    let url_path = match tab.as_deref() {
        Some(t) if !t.is_empty() => format!("settings.html?tab={}", t),
        _ => "settings.html".to_string(),
    };
    let tab = tab.as_deref().filter(|s| !s.is_empty());

    // Reopening an existing window won't re-read the URL, so it gets the tab
    // pushed via event; a freshly built window carries the tab in `url_path`.
    if let Some(window) = app.get_webview_window("settings") {
        // Re-center over the main window so reopening follows the user
        // across displays.
        recenter_over_main(&app, &window);
        reveal(&window);
        if let Some(t) = tab {
            // emit() serializes via JSON, so no string-escape footgun.
            let _ = window.emit(crate::modules::events::SETTINGS_TAB, t);
        }
        return Ok(());
    }

    let mut builder =
        tauri::WebviewWindowBuilder::new(&app, "settings", tauri::WebviewUrl::App(url_path.into()))
            .title("Settings")
            .inner_size(880.0, 620.0)
            .min_inner_size(600.0, 480.0)
            .resizable(true)
            .visible(false);

    // Owner-window relationship: keeps the child z-ordered above main without
    // pinning it above other apps. On Windows the OS auto-hides owned windows
    // when the owner minimizes, so the child follows main into the taskbar
    // instead of floating on the desktop.
    if let Some(main) = app.get_webview_window("main") {
        builder = builder.parent(&main).map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    // Linux/Windows render our own titlebar, so drop native chrome and make the
    // window transparent.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let builder = builder.decorations(false).transparent(true);

    let window = builder.build().map_err(|e| e.to_string())?;

    // Some Linux compositors (GNOME/Mutter with CSD-by-default) ignore the
    // builder-time decorations flag, so re-assert it after realize.
    #[cfg(target_os = "linux")]
    {
        let _ = window.set_decorations(false);
    }
    crate::platform::windows::disable_windows_corner_rounding(&window);

    // Tauri's default placement lands at the primary monitor's center even when
    // main is on a secondary display; re-center over main so it follows the
    // user.
    recenter_over_main(&app, &window);
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
    use std::path::Path;

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
    /// `pub async fn`.
    fn sync_command_names() -> Vec<String> {
        let mut files = Vec::new();
        rs_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut found = Vec::new();
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
                            found.push(name.to_string());
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
    /// So sync commands are banned outright. A new one fails here, which is the
    /// point: the default for anything touching the filesystem, a subprocess, a
    /// socket or a pipe is `pub async fn` plus `spawn_blocking`.
    #[test]
    fn no_new_sync_tauri_commands() {
        let found = sync_command_names();
        assert!(
            found.is_empty(),
            "sync #[tauri::command]s found: {found:?}\n\
             On Windows these run on the WebView2 UI thread and will freeze the \
             window if they block. Make them `pub async fn` + \
             `tauri::async_runtime::spawn_blocking`."
        );
    }
}
