//! Native messaging manifests: where each browser looks for the host
//! manifest, what it must contain, and how the proxy binary's path is
//! resolved.
//!
//! A wrong path here is a silently dead channel, so `proxy_path` resolves in a
//! fixed order and the status command reports the result.

use std::path::{Path, PathBuf};

use serde_json::json;
use tauri::Manager;

pub const HOST_NAME: &str = "dev.rendy.subclave";
/// Derived from the `key` field committed in
/// `extension/manifest.chrome.json`; the browser-verify script under scripts/
/// proves the two agree.
pub const CHROMIUM_EXTENSION_ID: &str = "fbefjngeldidlcdilbigapddeimhklpn";
pub const FIREFOX_EXTENSION_ID: &str = "subclave@rendy.dev";

/// The browser family a switch or a manifest belongs to.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    Chromium,
    Firefox,
}

impl Family {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Family::Chromium => "chromium",
            Family::Firefox => "firefox",
        }
    }

    pub(crate) fn all() -> [Family; 2] {
        [Family::Chromium, Family::Firefox]
    }
}

/// The directories the per-OS tables are built from.
pub struct Roots {
    /// The home dir; only Linux detection reads it (Snap, Flatpak, `.mozilla`).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub home: PathBuf,
    /// The config dir; only Linux and macOS detection read it.
    #[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
    pub config: PathBuf,
    /// The roaming app data dir; only Windows detection reads it.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub data: PathBuf,
    /// `%LOCALAPPDATA%`; only Windows detection reads it.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub local_data: PathBuf,
    pub app_data: PathBuf,
}

/// One detected browser: where its manifest goes and whether it is a
/// sandboxed (Snap/Flatpak) install.
pub struct BrowserSlot {
    pub family: Family,
    pub browser: &'static str,
    pub manifest_dir: PathBuf,
    pub sandboxed: bool,
}

pub fn roots(app: &tauri::AppHandle) -> Result<Roots, String> {
    let path = app.path();
    Ok(Roots {
        home: path.home_dir().map_err(|e| format!("browser: {e}"))?,
        config: path.config_dir().map_err(|e| format!("browser: {e}"))?,
        data: path.data_dir().map_err(|e| format!("browser: {e}"))?,
        local_data: path.local_data_dir().map_err(|e| format!("browser: {e}"))?,
        app_data: path.app_data_dir().map_err(|e| format!("browser: {e}"))?,
    })
}

/// Detected browsers whose config directory exists. Never the real home: the
/// tests build a `Roots` from a temp directory.
pub fn slots(roots: &Roots) -> Vec<BrowserSlot> {
    #[cfg(windows)]
    {
        slots_windows(roots)
    }
    #[cfg(not(windows))]
    {
        slots_unix(roots)
    }
}

