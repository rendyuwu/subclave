// Process-wide allocator.
// The default Windows system heap holds onto freed commit when a workload is
// bursty and fragmented - which is exactly the host process's job of buffering
// heavy PTY output (large base64 chunks) on its way to the webview. A single
// flood (a dev server, an AI CLI redrawing, a big build) inflates the commit to
// several GB and the heap never gives it back, so the process "stays at ~1 GB"
// long after the burst (a high-watermark, not a live leak: trimming the working
// set drops resident pages to a few MB while the committed bytes stay put).
// mimalloc purges freed segments back to the OS on a timer, so the watermark
// recedes once the burst ends.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// How often [`purge_allocator`] runs. Long enough that the sweep is free in
/// aggregate, short enough that a burst is handed back while the user is still
/// in the same sitting.
const ALLOCATOR_PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Hand freed-but-still-committed pages back to the OS.
///
/// Swapping the system heap for mimalloc (v0.3.76) was supposed to make the
/// commit recede on its own, and measurement says it does not: on a normal
/// working session the GUI host went 1.1 GB at 3 min, 7.0 GB at 20 min and
/// 15.8 GB at 64 min, while its working set stayed under 6 GB and dropped to
/// 24 MB under `EmptyWorkingSet` without climbing back. So the bulk of it was
/// committed, freed, untouched, and still charged against the system commit
/// limit, which was already at 32.8 of 44.1 GB. mimalloc only purges a segment
/// that happens to fall entirely free, and a bursty producer (the PTY -> webview
/// path buffers large base64 chunks) strands committed pages across many
/// partly-used segments, so the process commit only ever ratchets up.
///
/// `mi_collect(true)` forces the sweep that the timer-driven purge misses.
/// `libmimalloc-sys` at this version exposes no `mi_option_purge_delay`
/// constant, so this is the available lever; see the test at the bottom of this
/// file for the measured before/after.
pub fn purge_allocator() {
    // SAFETY: `mi_collect` takes no pointers, is documented thread-safe, and is
    // a no-op when there is nothing to reclaim.
    unsafe { libmimalloc_sys::mi_collect(true) };
}

/// Run [`purge_allocator`] on a timer, off the UI thread. Deliberately its own
/// thread rather than a Tauri async task: the sweep walks the heap, and the one
/// thing it must never do is share a thread with anything that draws.
fn spawn_allocator_purge_thread() {
    let _ = std::thread::Builder::new()
        .name("subclave-alloc-purge".into())
        .spawn(|| loop {
            std::thread::sleep(ALLOCATOR_PURGE_INTERVAL);
            purge_allocator();
        });
}

pub mod modules;

use modules::fs;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_window_state::StateFlags;

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

/// Force square corners on Windows 11. DWM paints an 8 px corner radius on
/// every top-level window even with `decorations: false` and `transparent: true`,
/// which leaves a transparent halo over our square webview. DWMWCP_DONOTROUND
/// keeps the OS corners sharp so the CSS border is the only frame.
#[cfg(target_os = "windows")]
fn disable_windows_corner_rounding(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
    };

    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as HWND;
    let pref: u32 = DWMWCP_DONOTROUND as u32;
    // SAFETY: hwnd is a valid window handle owned by Tauri for this call;
    // pref is a stack value passed by pointer with its size.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &pref as *const u32 as *const _,
            std::mem::size_of::<u32>() as u32,
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn disable_windows_corner_rounding(_window: &tauri::WebviewWindow) {}

