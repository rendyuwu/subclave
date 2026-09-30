//! System tray icon: Open, Lock and Quit.
//!
//! The tray is the app's presence while the window is hidden (close-to-tray)
//! and the lock affordance when the vault is unlocked without the window in
//! front. Menu clicks are the portable path; a left click on the icon also
//! shows the window where the platform delivers that event (Linux AppIndicator
//! does not, so "Open Subclave" is the path there).

use crate::modules::vault::{emit_locked, LockReason, VaultState};
use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

/// Build the tray icon and its menu. Called from `setup`; the caller logs a
/// failure and carries on.
///
/// Only a missing tray HOST degrades to "no tray, still an app". A host missing
/// the AppIndicator shared library does not: the GTK backend aborts inside
/// `libappindicator-sys` when it can find neither `libayatana-appindicator3`
/// nor `libappindicator3`, and the release profile is `panic = "abort"`, so it
/// is a crash at setup. The bundled dependency on that library is what keeps
/// the two apart; do not drop it and assume this function's failure path
/// covers the case.
pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItemBuilder::with_id("open", "Open Subclave").build(app)?;
    let lock = MenuItemBuilder::with_id("lock", "Lock").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&open, &lock, &separator, &quit])
        .build()?;

    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        // Left click opens the window; the menu is on right click, so a plain
        // click never pops a menu the user did not ask for.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "lock" => {
                let state = app.state::<VaultState>();
                // Only an actually-unlocked vault emits, so a Lock while
                // already locked is a no-op instead of a second event.
                if state.lock_inner() {
                    emit_locked(app, LockReason::Tray);
                }
            }
            // The webview owns the confirmation when a write is still parked.
            "quit" if crate::quit_or_confirm(app) => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });

    // A build without a configured icon still gets a usable menu rather than a
    // failed setup.
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

/// Reveal the main window, whether it is hidden, minimized or behind another
/// app.
fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
