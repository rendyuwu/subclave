//! macOS menu setup.

use tauri::menu::{MenuBuilder, SubmenuBuilder};

/// Rebuild the app menu.
///
/// The default app menu binds Cmd+W to "Close Window", and that native
/// accelerator fires before the webview's JS handler - so Cmd+W quit the whole
/// app. Rebuild a standard menu that keeps the App/Edit/Window items (so
/// copy/paste/quit and the system shortcuts still work) but OMITS Close Window,
/// so the accelerator does not fire at all. Windows/Linux have no such menu
/// accelerator (Windows is handled by `disable_browser_accelerator_keys`;
/// Linux's WebKitGTK does not bind Cmd/Ctrl+W), so this is macOS-only.
pub(crate) fn rebuild_app_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    let app_menu = SubmenuBuilder::new(app, "Subclave")
        .about(None)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .fullscreen()
        .build()?;
    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &edit_menu, &window_menu])
        .build()?;
    app.set_menu(menu)?;
    Ok(())
}
