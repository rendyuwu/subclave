import type { KeyBinding, ShortcutId } from "@/modules/shortcuts/shortcuts";
import type { GeneratorOptions } from "@/modules/vault/types";
import type { CustomTheme } from "./theme/model";
import { DEFAULT_CUSTOM_THEME } from "./themePresets";

export type ThemePref = "system" | "light" | "dark";

export type Preferences = {
  theme: ThemePref;
  autostart: boolean;
  restoreWindowState: boolean;
  shortcuts: Record<ShortcutId, KeyBinding[]>;
  /**
   * Brand color as 6-digit hex (`#RRGGBB`). Drives `--primary`, `--ring`,
   * `--sidebar-primary`, `--sidebar-ring`, and a derived `--accent`.
   * Default `#0057fe` (Subclave logo blue).
   */
  brandColor: string;
  /**
   * Custom theme overrides. When `customThemeEnabled` is true, the full color
   * token set (and background image) in `customTheme` is applied on top of
   * the base CSS variables. When false, only the brand color applies and
   * the base palette wins.
   */
  customThemeEnabled: boolean;
  customTheme: CustomTheme;
  /**
   * Whole-app transparency (0..1). The OS window is already transparent, so
   * lowering this fades EVERY surface toward the wallpaper image — or the
   * desktop when no image is set. 0 = fully see-through, 1 = solid (default).
   * Main window only; the settings window stays solid for readability.
   */
  appOpacity: number;
  /**
   * User-saved theme presets. Appear in the Theme settings preset grid
   * alongside the built-in `THEME_PRESETS`. The user "saves" the current
   * custom-theme state as a preset (with a chosen name); subsequent
   * tweaks to the live theme don't update the preset until they save
   * again. Items can be deleted individually.
   */
  userThemePresets: CustomTheme[];
  /**
   * Idle minutes before the vault locks itself; `0` = never. Rust reads the
   * same key from `subclave-settings.json` at unlock and touch time, so this
   * value must stay the on-disk one.
   */
  autoLockMinutes: number;
  /** Seconds before a copied secret is cleared from the clipboard; `0` = never. */
  clipboardClearSeconds: number;
  /** Lock the vault when the main window is minimized. */
  lockOnMinimize: boolean;
  /**
   * Closing the main window hides it instead of exiting. Default true, so the
   * app keeps running for the browser extension.
   */
  closeToTray: boolean;
  /** Last-used password-generator settings, reused by the browser extension. */
  generator: GeneratorOptions;
};

export const BRAND_COLOR_DEFAULT = "#0057fe";
const HEX6_RE = /^#([0-9a-f]{6})$/i;

export function normalizeBrandColor(value: string | undefined | null): string {
  if (!value) return BRAND_COLOR_DEFAULT;
  const trimmed = value.trim();
  if (HEX6_RE.test(trimmed)) return `#${trimmed.slice(1).toLowerCase()}`;
  // Accept 3-digit hex (#abc -> #aabbcc).
  const short = /^#([0-9a-f]{3})$/i.exec(trimmed);
  if (short) {
    const [r, g, b] = short[1].toLowerCase();
    return `#${r}${r}${g}${g}${b}${b}`;
  }
  return BRAND_COLOR_DEFAULT;
}

export const STORE_PATH = "subclave-settings.json";
export const KEY_THEME = "theme";
export const KEY_AUTOSTART = "autostart";
export const KEY_RESTORE_WINDOW = "restoreWindowState";
export const KEY_SHORTCUTS = "shortcuts";
export const KEY_BRAND_COLOR = "brandColor";
export const KEY_CUSTOM_THEME_ENABLED = "customThemeEnabled";
export const KEY_CUSTOM_THEME = "customTheme";
export const KEY_APP_OPACITY = "appOpacity";
export const KEY_USER_THEME_PRESETS = "userThemePresets";
export const KEY_AUTO_LOCK_MINUTES = "autoLockMinutes";
export const KEY_CLIPBOARD_CLEAR_SECONDS = "clipboardClearSeconds";
export const KEY_LOCK_ON_MINIMIZE = "lockOnMinimize";
export const KEY_CLOSE_TO_TRAY = "closeToTray";
export const KEY_GENERATOR = "generator";

export const APP_OPACITY_DEFAULT = 1;
// 0 = fully transparent (app dissolves into the wallpaper / desktop), 1 = solid.
export const APP_OPACITY_MIN = 0;
export const APP_OPACITY_MAX = 1;
export const APP_OPACITY_STEP = 0.05;