#[cfg(target_os = "linux")]
fn slots_unix(roots: &Roots) -> Vec<BrowserSlot> {
    // (browser, config-relative dir, snap home-relative dir, flatpak
    // home-relative dir). None where the vendor ships no such package.
    const CHROMIUM: &[(&str, &str, Option<&str>, Option<&str>)] = &[
        ("Google Chrome", "google-chrome", None, None),
        (
            "Chromium",
            "chromium",
            Some("snap/chromium/current/.config/chromium"),
            Some(".var/app/org.chromium.Chromium/config/chromium"),
        ),
        (
            "Microsoft Edge",
            "microsoft-edge",
            None,
            Some(".var/app/com.microsoft.Edge/config/microsoft-edge"),
        ),
        (
            "Brave",
            "BraveSoftware/Brave-Browser",
            Some("snap/brave/current/.config/BraveSoftware/Brave-Browser"),
            Some(".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser"),
        ),
        (
            "Vivaldi",
            "vivaldi",
            None,
            Some(".var/app/com.vivaldi.Vivaldi/config/vivaldi"),
        ),
    ];
    const CHROMIUM_LEAF: &str = "NativeMessagingHosts";
    const FIREFOX_LEAF: &str = "native-messaging-hosts";
    const FIREFOX_SANDBOXES: &[&str] = &[
        "snap/firefox/common/.mozilla",
        ".var/app/org.mozilla.firefox/.mozilla",
    ];

    let mut out = Vec::new();
    for (browser, rel, snap, flatpak) in CHROMIUM {
        let base = roots.config.join(rel);
        push_slot(
            &mut out,
            Family::Chromium,
            browser,
            &base,
            CHROMIUM_LEAF,
            false,
        );
        for sandbox in [snap, flatpak].into_iter().flatten() {
            let base = roots.home.join(sandbox);
            push_slot(
                &mut out,
                Family::Chromium,
                browser,
                &base,
                CHROMIUM_LEAF,
                true,
            );
        }
    }

    let firefox_base = roots.home.join(".mozilla");
    push_slot(
        &mut out,
        Family::Firefox,
        "Firefox",
        &firefox_base,
        FIREFOX_LEAF,
        false,
    );
    for sandbox in FIREFOX_SANDBOXES {
        let base = roots.home.join(sandbox);
        push_slot(
            &mut out,
            Family::Firefox,
            "Firefox",
            &base,
            FIREFOX_LEAF,
            true,
        );
    }
    out
}

#[cfg(target_os = "macos")]
fn slots_unix(roots: &Roots) -> Vec<BrowserSlot> {
    const CHROMIUM: &[(&str, &str)] = &[
        ("Google Chrome", "Google/Chrome"),
        ("Chromium", "Chromium"),
        ("Microsoft Edge", "Microsoft Edge"),
        ("Brave", "BraveSoftware/Brave-Browser"),
        ("Vivaldi", "Vivaldi"),
    ];
    let mut out = Vec::new();
    for (browser, rel) in CHROMIUM {
        let base = roots.config.join(rel);
        push_slot(
            &mut out,
            Family::Chromium,
            browser,
            &base,
            "NativeMessagingHosts",
            false,
        );
    }
    let firefox_base = roots.config.join("Mozilla");
    push_slot(
        &mut out,
        Family::Firefox,
        "Firefox",
        &firefox_base,
        "NativeMessagingHosts",
        false,
    );
    out
}

#[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos")))]
fn slots_unix(_roots: &Roots) -> Vec<BrowserSlot> {
    Vec::new()
}

/// Add a slot when its browser directory exists.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn push_slot(
    out: &mut Vec<BrowserSlot>,
    family: Family,
    browser: &'static str,
    base: &Path,
    leaf: &str,
    sandboxed: bool,
) {
    if !base.is_dir() {
        return;
    }
    out.push(BrowserSlot {
        family,
        browser,
        manifest_dir: base.join(leaf),
        sandboxed,
    });
}

/// The manifest file for `slot`. On Unix and macOS it sits in the browser's own
/// `NativeMessagingHosts` directory under the host name; on Windows one
/// directory per family is shared by every browser in it, so each browser gets
/// its own `<browser>.json` there. That keeps the status rows' paths distinct,
/// which the Settings list relies on for its row identity.
pub fn manifest_path(slot: &BrowserSlot) -> PathBuf {
    if cfg!(windows) {
        slot.manifest_dir.join(format!("{}.json", slot.browser))
    } else {
        slot.manifest_dir.join(format!("{HOST_NAME}.json"))
    }
}

/// The manifest JSON body for `family` and `proxy`.
fn manifest_body(family: Family, proxy: &Path) -> serde_json::Value {
    let path = proxy.to_string_lossy().to_string();
    match family {
        Family::Chromium => json!({
            "name": HOST_NAME,
            "description": "Subclave browser integration",
            "path": path,
            "type": "stdio",
            "allowed_origins": [format!("chrome-extension://{CHROMIUM_EXTENSION_ID}/")],
        }),
        Family::Firefox => json!({
            "name": HOST_NAME,
            "description": "Subclave browser integration",
            "path": path,
            "type": "stdio",
            "allowed_extensions": [FIREFOX_EXTENSION_ID],
        }),
    }
}