/// Make a borderless (`decorations: false`) window behave like a normal app on
/// Windows. Two defects come from dropping the native frame:
///
///   1. **Maximize covers the taskbar.** Windows only auto-clamps a maximized
///      window to the monitor *work area* when it carries a standard frame
///      (`WS_THICKFRAME` + `WS_CAPTION`). A borderless window instead fills the
///      whole monitor, so it runs off the bottom over the taskbar - and an
///      OS-level window screenshot then faithfully captures a window that
///      genuinely extends to the bottom of the screen.
///   2. **Taskbar button can't minimize.** Without `WS_MINIMIZEBOX`, clicking
///      the app's taskbar button does not toggle minimize the way every other
///      window does (only the in-app control works).
///
/// Both are fixed without re-adding any visible chrome (`WS_CAPTION` /
/// `WS_SYSMENU` stay off, so no title bar or system buttons are painted):
///   - re-add `WS_MINIMIZEBOX | WS_MAXIMIZEBOX` so the taskbar button and Aero
///     Snap work, and
///   - subclass the window proc to clamp `WM_GETMINMAXINFO`'s maximized rect to
///     the current monitor's work area. The original proc is called first so
///     TAO's `min_inner_size` enforcement (also delivered via this message) is
///     preserved; only the maximized position/size are overridden.
#[cfg(target_os = "windows")]
fn apply_windows_frame_fixes(window: &tauri::WebviewWindow) {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, GetWindowLongPtrW, SetWindowLongPtrW, GWLP_WNDPROC,
        GWL_STYLE, MINMAXINFO, WM_GETMINMAXINFO, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
    };

    // Single main window, so one slot for the original proc is enough.
    static PREV_WNDPROC: AtomicIsize = AtomicIsize::new(0);

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let prev = PREV_WNDPROC.load(Ordering::Relaxed);
        let call_prev = |hwnd, msg, wparam, lparam| {
            if prev != 0 {
                let prev_proc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
                    std::mem::transmute(prev);
                CallWindowProcW(Some(prev_proc), hwnd, msg, wparam, lparam)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        };

        if msg == WM_GETMINMAXINFO {
            // Let the original proc fill defaults first (TAO enforces the
            // window's minimum size here), then clamp the maximized rect.
            let result = call_prev(hwnd, msg, wparam, lparam);
            let h_monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(h_monitor, &mut mi) != 0 {
                let work: RECT = mi.rcWork;
                let mon: RECT = mi.rcMonitor;
                let info = &mut *(lparam as *mut MINMAXINFO);
                // ptMaxPosition is relative to the monitor origin.
                info.ptMaxPosition.x = work.left - mon.left;
                info.ptMaxPosition.y = work.top - mon.top;
                info.ptMaxSize.x = work.right - work.left;
                info.ptMaxSize.y = work.bottom - work.top;
            }
            return result;
        }

        call_prev(hwnd, msg, wparam, lparam)
    }

    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as HWND;

    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let new_style = style | (WS_MINIMIZEBOX as isize) | (WS_MAXIMIZEBOX as isize);
        if new_style != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, new_style);
        }

        // Install the subclass once; re-running would chain it onto itself.
        if PREV_WNDPROC.load(Ordering::Relaxed) == 0 {
            let prev = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc as *const () as isize);
            PREV_WNDPROC.store(prev, Ordering::Relaxed);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn apply_windows_frame_fixes(_window: &tauri::WebviewWindow) {}

/// Disable WebView2 "browser accelerator keys" on the MAIN webview. By default
/// WebView2 treats Ctrl+W (close window), Ctrl+N/T, Ctrl+P, Ctrl+R/F5, etc. as
/// browser shortcuts and acts on them BEFORE web content can cancel them with
/// `preventDefault` - so Ctrl+W quit the whole app instead of being left for the
/// web content to handle. Subclave is an app shell,
/// not a browser, so the app's keyboard handlers should own those combos. The
/// in-app browser child webview is a separate webview and keeps its own defaults.
#[cfg(target_os = "windows")]
fn disable_browser_accelerator_keys(window: &tauri::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
    use windows::core::Interface;
    let _ = window.with_webview(|platform| unsafe {
        let Ok(core) = platform.controller().CoreWebView2() else {
            return;
        };
        let Ok(settings) = core.Settings() else {
            return;
        };
        let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() else {
            return;
        };
        let _ = settings3.SetAreBrowserAcceleratorKeysEnabled(false);
    });
}

#[cfg(not(target_os = "windows"))]
fn disable_browser_accelerator_keys(_window: &tauri::WebviewWindow) {}

