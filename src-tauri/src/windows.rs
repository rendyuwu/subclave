//! Main-window lifecycle: event handling, size floor, child windows.

use tauri::{Emitter, Manager};

use crate::modules;

/// The user's preferences, read at use time. A failed data-dir resolve falls
/// back to the defaults rather than panicking inside a window-event handler.
fn prefs_now(app: &tauri::AppHandle) -> modules::prefs::Prefs {
    modules::vault::vault_dir(app)
        .map(|dir| modules::prefs::read(&dir))
        .unwrap_or_default()
}

/// Reveal a window: show it if hidden, restore it if minimized, and focus it.
/// The close/quit prompt, the single-instance relaunch, the Settings window
/// reopen and the tray Open all end in this sequence.
pub(crate) fn reveal(window: &tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
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
        reveal(&window);
    }
    let _ = app.emit(modules::events::QUIT_REQUESTED, ());
    false
}

/// Center a child window over the main window (so it follows the user across
/// monitors instead of landing on the primary display). No-op if either
/// window's geometry can't be read. Shared by the Settings window build and its
/// reopen path.
pub(crate) fn recenter_over_main(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
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
    // A saved size that already clears the floor is left alone - the clamp
    // exists to raise a stale size, not to normalize a larger one.
    if size.width < min_width || size.height < min_height {
        let _ = window.set_size(tauri::LogicalSize::new(
            size.width.max(min_width),
            size.height.max(min_height),
        ));
    }
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
            if let Some(child) = app.get_webview_window("settings") {
                if minimized {
                    let _ = child.minimize();
                } else if child.is_minimized().unwrap_or(false) {
                    let _ = child.unminimize();
                    let _ = child.show();
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
            if let Some(child) = app.get_webview_window("settings") {
                let _ = child.close();
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
    /// The clamp `enforce_configured_min_size` applies to a restored size,
    /// mirrored here because the production path needs a live window. The
    /// `tauri-plugin-window-state` plugin restores a saved size with a bare
    /// `set_size`, which Windows does not check against the window minimum, so
    /// a profile saved at the old 420x280 floor comes back at 420x280 under a
    /// 640x480 config; the clamp raises each axis to the floor and leaves a
    /// larger saved size untouched. A table because the two axes are
    /// independent and "exactly at the floor" is the boundary that would
    /// otherwise regress to a resize on every launch.
    #[test]
    fn only_a_size_below_the_floor_is_raised() {
        let (fw, fh) = (640.0, 480.0);
        let corrected = |size: (f64, f64)| {
            (size.0 < fw || size.1 < fh).then(|| (size.0.max(fw), size.1.max(fh)))
        };
        let cases = [
            ((420.0, 280.0), Some((640.0, 480.0))),
            ((900.0, 280.0), Some((900.0, 480.0))),
            ((420.0, 700.0), Some((640.0, 700.0))),
            ((1280.0, 800.0), None),
            ((640.0, 480.0), None),
        ];
        for (size, want) in cases {
            assert_eq!(corrected(size), want, "size {size:?} against floor 640x480");
        }
    }
}