/// Write every detected browser's manifest for `family`.
pub fn write_for_family(app: &tauri::AppHandle, family: Family) -> Result<(), String> {
    let roots = roots(app)?;
    let proxy = proxy_path(app)?;
    write_for_roots(&roots, family, &proxy)
}

pub(crate) fn write_for_roots(roots: &Roots, family: Family, proxy: &Path) -> Result<(), String> {
    let body = manifest_body(family, proxy);
    let text = serde_json::to_string_pretty(&body).map_err(|e| format!("browser: {e}"))?;
    let detected = slots(roots);
    for slot in detected.iter().filter(|s| s.family == family) {
        std::fs::create_dir_all(&slot.manifest_dir).map_err(|e| {
            format!(
                "browser: could not create {}: {e}",
                slot.manifest_dir.display()
            )
        })?;
        let path = manifest_path(slot);
        std::fs::write(&path, &text)
            .map_err(|e| format!("browser: could not write {}: {e}", path.display()))?;
        #[cfg(windows)]
        write_registry_for(slot.browser, &path)?;
    }
    Ok(())
}

/// Remove every detected browser's manifest for `family`.
pub fn remove_for_family(app: &tauri::AppHandle, family: Family) -> Result<(), String> {
    let roots = roots(app)?;
    remove_for_roots(&roots, family)
}

pub(crate) fn remove_for_roots(roots: &Roots, family: Family) -> Result<(), String> {
    for slot in slots(roots).iter().filter(|s| s.family == family) {
        let path = manifest_path(slot);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("browser: could not remove {}: {e}", path.display())),
        }
        #[cfg(windows)]
        remove_registry_for(slot.browser)?;
    }
    Ok(())
}

/// Resolve the proxy binary. Order matters: an AppImage copy beats the sibling
/// name, which beats the Linux system path.
pub fn proxy_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    // The app data dir is only read by the Linux AppImage copy-out below.
    #[cfg(not(target_os = "linux"))]
    let _ = app;

    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from));

    #[cfg(target_os = "linux")]
    {
        if let Some(appimage) = std::env::var_os("APPIMAGE") {
            if !appimage.is_empty() {
                let app_data = app
                    .path()
                    .app_data_dir()
                    .map_err(|e| format!("browser: {e}"))?;
                let mounted = exe_dir
                    .as_ref()
                    .map(|dir| dir.join(proxy_file_name()))
                    .ok_or_else(|| "browser: the executable directory is unknown".to_string())?;
                return refresh_appimage_copy(&mounted, &app_data);
            }
        }
    }

    if let Some(dir) = exe_dir.as_ref() {
        let sibling = dir.join(proxy_file_name());
        if sibling.exists() {
            return Ok(sibling);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let system = PathBuf::from("/usr/bin").join(proxy_file_name());
        if system.exists() {
            return Ok(system);
        }
    }

    Err("browser: the proxy binary could not be located".to_string())
}

fn proxy_file_name() -> &'static str {
    if cfg!(windows) {
        "subclave-proxy.exe"
    } else {
        "subclave-proxy"
    }
}

/// Copy the running mount's sibling into the app data dir when the bytes
/// differ, and return the copy's path.
#[cfg(target_os = "linux")]
fn refresh_appimage_copy(mounted: &Path, app_data: &Path) -> Result<PathBuf, String> {
    let dest = app_data.join(proxy_file_name());
    let changed = match (digest(mounted), digest(&dest)) {
        (Some(a), Some(b)) => a != b,
        (Some(_), None) => true,
        (None, _) => {
            return Err(format!(
                "browser: the mounted proxy at {} is unreadable",
                mounted.display()
            ))
        }
    };
    if changed {
        std::fs::create_dir_all(app_data).map_err(|e| format!("browser: {e}"))?;
        let temp = app_data.join(format!("{}.tmp", proxy_file_name()));
        std::fs::copy(mounted, &temp)
            .map_err(|e| format!("browser: could not copy the proxy: {e}"))?;
        set_executable(&temp);
        std::fs::rename(&temp, &dest).map_err(|e| format!("browser: {e}"))?;
    }
    Ok(dest)
}

