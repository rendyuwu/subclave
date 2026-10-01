//! Windows-specific window fixes. Compiled on every target; the non-Windows
//! arms are no-op stubs so the callers stay platform-agnostic.

/// Force square corners on Windows 11. DWM paints an 8 px corner radius on
/// every top-level window even with `decorations: false` and `transparent: true`,
/// which leaves a transparent halo over our square webview. DWMWCP_DONOTROUND
/// keeps the OS corners sharp so the CSS border is the only frame.
#[cfg(target_os = "windows")]
pub(crate) fn disable_windows_corner_rounding(window: &tauri::WebviewWindow) {
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
pub(crate) fn disable_windows_corner_rounding(_window: &tauri::WebviewWindow) {}

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
pub(crate) fn apply_windows_frame_fixes(window: &tauri::WebviewWindow) {
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
pub(crate) fn apply_windows_frame_fixes(_window: &tauri::WebviewWindow) {}

/// Disable WebView2 "browser accelerator keys" on the MAIN webview. By default
/// WebView2 treats Ctrl+W (close window), Ctrl+N/T, Ctrl+P, Ctrl+R/F5, etc. as
/// browser shortcuts and acts on them BEFORE web content can cancel them with
/// `preventDefault` - so Ctrl+W quit the whole app instead of being left for the
/// web content to handle. Subclave is an app shell,
/// not a browser, so the app's keyboard handlers should own those combos. The
/// in-app browser child webview is a separate webview and keeps its own defaults.
#[cfg(target_os = "windows")]
pub(crate) fn disable_browser_accelerator_keys(window: &tauri::WebviewWindow) {
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
pub(crate) fn disable_browser_accelerator_keys(_window: &tauri::WebviewWindow) {}

/// The Windows setup block from `run`, in its original order: strip DWM rounded
/// corners, clamp borderless maximize to the work area and restore the taskbar
/// minimize affordance, then stop WebView2 from hijacking Ctrl+W / Ctrl+P /
/// Ctrl+R / etc. as browser shortcuts (as window-closing actions they would act
/// on the whole window before web content could cancel them). Off Windows every
/// step is a no-op.
pub(crate) fn apply_main_window_fixes(window: &tauri::WebviewWindow) {
    disable_windows_corner_rounding(window);
    apply_windows_frame_fixes(window);
    disable_browser_accelerator_keys(window);
    // A session saved while maximized may have been restored before the
    // work-area clamp was installed, leaving it over the taskbar. Re-assert
    // maximize now - the window is still hidden (the frontend calls show()
    // after first paint), so there's no flicker.
    #[cfg(target_os = "windows")]
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
        let _ = window.maximize();
    }
}
