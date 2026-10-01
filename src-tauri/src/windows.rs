//! Main-window lifecycle: event handling, size floor, child windows.

use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::modules;
use crate::platform::windows::disable_windows_corner_rounding;

/// The user's preferences, read at use time. A failed data-dir resolve falls
/// back to the defaults rather than panicking inside a window-event handler.
fn prefs_now(app: &tauri::AppHandle) -> modules::prefs::Prefs {
    modules::vault::vault_dir(app)
        .map(|dir| modules::prefs::read(&dir))
        .unwrap_or_default()
}

/// Decide whether the process may exit now, shared by the window close, the
/// tray Quit and `RunEvent::ExitRequested`.
///
/// true: nothing would be lost, the caller may exit. false: a write is still
/// parked, the window is shown and focused, the webview is told to open its
/// confirmation and it owns the exit from here.
pub(crate) fn quit_or_confirm(app: &tauri::AppHandle) -> bool {
    if !app.state::<modules::vault::VaultState>().has_pending() {
        return true;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    let _ = app.emit(modules::events::QUIT_REQUESTED, ());
    false
}

/// Center a child window over the main window (so it follows the user across
/// monitors instead of landing on the primary display). No-op if either
/// window's geometry can't be read. Shared by the Settings and Debug windows.
fn recenter_over_main(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    if let Some(main) = app.get_webview_window("main") {
        if let (Ok(main_pos), Ok(main_size), Ok(win_size)) = (
            main.outer_position(),
            main.outer_size(),
            window.outer_size(),
        ) {
            let x = main_pos.x + (main_size.width as i32 - win_size.width as i32) / 2;
            let y = main_pos.y + (main_size.height as i32 - win_size.height as i32) / 2;
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
}

/// The logical size a window has to be resized to in order to respect `min`, or
/// `None` when it already does. Split out of [`enforce_configured_min_size`] so
/// the decision is testable without a live window - everything else in that
/// function is I/O against one.
fn min_size_correction(current: (f64, f64), min: (f64, f64)) -> Option<(f64, f64)> {
    if current.0 >= min.0 && current.1 >= min.1 {
        return None;
    }
    Some((current.0.max(min.0), current.1.max(min.1)))
}

/// Re-apply the configured size floor after `tauri-plugin-window-state` has
/// restored a saved size.
///
/// `minWidth`/`minHeight` from the config reach the window as TAO's
/// `min_inner_size`, and the OS enforces that for *user* resizing (on Windows
/// through `WM_GETMINMAXINFO`). A programmatic resize is not user resizing:
/// the window-state plugin restores a saved size with a bare
/// `set_size(PhysicalSize { .. })`
/// (`tauri-plugin-window-state` 2.4.1, `WindowExt::restore_state`) which lands
/// as a plain `SetWindowPos`, and that is not clamped against the tracking
/// size. So any profile carrying a window saved smaller than the floor comes
/// back below it, and raising the floor never reaches an existing user. The
/// change that makes a setting apply owns the setting actually applying, and a
/// second mechanism was quietly bypassing it.
/// GTK3 clamps the same call via its geometry hints so this is Windows-first,
/// but the fix is correct everywhere and is not gated on a platform.
///
/// The minimum is read back out of the merged runtime config rather than
/// restated here, so `tauri.conf.json` and the two platform files that must
/// echo it (enforced by `scripts/tauri-config-parity-verify.ts`) stay the only
/// place the number is written.
///
/// The early return below leaves a maximized or fullscreen window alone, so a
/// profile that quit maximized is restored over a below-floor size this
/// setup-time call cannot correct. The main window's `Resized` handler in
/// `run` calls this again, and the first time that window is sized normally -
/// its un-maximize - is when the floor lands.
pub(crate) fn enforce_configured_min_size(config: &tauri::Config, window: &tauri::WebviewWindow) {
    let Some(window_config) = config
        .app
        .windows
        .iter()
        .find(|w| w.label == window.label())
    else {
        return;
    };
    let (Some(min_width), Some(min_height)) = (window_config.min_width, window_config.min_height)
    else {
        return;
    };
    // A maximized or fullscreen window is not currently showing its restored
    // size, and `set_size` would drag it out of that state - TAO's Windows
    // `set_inner_size` clears the MAXIMIZED flag outright. Leave both alone and
    // let the floor apply the next time the window is sized normally.
    if window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
        return;
    }
    let (Ok(scale), Ok(size)) = (window.scale_factor(), window.inner_size()) else {
        return;
    };
    // The config states logical pixels; `inner_size` answers in physical ones.
    let size = size.to_logical::<f64>(scale);
    // `None` is a legitimately larger saved size - don't fight the plugin over it.
    if let Some((width, height)) =
        min_size_correction((size.width, size.height), (min_width, min_height))
    {
        let _ = window.set_size(tauri::LogicalSize::new(width, height));
    }
}

/// Open (or reveal) an owner-parented child window with our custom chrome.
/// Returns `Ok(None)` when an existing window was revealed, `Ok(Some(window))`
/// when a new one was built. Shared by the Settings and Debug windows.
pub(crate) fn open_or_reveal_child(
    app: &tauri::AppHandle,
    label: &str,
    url: String,
    title: &str,
    size: (f64, f64),
    min_size: (f64, f64),
) -> Result<Option<tauri::WebviewWindow>, String> {
    if let Some(window) = app.get_webview_window(label) {
        // Re-center over the main window so reopening follows the user
        // across displays.
        recenter_over_main(app, &window);
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(None);
    }

    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title(title)
        .inner_size(size.0, size.1)
        .min_inner_size(min_size.0, min_size.1)
        .resizable(true)
        .visible(false);

    // Owner-window relationship: keeps the child z-ordered above main without
    // pinning it above other apps. On Windows the OS auto-hides owned
    // windows when the owner minimizes, so the child follows main into the
    // taskbar instead of floating on the desktop.
    if let Some(main) = app.get_webview_window("main") {
        builder = builder.parent(&main).map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    // Linux/Windows render our own titlebar, so drop native chrome and
    // make the window transparent.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let builder = builder.decorations(false).transparent(true);

    let window = builder.build().map_err(|e| e.to_string())?;

    // Some Linux compositors (GNOME/Mutter with CSD-by-default) ignore the
    // builder-time decorations flag, so re-assert it after realize.
    #[cfg(target_os = "linux")]
    {
        let _ = window.set_decorations(false);
    }
    disable_windows_corner_rounding(&window);

    // Tauri's default placement lands at the primary monitor's center even
    // when main is on a secondary display; re-center over main so it follows
    // the user.
    recenter_over_main(app, &window);
    Ok(Some(window))
}

/// The main window's `on_window_event` handler, kept out of the builder chain
/// in `run`.
pub(crate) fn on_main_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    // Mirror main-window minimize/restore onto the settings child.
    // Owner-window semantics handle this on Windows; the explicit
    // mirroring below covers Linux/macOS and decoration-less
    // transparent windows where the OS auto-mirror is unreliable.
    // Only the main window's events drive the mirroring onto its
    // children (settings); ignore the children's own events.
    let label = window.label();
    if label != "main" {
        return;
    }
    let app = window.app_handle().clone();
    const CHILDREN: [&str; 1] = ["settings"];
    match event {
        // Close-to-tray hides the window and keeps the app (and the
        // browser extension's connection) alive. Otherwise the close
        // really quits - but a still-parked write asks first, and the
        // webview owns that dialog. `app.exit(0)` on the quit path
        // rather than letting the OS close the window: macOS would
        // otherwise keep running with no window at all.
        tauri::WindowEvent::CloseRequested { api, .. } => {
            // ALWAYS prevent the raw close, even on the quit path: the
            // exit request can still be vetoed (a mutation parks a seal
            // between the two calls), and by then the window is gone,
            // so the confirmation would go to nothing and the process
            // would linger windowless.
            api.prevent_close();
            if prefs_now(&app).close_to_tray {
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.hide();
                }
            } else if quit_or_confirm(&app) {
                app.exit(0);
            }
        }
        // On Windows, minimize arrives as a Resized event (Tauri 2 has
        // no Minimized variant). Sample the state and mirror it.
        tauri::WindowEvent::Resized(_) => {
            let Some(main) = app.get_webview_window("main") else {
                return;
            };
            let minimized = main.is_minimized().unwrap_or(false);
            // Locking on minimize drops the payload before the window
            // is hidden behind the tray; only an actually-unlocked
            // vault emits, so a second minimize is a no-op.
            if minimized
                && prefs_now(&app).lock_on_minimize
                && app.state::<modules::vault::VaultState>().lock_inner()
            {
                modules::vault::emit_locked(&app, modules::vault::LockReason::Minimize);
            }
            for child in CHILDREN {
                let Some(w) = app.get_webview_window(child) else {
                    continue;
                };
                if minimized {
                    let _ = w.minimize();
                } else if w.is_minimized().unwrap_or(false) {
                    let _ = w.unminimize();
                    let _ = w.show();
                }
            }
            // The size floor, for the one case the setup-time clamp has
            // to skip: a profile that quit maximized comes back maximized
            // over a below-floor restored size, and this is the first
            // event where that size is on screen - the un-maximize.
            // `enforce_configured_min_size` still leaves a maximized or
            // fullscreen window alone, and the OS already clamps a user
            // resize, so every other Resized is a no-op. Not while
            // minimized: the size read then is not the restored one, and
            // `set_size` would bring the window back up.
            if !minimized {
                enforce_configured_min_size(app.config(), &main);
            }
        }
        // Destroyed, not CloseRequested: the GUI can veto its own close
        // (the quit prompt), and taking the settings window down on a
        // close the user then cancels would be wrong.
        tauri::WindowEvent::Destroyed => {
            for child in CHILDREN {
                if let Some(w) = app.get_webview_window(child) {
                    let _ = w.close();
                }
            }
        }
        // The webview owns the pull rate limit; this only says the user
        // came back. The `label != "main"` guard above is what keeps
        // the settings window's own focus out of it.
        tauri::WindowEvent::Focused(true) => {
            let _ = window.emit(modules::events::SYNC_FOCUSED, ());
        }
        _ => {}
    }
}

#[cfg(test)]
mod min_size_tests {
    use super::min_size_correction;

    /// The whole point of the clamp: `tauri-plugin-window-state` restores a
    /// saved size with a bare `set_size`, which Windows does not check against
    /// the window's minimum, so a profile saved at the old 420x280 floor comes
    /// back at 420x280 under a 640x480 config and never sees the new floor.
    #[test]
    fn a_size_saved_below_the_floor_is_raised_to_it() {
        assert_eq!(
            min_size_correction((420.0, 280.0), (640.0, 480.0)),
            Some((640.0, 480.0))
        );
    }

    /// Only the short axis moves. A window saved wide and short keeps its width
    /// instead of being snapped back to the floor's aspect.
    #[test]
    fn only_the_axis_below_the_floor_moves() {
        assert_eq!(
            min_size_correction((900.0, 280.0), (640.0, 480.0)),
            Some((900.0, 480.0))
        );
        assert_eq!(
            min_size_correction((420.0, 700.0), (640.0, 480.0)),
            Some((640.0, 700.0))
        );
    }

    /// A saved size the user chose and that clears the floor must come back
    /// untouched - the clamp exists to raise a stale size, not to normalize one.
    #[test]
    fn a_larger_saved_size_is_left_alone() {
        assert_eq!(min_size_correction((1280.0, 800.0), (640.0, 480.0)), None);
    }

    /// Exactly at the floor is not below it, so no resize is issued at all.
    /// Without this the clamp would fire on every launch of a floor-sized
    /// window and fight the plugin for no reason.
    #[test]
    fn a_size_exactly_at_the_floor_is_left_alone() {
        assert_eq!(min_size_correction((640.0, 480.0), (640.0, 480.0)), None);
    }
}
