//! The one definition of the browser socket address.
//!
//! The proxy and the app each compute the address independently; a drift
//! between the two is a dead channel with no error to explain it, so the
//! computation lives here once and both call it.
//!
//! `SUBCLAVE_BROWSER_SOCKET` overrides everything when it is set to an
//! absolute path. It is a test seam (the proxy relay tests and the Playwright
//! harness) read from the process environment, which a web page cannot
//! influence.

use std::path::PathBuf;

/// The app identifier the socket directory and pipe names are built from.
pub const APP_ID: &str = "dev.rendy.subclave";

/// The debug-build suffix, matching `tauri.dev.conf.json`'s identifier, so a
/// `pnpm tauri:dev` instance never collides with an installed release.
#[cfg(debug_assertions)]
const DEV_SUFFIX: &str = ".dev";
#[cfg(not(debug_assertions))]
const DEV_SUFFIX: &str = "";

/// The override variable name.
pub const SOCKET_ENV: &str = "SUBCLAVE_BROWSER_SOCKET";

fn override_path() -> Option<PathBuf> {
    let raw = std::env::var_os(SOCKET_ENV)?;
    let path = PathBuf::from(raw);
    path.is_absolute().then_some(path)
}

/// Absolute socket path, or the full pipe path on Windows.
pub fn socket_address() -> Option<PathBuf> {
    if let Some(path) = override_path() {
        return Some(path);
    }
    #[cfg(unix)]
    {
        let dir = socket_dir()?;
        Some(dir.join("browser.sock"))
    }
    #[cfg(windows)]
    {
        pipe_name().map(PathBuf::from)
    }
}

/// The directory that must be owned by the user with mode 0700 (Unix only, so
/// `None` on Windows and for an override whose parent is unknown).
pub fn socket_dir() -> Option<PathBuf> {
    if let Some(path) = override_path() {
        return path.parent().map(PathBuf::from);
    }
    #[cfg(unix)]
    {
        socket_dir_inner()
    }
    #[cfg(windows)]
    {
        None
    }
}

#[cfg(unix)]
fn socket_dir_inner() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        // macOS always sets TMPDIR for a GUI app; the fallback is the Unix one.
        if let Some(tmp) = std::env::var_os("TMPDIR") {
            let tmp = tmp.to_string_lossy();
            let trimmed = tmp.trim_end_matches('/');
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed).join(leaf_name()));
            }
        }
        return Some(fallback_dir());
    }
    #[cfg(not(target_os = "macos"))]
    {
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            let runtime = runtime.to_string_lossy();
            if !runtime.is_empty() {
                return Some(PathBuf::from(runtime.as_ref()).join(leaf_name()));
            }
        }
        Some(fallback_dir())
    }
}

#[cfg(unix)]
fn leaf_name() -> String {
    format!("{APP_ID}{DEV_SUFFIX}")
}

#[cfg(unix)]
fn fallback_dir() -> PathBuf {
    let uid = unsafe { libc::geteuid() };
    std::env::temp_dir().join(format!("{APP_ID}-{uid}{DEV_SUFFIX}"))
}

/// `\\.\pipe\dev.rendy.subclave-browser-<fnv1a(sid)>`, plus the debug suffix.
#[cfg(windows)]
pub fn pipe_name() -> Option<String> {
    let sid = current_user_sid()?;
    let hash = fnv1a(sid.as_bytes());
    Some(format!(r"\\.\pipe\{APP_ID}-browser-{hash:08x}{DEV_SUFFIX}"))
}

/// FNV-1a over the SID string. A uniqueness suffix, never a security
/// boundary; the pipe's DACL is.
#[cfg(windows)]
pub fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for b in bytes {
        hash ^= u32::from(*b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// The string form of the current token's user SID.
#[cfg(windows)]
fn current_user_sid() -> Option<String> {
    // Registry-based SID read is deliberately not used: the process token is
    // what the pipe DACL and the naming both derive from.
    crate::client::current_user_sid_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The override lives in the process environment, so the tests that move
    /// it must not run concurrently with one another.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env<T>(value: Option<&str>, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os(SOCKET_ENV);
        match value {
            Some(v) => std::env::set_var(SOCKET_ENV, v),
            None => std::env::remove_var(SOCKET_ENV),
        }
        let out = f();
        match previous {
            Some(v) => std::env::set_var(SOCKET_ENV, v),
            None => std::env::remove_var(SOCKET_ENV),
        }
        out
    }

    #[cfg(unix)]
    #[test]
    fn override_is_used_verbatim_when_absolute() {
        with_env(Some("/tmp/subclave-test-socket.sock"), || {
            assert_eq!(
                socket_address(),
                Some(PathBuf::from("/tmp/subclave-test-socket.sock"))
            );
            assert_eq!(socket_dir(), Some(PathBuf::from("/tmp")));
        });
    }

    #[cfg(unix)]
    #[test]
    fn default_address_ends_in_browser_sock() {
        with_env(None, || {
            let address = socket_address().expect("unix always resolves an address");
            assert!(address.ends_with("browser.sock"));
            assert_eq!(socket_dir().as_deref(), address.parent());
        });
    }
}