#[cfg(target_os = "linux")]
fn digest(path: &Path) -> Option<[u8; 32]> {
    use ring::digest::{digest, SHA256};
    let bytes = std::fs::read(path).ok()?;
    let hash = digest(&SHA256, &bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(hash.as_ref());
    Some(out)
}

#[cfg(target_os = "linux")]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

/// Refresh the AppImage copy and re-write the manifests for every family whose
/// switch is on. Idempotent; called from `setup()`.
pub fn startup_refresh(app: &tauri::AppHandle) {
    // A failed resolve is reported through the status command's listenError.
    let Ok(roots) = roots(app) else {
        return;
    };
    let prefs = crate::modules::prefs::read(&roots.app_data);
    let proxy = match proxy_path(app) {
        Ok(proxy) => proxy,
        Err(e) => {
            log::warn!("subclave: browser manifests not refreshed: {e}");
            return;
        }
    };
    for (family, on) in [
        (Family::Chromium, prefs.browser_chromium),
        (Family::Firefox, prefs.browser_firefox),
    ] {
        if !on {
            continue;
        }
        if let Err(e) = write_for_roots(&roots, family, &proxy) {
            log::warn!("subclave: browser manifests not refreshed: {e}");
        }
    }
}

/// One family's row for `browser_integration_status`.
pub fn status_rows(app: &tauri::AppHandle, listen_error: Option<String>) -> Vec<serde_json::Value> {
    let roots = match roots(app) {
        Ok(roots) => roots,
        Err(_) => return Vec::new(),
    };
    let detected = slots(&roots);
    let prefs = crate::modules::prefs::read(&roots.app_data);
    Family::all()
        .into_iter()
        .map(|family| {
            let browsers: Vec<serde_json::Value> = detected
                .iter()
                .filter(|s| s.family == family)
                .map(|slot| {
                    let path = manifest_path(slot);
                    json!({
                        "browser": slot.browser,
                        "manifestPath": path.to_string_lossy(),
                        "installed": path.is_file(),
                        "sandboxed": slot.sandboxed,
                    })
                })
                .collect();
            json!({
                "family": family.as_str(),
                "enabled": match family {
                    Family::Chromium => prefs.browser_chromium,
                    Family::Firefox => prefs.browser_firefox,
                },
                "browsers": browsers,
                "listenError": listen_error,
            })
        })
        .collect()
}

// ---- Windows: registry keys and per-family JSON files ----
//
// The full key paths are written out rather than composed, so this file and
// `installer.nsh`'s `DeleteRegKey` lines can be compared literally.

/// (browser, `%LOCALAPPDATA%`-relative profile dir, default-value key path).
#[cfg(windows)]
const CHROMIUM: &[(&str, &str, &str)] = &[
    (
        "Google Chrome",
        "Google/Chrome/User Data",
        r"Software\Google\Chrome\NativeMessagingHosts\dev.rendy.subclave",
    ),
    (
        "Chromium",
        "Chromium/User Data",
        r"Software\Chromium\NativeMessagingHosts\dev.rendy.subclave",
    ),
    (
        "Microsoft Edge",
        "Microsoft/Edge/User Data",
        r"Software\Microsoft\Edge\NativeMessagingHosts\dev.rendy.subclave",
    ),
    ("Brave", "BraveSoftware/Brave-Browser/User Data", ""),
    ("Vivaldi", "Vivaldi/User Data", ""),
];

/// The Firefox key path, matching `installer.nsh`.
#[cfg(windows)]
const FIREFOX_REGISTRY: &str = r"Software\Mozilla\NativeMessagingHosts\dev.rendy.subclave";

/// The registry path for `browser`, or `None` for a browser that reads another
/// browser's key (Brave and Vivaldi read Chrome's).
#[cfg(windows)]
fn registry_path(browser: &str) -> Option<&'static str> {
    if browser == "Firefox" {
        return Some(FIREFOX_REGISTRY);
    }
    CHROMIUM
        .iter()
        .find(|(name, _, _)| *name == browser)
        .map(|(_, _, key)| *key)
        .filter(|key| !key.is_empty())
}

