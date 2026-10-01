//! User preferences read from `subclave-settings.json`.
//!
//! The Settings window writes a flat JSON map of camelCase keys through the
//! webview's `appDataDir()`, which resolves to the same directory Tauri hands
//! [`read`]. Reading NEVER fails: a missing file, invalid JSON, or a
//! wrong-typed value falls back to that key's default, so a broken settings
//! file can never keep the vault from unlocking.

use std::path::Path;

use crate::modules::generator::GeneratorOptions;

/// File name under the app data dir, matching the Settings store's writer.
pub const SETTINGS_FILE_NAME: &str = "subclave-settings.json";

/// Clamp ceilings. `0` is a legal value meaning "never" and passes through.
const MAX_AUTO_LOCK_MINUTES: u64 = 1440;
const MAX_CLIPBOARD_CLEAR_SECONDS: u64 = 600;

/// The preferences the Rust side reads at use time. The field docs name the
/// on-disk key.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// `autoLockMinutes`, 0 to 1440. 0 = never.
    pub auto_lock_minutes: u64,
    /// `clipboardClearSeconds`, 0 to 600. 0 = never.
    pub clipboard_clear_seconds: u64,
    /// `lockOnMinimize`.
    pub lock_on_minimize: bool,
    /// `closeToTray`.
    pub close_to_tray: bool,
    /// `browser.chromium`: the Chromium-family native messaging switch.
    pub browser_chromium: bool,
    /// `browser.firefox`: the Firefox native messaging switch.
    pub browser_firefox: bool,
    /// `generator`, the password generator options the browser's
    /// `generate-password` shares with the app's popover.
    pub generator: GeneratorOptions,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            auto_lock_minutes: 10,
            clipboard_clear_seconds: 30,
            lock_on_minimize: false,
            close_to_tray: true,
            browser_chromium: false,
            browser_firefox: false,
            generator: GeneratorOptions::default(),
        }
    }
}

/// A number for `key`, clamped to `0..=max`. A non-number falls back to
/// `default`; a number out of range clamps rather than refuses, because the
/// file is written by a sibling window, not by an attacker. A JSON number
/// that is negative clamps up to `0`, so "off" survives.
fn number(value: &serde_json::Value, key: &str, default: u64, max: u64) -> u64 {
    match value.get(key) {
        Some(serde_json::Value::Number(n)) => match (n.as_u64(), n.as_i64()) {
            (Some(u), _) => u.min(max),
            (None, Some(i)) => i.clamp(0, max as i64) as u64,
            _ => default,
        },
        _ => default,
    }
}