#[tauri::command]
async fn open_settings_window(app: tauri::AppHandle, tab: Option<String>) -> Result<(), String> {
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
async fn quit_subclave(app: tauri::AppHandle) -> Result<(), String> {
    app.state::<modules::vault::VaultState>().drop_pending();
    app.exit(0);
    Ok(())
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
fn enforce_configured_min_size(config: &tauri::Config, window: &tauri::WebviewWindow) {
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
fn open_or_reveal_child(
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

// WebKitGTK's DMA-BUF renderer fails to create an EGL display on wlroots
// compositors, NVIDIA's proprietary driver, and minimal sessions.
// It works on Mesa-backed GNOME/KDE/COSMIC, so only fall back where trouble is
// likely. Override with WEBKIT_DISABLE_DMABUF_RENDERER=1 (safe) or =0 (hardware).
#[cfg(target_os = "linux")]
fn configure_linux_rendering() {
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_some() {
        return;
    }

    let wayland = std::env::var("XDG_SESSION_TYPE")
        .map(|v| v.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if !wayland {
        return;
    }

    match wayland_dmabuf_fallback_reason() {
        Some(reason) => {
            eprintln!(
                "subclave: Wayland session, {reason}; disabling WebKitGTK DMA-BUF renderer \
                 (override: WEBKIT_DISABLE_DMABUF_RENDERER=0)"
            );
            unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
        }
        None => eprintln!(
            "subclave: Wayland session on a known-good compositor; keeping WebKitGTK DMA-BUF renderer \
             (set WEBKIT_DISABLE_DMABUF_RENDERER=1 if the window stays blank)"
        ),
    }
}

#[cfg(target_os = "linux")]
fn wayland_dmabuf_fallback_reason() -> Option<&'static str> {
    if has_nvidia_gpu() {
        return Some("NVIDIA proprietary driver detected");
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("XDG_SESSION_DESKTOP"))
        .unwrap_or_default()
        .to_lowercase();
    const KNOWN_GOOD: [&str; 6] = ["gnome", "kde", "plasma", "cosmic", "unity", "pantheon"];
    if !desktop.is_empty() && KNOWN_GOOD.iter().any(|d| desktop.contains(d)) {
        return None;
    }
    if desktop.is_empty() {
        Some("compositor not advertised (XDG_CURRENT_DESKTOP unset)")
    } else {
        Some("wlroots / unrecognised compositor")
    }
}

#[cfg(target_os = "linux")]
fn has_nvidia_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
        || matches!(
            std::env::var("__GLX_VENDOR_LIBRARY_NAME").as_deref(),
            Ok("nvidia")
        )
        || matches!(
            std::env::var("__NV_PRIME_RENDER_OFFLOAD").as_deref(),
            Ok("1")
        )
}

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
fn quit_or_confirm(app: &tauri::AppHandle) -> bool {
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    configure_linux_rendering();

    let builder = tauri::Builder::default().plugin(tauri_plugin_process::init());

    // Relaunching while an instance is already up reveals its window instead of
    // starting a second process (the default file association for a packaged
    // app is "run it", and a second empty window would be the wrong answer).
    // Desktop-only (the plugin does not build for android/ios). Skipped in debug
    // builds so `pnpm tauri dev` can run alongside an installed release.
    #[cfg(all(desktop, not(debug_assertions)))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
    }));

    // Updater is desktop-only; the plugin does not compile on android/ios.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    builder
        .setup(|app| {
            // Keep the process commit from ratcheting up across a long session.
            spawn_allocator_purge_thread();
            // Clipboard auto-clear, idle lock and the pending-save retry.
            spawn_vault_tick_thread(app.handle().clone());
            // Strip Windows 11 DWM rounded corners so the app reads as square.
            if let Some(window) = app.get_webview_window("main") {
                disable_windows_corner_rounding(&window);
                // Clamp borderless maximize to the work area + restore the
                // taskbar minimize affordance (see fn docs).
                apply_windows_frame_fixes(&window);
                // Stop WebView2 from hijacking Ctrl+W/Ctrl+P/Ctrl+R/etc. as
                // browser shortcuts; as window-closing actions they would act
                // on the whole window before web content could cancel them.
                disable_browser_accelerator_keys(&window);
                // A session saved while maximized may have been restored before
                // the work-area clamp was installed, leaving it over the taskbar.
                // Re-assert maximize now - the window is still hidden (the
                // frontend calls show() after first paint), so there's no flicker.
                #[cfg(target_os = "windows")]
                if window.is_maximized().unwrap_or(false) {
                    let _ = window.unmaximize();
                    let _ = window.maximize();
                }
                // Config windows are built - and the window-state plugin's
                // `on_window_ready` restore therefore runs - before this setup
                // hook, so this is the first point at which the restored size
                // is observable. Raise it back to the configured floor if the
                // saved size predates a floor increase (see fn docs).
                enforce_configured_min_size(app.config(), &window);
            }
            // System tray: Open, Lock, Quit. A host with no tray host logs and
            // keeps running without one; a host missing the AppIndicator
            // library aborts inside the toolkit instead, which is why the
            // bundle depends on it.
            if let Err(e) = modules::tray::build(app.handle()) {
                log::error!("subclave: could not build the tray icon: {e}");
            }
            // macOS: the default app menu binds Cmd+W to "Close Window", and
            // that native accelerator fires before the webview's JS handler - so
            // Cmd+W quit the whole app. Rebuild a standard menu that keeps the
            // App/Edit/Window items (so copy/paste/quit and the system shortcuts
            // still work) but OMITS Close Window, so the accelerator does not
            // fire at all. Windows/Linux have no such menu accelerator (Windows is
            // handled by disable_browser_accelerator_keys; Linux's WebKitGTK does
            // not bind Cmd/Ctrl+W), so this is macOS-only.
            #[cfg(target_os = "macos")]
            {
                use tauri::menu::{MenuBuilder, SubmenuBuilder};
                let h = app.handle();
                let app_menu = SubmenuBuilder::new(h, "Subclave")
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
                let edit_menu = SubmenuBuilder::new(h, "Edit")
                    .undo()
                    .redo()
                    .separator()
                    .cut()
                    .copy()
                    .paste()
                    .select_all()
                    .build()?;
                let window_menu = SubmenuBuilder::new(h, "Window")
                    .minimize()
                    .maximize()
                    .separator()
                    .fullscreen()
                    .build()?;
                let menu = MenuBuilder::new(h)
                    .items(&[&app_menu, &edit_menu, &window_menu])
                    .build()?;
                h.set_menu(menu)?;
            }
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
            open_settings_window,
            quit_subclave,
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
        ])
        .manage(modules::vault::VaultState::default())
        .on_window_event(|window, event| {
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
                _ => {}
            }
        })
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

