pub mod modules;

mod allocator;
mod commands;
mod windows;

pub use allocator::purge_allocator;
pub(crate) use windows::quit_or_confirm;

use modules::fs;
use tauri::Manager;
use tauri_plugin_window_state::StateFlags;

mod platform {
    #[cfg(target_os = "linux")]
    pub mod linux;
    #[cfg(target_os = "macos")]
    pub mod macos;
    pub mod windows;
}

/// The one background tick: clipboard auto-clear every second, the idle-lock
/// deadline check every 5th tick, and the pending-save retry every 10th.
/// Deliberately a `std::thread` like the allocator purge: the work is
/// blocking (clipboard round trips, file writes), and a tokio sleep is not
/// reachable through `tauri::async_runtime`. The idle check goes through
/// `VaultState::access`, so the expiry, the payload wipe and the auto-lock
/// event are the same code path every command shell uses; the retry works
/// while locked because a pending write holds ciphertext only.
fn spawn_vault_tick_thread(app: tauri::AppHandle) {
    let _ = std::thread::Builder::new()
        .name("subclave-vault-tick".into())
        .spawn(move || {
            let mut tick: u64 = 0;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                tick = tick.wrapping_add(1);
                modules::clipboard::clear_tick();
                if tick.is_multiple_of(5) {
                    // An expired deadline locks here and flags the event;
                    // drain it so `subclave:vault-locked` fires now. The
                    // guard is dropped deliberately: the tick holds no state.
                    let state = app.state::<modules::vault::VaultState>();
                    drop(state.access());
                    modules::vault::drain_auto_lock(&app);
                }
                if tick.is_multiple_of(10) {
                    if let Ok(dir) = app.path().app_data_dir() {
                        let state = app.state::<modules::vault::VaultState>();
                        let _ = modules::vault::vault_retry_save_inner(&state, &dir);
                        // The banner clears on the retry's success without a
                        // command shell to drain for it.
                        modules::vault::drain_save_event(&app);
                    }
                }
            }
        });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    platform::linux::configure_linux_rendering();

    let builder = tauri::Builder::default().plugin(tauri_plugin_process::init());

    // Relaunching while an instance is already up reveals its window instead of
    // starting a second process (the default file association for a packaged
    // app is "run it", and a second empty window would be the wrong answer).
    // Desktop-only (the plugin does not build for android/ios). Skipped in debug
    // builds so `pnpm tauri dev` can run alongside an installed release.
    #[cfg(all(desktop, not(debug_assertions)))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            windows::reveal(&window);
        }
    }));

    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    builder
        .setup(|app| {
            // Keep the process commit from ratcheting up across a long session.
            allocator::spawn_allocator_purge_thread();
            // Clipboard auto-clear, idle lock and the pending-save retry.
            spawn_vault_tick_thread(app.handle().clone());
            if let Some(window) = app.get_webview_window("main") {
                // Windows 11 DWM rounded corners, borderless maximize clamping
                // and the WebView2 browser accelerators (see the fn docs).
                platform::windows::apply_main_window_fixes(&window);
                // Config windows are built - and the window-state plugin's
                // `on_window_ready` restore therefore runs - before this setup
                // hook, so this is the first point at which the restored size
                // is observable. Raise it back to the configured floor if the
                // saved size predates a floor increase (see fn docs).
                windows::enforce_configured_min_size(app.config(), &window);
            }
            // System tray: Open, Lock, Quit. A host with no tray host logs and
            // keeps running without one; a host missing the AppIndicator
            // library aborts inside the toolkit instead, which is why the
            // bundle depends on it.
            if let Err(e) = modules::tray::build(app.handle()) {
                log::error!("subclave: could not build the tray icon: {e}");
            }
            // macOS: rebuild a menu without the Cmd+W "Close Window" item (see
            // the fn docs).
            #[cfg(target_os = "macos")]
            platform::macos::rebuild_app_menu(app.handle())?;
            Ok(())
        })
        // Skip restoring VISIBLE; the frontend calls window.show() after first
        // paint so the user never sees a transparent window-shadow flash on
        // Windows/Linux.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(StateFlags::all() & !StateFlags::VISIBLE)
                .with_denylist(&["settings"])
                .build(),
        )
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_os::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            fs::file::fs_read_file,
            fs::file::fs_write_file,
            commands::open_settings_window,
            commands::quit_subclave,
            modules::vault::vault_status,
            modules::vault::vault_create,
            modules::vault::vault_unlock,
            modules::vault::vault_lock,
            modules::vault::vault_touch,
            modules::vault::vault_change_master,
            modules::vault::vault_retry_save,
            modules::vault::vault_restore_snapshot,
            modules::vault::vault_list,
            modules::vault::vault_search,
            modules::vault::vault_entry_get,
            modules::vault::vault_entry_reveal,
            modules::vault::vault_entry_upsert,
            modules::vault::vault_entry_move,
            modules::vault::vault_entry_trash,
            modules::vault::vault_entry_restore,
            modules::vault::vault_entry_delete,
            modules::vault::vault_entry_restore_version,
            modules::vault::vault_group_upsert,
            modules::vault::vault_group_delete,
            modules::clipboard::clip_copy_field,
            modules::totp::totp_code,
            modules::totp::totp_preview,
            modules::generator::gen_password,
            modules::strength::gen_strength,
            modules::sync::engine::sync_configure,
            modules::sync::engine::sync_disable,
            modules::sync::engine::sync_pull,
            modules::sync::engine::sync_push,
            modules::sync::engine::sync_join,
        ])
        .manage(modules::vault::VaultState::default())
        .manage(modules::sync::engine::SyncState::default())
        .on_window_event(windows::on_main_window_event)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| match event {
            // A close or quit that still has a parked write gets the webview's
            // confirmation instead of exiting; the exit is vetoed until the
            // user decides.
            tauri::RunEvent::ExitRequested { api, .. } => {
                if !quit_or_confirm(app) {
                    api.prevent_exit();
                }
            }
            // Clear the clipboard on quit, but only when it still holds a
            // copied secret: anything the user copied since must survive.
            tauri::RunEvent::Exit => modules::clipboard::clear_on_exit(),
            _ => {}
        });
}