#[cfg(windows)]
fn slots_windows(roots: &Roots) -> Vec<BrowserSlot> {
    let mut out = Vec::new();
    for &(browser, profile, _key) in CHROMIUM {
        let profile_dir = roots.local_data.join(profile);
        if profile_dir.is_dir() || registry_key_exists(browser) {
            out.push(BrowserSlot {
                family: Family::Chromium,
                browser,
                manifest_dir: roots.app_data.join("browser-hosts").join("chromium"),
                sandboxed: false,
            });
        }
    }
    let firefox_profile = roots.data.join("Mozilla/Firefox");
    if firefox_profile.is_dir() || registry_key_exists("Firefox") {
        out.push(BrowserSlot {
            family: Family::Firefox,
            browser: "Firefox",
            manifest_dir: roots.app_data.join("browser-hosts").join("firefox"),
            sandboxed: false,
        });
    }
    out
}

#[cfg(windows)]
fn registry_key_exists(browser: &str) -> bool {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };
    let Some(path) = registry_path(browser) else {
        return false;
    };
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut handle: HKEY = std::ptr::null_mut();
    let status =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide.as_ptr(), 0, KEY_READ, &mut handle) };
    if status == 0 {
        unsafe { RegCloseKey(handle) };
        true
    } else {
        false
    }
}