// Test module last: clippy's `items_after_test_module`.
#[cfg(all(test, target_os = "windows"))]
mod allocator_tests {
    use super::purge_allocator;

    /// Bytes this process has committed privately, i.e. what it charges against
    /// the system commit limit. Deliberately NOT the working set: the whole
    /// point of the bug is that the working set looks fine while the commit
    /// climbs into double-digit GB.
    fn committed_private_bytes() -> usize {
        use windows::Win32::System::Memory::{
            VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_PRIVATE,
        };
        let stride = std::mem::size_of::<MEMORY_BASIC_INFORMATION>();
        let mut addr: usize = 0;
        let mut total: usize = 0;
        loop {
            let mut mbi = MEMORY_BASIC_INFORMATION::default();
            // SAFETY: querying our own address space; `mbi` is a live, correctly
            // sized buffer. A zero return means the walk ran off the top.
            let read = unsafe { VirtualQuery(Some(addr as *const _), &mut mbi, stride) };
            if read == 0 {
                break;
            }
            if mbi.State == MEM_COMMIT && mbi.Type == MEM_PRIVATE {
                total += mbi.RegionSize;
            }
            let next = mbi.BaseAddress as usize + mbi.RegionSize;
            if next <= addr {
                break; // no forward progress; refuse to spin
            }
            addr = next;
        }
        total
    }

    const CHUNK: usize = 1024 * 1024;

    /// Allocate `chunks` MiB, touch every page, then drop it all. `vec![_; _]`
    /// memsets, so this is committed AND resident, exactly like a base64 chunk
    /// on its way to the webview.
    fn burst_and_free(chunks: usize) {
        let held: Vec<Vec<u8>> = (0..chunks).map(|_| vec![7u8; CHUNK]).collect();
        std::hint::black_box(&held);
    }

    /// The regression guard for "Subclave gets heavy and then stops responding after
    /// a long session". Both halves are needed and they assert different things.
    ///
    /// Deliberately ONE test function rather than two: each phase reads the
    /// process-wide commit, and `cargo test` runs test functions in parallel, so
    /// two allocator tests would measure each other's bursts and flake.
    #[test]
    fn freed_memory_goes_back_to_the_os_and_does_not_ratchet() {
        // Phase 1: a single burst must not stay charged to the process. Measured
        // without the fix, the allocator returned NOTHING: 1090 MiB of a 1090 MiB
        // burst was still committed afterwards.
        let base = committed_private_bytes();
        let held: Vec<Vec<u8>> = (0..1024).map(|_| vec![7u8; CHUNK]).collect();
        let peak = committed_private_bytes();
        drop(held);
        purge_allocator();
        let after = committed_private_bytes();

        let grew = peak.saturating_sub(base);
        let kept = after.saturating_sub(base);
        assert!(
            grew > 512 * 1024 * 1024,
            "burst did not register: base={base} peak={peak} (grew {grew})"
        );
        assert!(
            kept * 4 < grew,
            "allocator kept {} MiB of the {} MiB burst after purge_allocator()",
            kept / 1024 / 1024,
            grew / 1024 / 1024
        );

        // Phase 2: the actual complaint is not that one burst is expensive, it is
        // that an hour of them never comes back down. Repeated cycles must land
        // in the same place instead of drifting upward, which is the "long
        // session stays flat" property in miniature.
        let mut marks = Vec::new();
        for _ in 0..3 {
            burst_and_free(256);
            purge_allocator();
            marks.push(committed_private_bytes());
        }
        let drift = marks.last().unwrap().saturating_sub(marks[0]);
        assert!(
            drift < 64 * 1024 * 1024,
            "commit drifted up {} MiB across three burst cycles: {:?} MiB",
            drift / 1024 / 1024,
            marks.iter().map(|m| m / 1024 / 1024).collect::<Vec<_>>()
        );
    }
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