/// A bool for `key`; anything else falls back to `default`.
fn flag(value: &serde_json::Value, key: &str, default: bool) -> bool {
    value.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

/// The generator options from the `generator` object, each key falling back to
/// the generator's own default and clamped to its limits, so the browser's
/// `generate-password` and the app's popover always agree.
fn generator_options(value: &serde_json::Value, fallback: &GeneratorOptions) -> GeneratorOptions {
    let object = value.get("generator").unwrap_or(&serde_json::Value::Null);
    GeneratorOptions {
        length: number(object, "length", u64::from(fallback.length), 128).max(8) as u32,
        lower: flag(object, "lower", fallback.lower),
        upper: flag(object, "upper", fallback.upper),
        digits: flag(object, "digits", fallback.digits),
        symbols: flag(object, "symbols", fallback.symbols),
        exclude_ambiguous: flag(object, "excludeAmbiguous", fallback.exclude_ambiguous),
    }
}

/// Read the preferences for `dir`, falling back per key.
pub(crate) fn read(dir: &Path) -> Prefs {
    let fallback = Prefs::default();
    let Ok(text) = std::fs::read_to_string(dir.join(SETTINGS_FILE_NAME)) else {
        return fallback;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return fallback;
    };
    let browser = value.get("browser").unwrap_or(&serde_json::Value::Null);
    Prefs {
        auto_lock_minutes: number(
            &value,
            "autoLockMinutes",
            fallback.auto_lock_minutes,
            MAX_AUTO_LOCK_MINUTES,
        ),
        clipboard_clear_seconds: number(
            &value,
            "clipboardClearSeconds",
            fallback.clipboard_clear_seconds,
            MAX_CLIPBOARD_CLEAR_SECONDS,
        ),
        lock_on_minimize: flag(&value, "lockOnMinimize", fallback.lock_on_minimize),
        close_to_tray: flag(&value, "closeToTray", fallback.close_to_tray),
        browser_chromium: flag(browser, "chromium", fallback.browser_chromium),
        browser_firefox: flag(browser, "firefox", fallback.browser_firefox),
        generator: generator_options(&value, &fallback.generator),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subclave-prefs-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id(),
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_settings(dir: &TempDir, text: &str) {
        std::fs::write(dir.0.join(SETTINGS_FILE_NAME), text).unwrap();
    }

    #[test]
    fn missing_file_is_all_defaults() {
        let dir = TempDir::new("missing");
        assert_eq!(read(&dir.0), Prefs::default());
    }

    #[test]
    fn invalid_json_is_all_defaults() {
        let dir = TempDir::new("badjson");
        write_settings(&dir, "{not json");
        assert_eq!(read(&dir.0), Prefs::default());
    }

    #[test]
    fn wrong_typed_values_fall_back_per_key() {
        let dir = TempDir::new("types");
        write_settings(
            &dir,
            r#"{"autoLockMinutes":"ten","clipboardClearSeconds":[],"lockOnMinimize":"yes","closeToTray":1}"#,
        );
        assert_eq!(read(&dir.0), Prefs::default());
    }

    #[test]
    fn numbers_clamp_and_zero_survives() {
        let dir = TempDir::new("clamp");
        write_settings(
            &dir,
            r#"{"autoLockMinutes":99999,"clipboardClearSeconds":-5}"#,
        );
        let prefs = read(&dir.0);
        assert_eq!(prefs.auto_lock_minutes, 1440);
        assert_eq!(prefs.clipboard_clear_seconds, 0);
        write_settings(&dir, r#"{"autoLockMinutes":0,"clipboardClearSeconds":0}"#);
        let prefs = read(&dir.0);
        assert_eq!(prefs.auto_lock_minutes, 0);
        assert_eq!(prefs.clipboard_clear_seconds, 0);
    }

    #[test]
    fn values_and_flags_are_read() {
        let dir = TempDir::new("read");
        write_settings(
            &dir,
            r#"{"autoLockMinutes":5,"clipboardClearSeconds":45,"lockOnMinimize":true,"closeToTray":false}"#,
        );
        assert_eq!(
            read(&dir.0),
            Prefs {
                auto_lock_minutes: 5,
                clipboard_clear_seconds: 45,
                lock_on_minimize: true,
                close_to_tray: false,
                browser_chromium: false,
                browser_firefox: false,
                generator: GeneratorOptions::default(),
            }
        );
    }

    #[test]
    fn browser_flags_are_read_from_the_browser_object() {
        let dir = TempDir::new("browser");
        write_settings(&dir, r#"{"browser":{"chromium":true,"firefox":false}}"#);
        let prefs = read(&dir.0);
        assert!(prefs.browser_chromium);
        assert!(!prefs.browser_firefox);

        // A half-written or wrongly typed object falls back to false.
        write_settings(&dir, r#"{"browser":{"chromium":"yes"}}"#);
        let prefs = read(&dir.0);
        assert!(!prefs.browser_chromium);
        write_settings(&dir, r#"{"browser":1}"#);
        assert!(!read(&dir.0).browser_chromium);
    }

    #[test]
    fn generator_options_round_trip_and_clamp() {
        let dir = TempDir::new("generator");
        write_settings(
            &dir,
            r#"{"generator":{"length":32,"lower":false,"upper":false,"digits":true,"symbols":true,"excludeAmbiguous":true}}"#,
        );
        let prefs = read(&dir.0);
        assert_eq!(prefs.generator.length, 32);
        assert!(!prefs.generator.lower);
        assert!(!prefs.generator.upper);
        assert!(prefs.generator.digits);
        assert!(prefs.generator.symbols);
        assert!(prefs.generator.exclude_ambiguous);

        // Out-of-range numbers clamp to the generator's own limits.
        write_settings(&dir, r#"{"generator":{"length":500}}"#);
        assert_eq!(read(&dir.0).generator.length, 128);
        write_settings(&dir, r#"{"generator":{"length":2}}"#);
        assert_eq!(read(&dir.0).generator.length, 8);
        // A missing object keeps every default.
        write_settings(&dir, "{}");
        assert_eq!(read(&dir.0).generator, GeneratorOptions::default());
    }
}