#[cfg(windows)]
fn write_registry_for(browser: &str, json_path: &Path) -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    let Some(path) = registry_path(browser) else {
        return Ok(());
    };
    let key: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let value: Vec<u16> = json_path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut handle: HKEY = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            0,
            std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null(),
            &mut handle,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "browser: could not create the registry key for {browser}"
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2) };
    let status = unsafe {
        RegSetValueExW(
            handle,
            std::ptr::null(),
            0,
            REG_SZ,
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    };
    unsafe { RegCloseKey(handle) };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "browser: could not set the registry value for {browser}"
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn remove_registry_for(browser: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{RegDeleteKeyW, HKEY_CURRENT_USER};
    let Some(path) = registry_path(browser) else {
        return Ok(());
    };
    let key: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    // A missing key is fine.
    unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, key.as_ptr()) };
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::modules::vault::test_util::TempDir;

    fn make_roots() -> (TempDir, Roots) {
        let dir = TempDir::new("manifests");
        let roots = Roots {
            home: dir.0.join("home"),
            config: dir.0.join("config"),
            data: dir.0.join("data"),
            local_data: dir.0.join("local"),
            app_data: dir.0.join("appdata"),
        };
        std::fs::create_dir_all(&roots.home).unwrap();
        std::fs::create_dir_all(&roots.config).unwrap();
        (dir, roots)
    }

    fn chrome_manifest(roots: &Roots) -> PathBuf {
        roots
            .config
            .join("google-chrome/NativeMessagingHosts")
            .join(format!("{HOST_NAME}.json"))
    }

    fn firefox_manifest(roots: &Roots) -> PathBuf {
        roots
            .home
            .join(".mozilla/native-messaging-hosts")
            .join(format!("{HOST_NAME}.json"))
    }

    #[test]
    fn detection_finds_only_created_dirs() {
        let (_dir, roots) = make_roots();
        assert!(slots(&roots).is_empty());

        std::fs::create_dir_all(roots.config.join("google-chrome")).unwrap();
        std::fs::create_dir_all(roots.home.join(".mozilla")).unwrap();
        std::fs::create_dir_all(roots.config.join("microsoft-edge")).unwrap();
        let found = slots(&roots);
        let names: Vec<&str> = found.iter().map(|s| s.browser).collect();
        assert!(names.contains(&"Google Chrome"));
        assert!(names.contains(&"Microsoft Edge"));
        assert!(names.contains(&"Firefox"));
        assert!(!names.contains(&"Chromium"));
        assert!(!names.contains(&"Brave"));
        assert!(!names.contains(&"Vivaldi"));
        assert!(found.iter().all(|s| !s.sandboxed));
    }

    #[test]
    fn chromium_manifest_carries_allowed_origins() {
        let (_dir, roots) = make_roots();
        std::fs::create_dir_all(roots.config.join("google-chrome")).unwrap();
        write_for_roots(&roots, Family::Chromium, Path::new("/opt/subclave-proxy")).unwrap();

        let path = chrome_manifest(&roots);
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["name"], HOST_NAME);
        assert_eq!(value["path"], "/opt/subclave-proxy");
        assert_eq!(value["type"], "stdio");
        assert_eq!(
            value["allowed_origins"][0],
            format!("chrome-extension://{CHROMIUM_EXTENSION_ID}/")
        );
        assert!(value.get("allowed_extensions").is_none());
    }

    #[test]
    fn firefox_manifest_carries_allowed_extensions_and_no_key() {
        let (_dir, roots) = make_roots();
        std::fs::create_dir_all(roots.home.join(".mozilla")).unwrap();
        write_for_roots(&roots, Family::Firefox, Path::new("/opt/subclave-proxy")).unwrap();

        let path = firefox_manifest(&roots);
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["allowed_extensions"][0], FIREFOX_EXTENSION_ID);
        assert!(value.get("allowed_origins").is_none());
        assert!(value.get("key").is_none());
    }

    #[test]
    fn write_then_remove_leaves_nothing_behind() {
        let (_dir, roots) = make_roots();
        std::fs::create_dir_all(roots.config.join("google-chrome")).unwrap();
        std::fs::create_dir_all(roots.home.join(".mozilla")).unwrap();
        write_for_roots(&roots, Family::Chromium, Path::new("/opt/subclave-proxy")).unwrap();
        write_for_roots(&roots, Family::Firefox, Path::new("/opt/subclave-proxy")).unwrap();
        assert!(chrome_manifest(&roots).is_file());
        assert!(firefox_manifest(&roots).is_file());

        remove_for_roots(&roots, Family::Chromium).unwrap();
        assert!(!chrome_manifest(&roots).exists());
        assert!(firefox_manifest(&roots).is_file());
        // Removing again is fine.
        remove_for_roots(&roots, Family::Chromium).unwrap();

        remove_for_roots(&roots, Family::Firefox).unwrap();
        assert!(!firefox_manifest(&roots).exists());
    }

    #[test]
    fn sandboxed_dirs_are_reported_and_written() {
        let (_dir, roots) = make_roots();
        let snap_chrome = roots.home.join("snap/chromium/current/.config/chromium");
        std::fs::create_dir_all(&snap_chrome).unwrap();
        let snap_firefox = roots.home.join("snap/firefox/common/.mozilla");
        std::fs::create_dir_all(&snap_firefox).unwrap();

        let found = slots(&roots);
        let chrome = found
            .iter()
            .find(|s| s.browser == "Chromium")
            .expect("snap chromium detected");
        assert!(chrome.sandboxed);
        assert!(chrome.manifest_dir.starts_with(&snap_chrome));
        let firefox = found
            .iter()
            .find(|s| s.browser == "Firefox")
            .expect("snap firefox detected");
        assert!(firefox.sandboxed);
        assert!(firefox.manifest_dir.starts_with(&snap_firefox));

        write_for_roots(&roots, Family::Chromium, Path::new("/opt/subclave-proxy")).unwrap();
        write_for_roots(&roots, Family::Firefox, Path::new("/opt/subclave-proxy")).unwrap();
        assert!(snap_chrome
            .join("NativeMessagingHosts")
            .join(format!("{HOST_NAME}.json"))
            .is_file());
        assert!(snap_firefox
            .join("native-messaging-hosts")
            .join(format!("{HOST_NAME}.json"))
            .is_file());
    }
}