// The same clamps the Rust side applies when it reads this file, so the
// settings window and the vault agree on what is legal.
export const AUTO_LOCK_MINUTES_MAX = 1440;
export const CLIPBOARD_CLEAR_SECONDS_MAX = 600;
export const GENERATOR_LENGTH_MIN = 8;
export const GENERATOR_LENGTH_MAX = 128;

export const DEFAULT_PREFERENCES: Preferences = {
  theme: "system",
  autostart: false,
  restoreWindowState: true,
  shortcuts: {} as Record<ShortcutId, KeyBinding[]>,
  brandColor: BRAND_COLOR_DEFAULT,
  customThemeEnabled: false,
  customTheme: DEFAULT_CUSTOM_THEME,
  appOpacity: APP_OPACITY_DEFAULT,
  userThemePresets: [],
  autoLockMinutes: 10,
  clipboardClearSeconds: 30,
  lockOnMinimize: false,
  closeToTray: true,
  generator: {
    length: 20,
    lower: true,
    upper: true,
    digits: true,
    symbols: true,
    excludeAmbiguous: false,
  },
};

export type PrefKey = keyof Preferences;

// One entry per PrefKey. `satisfies Record<PrefKey, string>` turns a forgotten
// key into a COMPILE error instead of a silently-dropped cross-window update -
// a missing entry here was the documented root cause of the opacity/preset
// cross-window bugs (appOpacity + userThemePresets were the entries that got
// dropped). The reverse lookup the listeners need is derived below.
export const PREF_STORE_KEYS = {
  theme: KEY_THEME,
  autostart: KEY_AUTOSTART,
  restoreWindowState: KEY_RESTORE_WINDOW,
  shortcuts: KEY_SHORTCUTS,
  brandColor: KEY_BRAND_COLOR,
  customThemeEnabled: KEY_CUSTOM_THEME_ENABLED,
  customTheme: KEY_CUSTOM_THEME,
  // Written from the Settings window, consumed live by the main window.
  appOpacity: KEY_APP_OPACITY,
  userThemePresets: KEY_USER_THEME_PRESETS,
  autoLockMinutes: KEY_AUTO_LOCK_MINUTES,
  clipboardClearSeconds: KEY_CLIPBOARD_CLEAR_SECONDS,
  lockOnMinimize: KEY_LOCK_ON_MINIMIZE,
  closeToTray: KEY_CLOSE_TO_TRAY,
  generator: KEY_GENERATOR,
} satisfies Record<PrefKey, string>;

export function clampOpacity(value: number): number {
  if (!Number.isFinite(value)) return APP_OPACITY_DEFAULT;
  return Math.min(APP_OPACITY_MAX, Math.max(APP_OPACITY_MIN, value));
}

/**
 * A whole number in `0..=max`. `0` is legal and means "never"; a wrong-typed or
 * absent value falls back to the default instead of collapsing to 0, and an
 * out-of-range one is clamped, matching `prefs.rs` on the Rust side.
 */
export function clampPref(value: unknown, fallback: number, max: number): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(0, Math.trunc(value)));
}

/** Fill in any missing or wrong-typed generator field from the defaults. */
export function normalizeGeneratorOptions(value: unknown): GeneratorOptions {
  const raw = (value ?? {}) as Partial<Record<keyof GeneratorOptions, unknown>>;
  const fallback = DEFAULT_PREFERENCES.generator;
  const bool = (v: unknown, d: boolean): boolean => (typeof v === "boolean" ? v : d);
  return {
    length:
      typeof raw.length === "number" && Number.isFinite(raw.length)
        ? Math.min(GENERATOR_LENGTH_MAX, Math.max(GENERATOR_LENGTH_MIN, Math.trunc(raw.length)))
        : fallback.length,
    lower: bool(raw.lower, fallback.lower),
    upper: bool(raw.upper, fallback.upper),
    digits: bool(raw.digits, fallback.digits),
    symbols: bool(raw.symbols, fallback.symbols),
    excludeAmbiguous: bool(raw.excludeAmbiguous, fallback.excludeAmbiguous),
  };
}

/**
 * Emitted after every preferences write with `{ key, value }`. Declared here
 * rather than in `./mutations` so `./load` can listen for it without importing
 * the module that imports `./load`.
 */
export const PREFS_CHANGED_EVENT = "subclave://prefs-changed";
