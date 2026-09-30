import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { createRecoveredStore } from "@/lib/recoveredStore";
import type { StoreRecovery } from "@/lib/storeRecovery";
import type { KeyBinding, ShortcutId } from "@/modules/shortcuts/shortcuts";
import { normalizeCustomTheme, type CustomTheme } from "./customTheme";
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

const STORE_PATH = "subclave-settings.json";
const KEY_THEME = "theme";
const KEY_AUTOSTART = "autostart";
const KEY_RESTORE_WINDOW = "restoreWindowState";
const KEY_SHORTCUTS = "shortcuts";
const KEY_BRAND_COLOR = "brandColor";
const KEY_CUSTOM_THEME_ENABLED = "customThemeEnabled";
const KEY_CUSTOM_THEME = "customTheme";
const KEY_APP_OPACITY = "appOpacity";
const KEY_USER_THEME_PRESETS = "userThemePresets";

export const APP_OPACITY_DEFAULT = 1;
// 0 = fully transparent (app dissolves into the wallpaper / desktop), 1 = solid.
export const APP_OPACITY_MIN = 0;
export const APP_OPACITY_MAX = 1;
export const APP_OPACITY_STEP = 0.05;

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
};

export type PrefKey = keyof Preferences;

// One entry per PrefKey. `satisfies Record<PrefKey, string>` turns a forgotten
// key into a COMPILE error instead of a silently-dropped cross-window update -
// a missing entry here was the documented root cause of the opacity/preset
// cross-window bugs (appOpacity + userThemePresets were the entries that got
// dropped). The reverse lookup the listeners need is derived below.
const PREF_STORE_KEYS = {
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
} satisfies Record<PrefKey, string>;

/**
 * Cache invalidation for the store FILE, and nothing else.
 *
 * Separate from {@link PREFS_CHANGED_EVENT} because `commit()` emits its event
 * with no payload, and `onPreferencesChange` subscribers need the key and the
 * value. One event serving both would deliver twice per write, once empty.
 */
const PREFS_STORE_CHANGED_EVENT = "subclave://prefs-store-changed";

const io = createRecoveredStore({
  path: STORE_PATH,
  loadKey: KEY_THEME,
  changedEvent: PREFS_STORE_CHANGED_EVENT,
});

/** Startup entry point, for `src/app/hooks/useStoreRecoveryNotices.ts`. */
export const ensureLoaded = (): Promise<StoreRecovery | null> => io.ensureLoaded();
export const takeRecoveryNotice = (): StoreRecovery | null => io.takeRecoveryNotice();
export const onSettingsStoreChanged = (cb: () => void): Promise<() => void> => io.onChanged(cb);

// Every setter mirrors its write through this event, so a consumer in another
// window gets the key AND the new value rather than only the news that the file
// moved.
const PREFS_CHANGED_EVENT = "subclave://prefs-changed";

async function writePref<T>(key: string, value: T): Promise<void> {
  // Through the port's queue: it is the only queue per store file, and two
  // setters firing as the user leaves two fields is the ordinary case.
  await io.enqueueWrite(async () => {
    await io.set(key, value);
    await io.commit();
  });
  // AFTER the commit, so a listener that re-reads sees the bytes. Tauri v2
  // self-delivers `emit()`, so this window's own listener fires exactly once
  // and every other window once. The webview-label dedupe this replaces
  // suppressed only the WRITER's duplicate: the old plugin store's `onChange`
  // was a broadcast to every window, so a non-writing window fired twice. One
  // channel removes that too.
  await emit(PREFS_CHANGED_EVENT, { key, value });
}

export async function loadPreferences(): Promise<Preferences> {
  // No file read at all: the port's settle pass already forced the load, and
  // `createFileKeyValueStore` serves every later `get` from the whole-file cache
  // it installed. A cold path would still cost ONE `fs_read_file` for all
  // entries, because the load shares its in-flight promise.
  const entries = await Promise.all(
    Object.values(PREF_STORE_KEYS).map(async (k) => [k, await io.get<unknown>(k)] as const),
  );
  const map = new Map<string, unknown>(entries);
  // `RecoveredStoreIo.get` coerces a missing key to `null` where `Map.get` gave
  // `undefined`. Normalised back here rather than at every call site: the rest
  // of the body already tolerates `null` (`normalizeBrandColor` takes
  // `string | undefined | null`, `normalizeCustomTheme` takes `unknown`), so
  // this is one less shape to reason about rather than a fix for a break.
  const get = <T>(k: string): T | undefined => (map.get(k) ?? undefined) as T | undefined;
  return {
    theme: get<ThemePref>(KEY_THEME) ?? DEFAULT_PREFERENCES.theme,
    autostart: get<boolean>(KEY_AUTOSTART) ?? DEFAULT_PREFERENCES.autostart,
    restoreWindowState: get<boolean>(KEY_RESTORE_WINDOW) ?? DEFAULT_PREFERENCES.restoreWindowState,
    shortcuts:
      get<Record<ShortcutId, KeyBinding[]>>(KEY_SHORTCUTS) ?? DEFAULT_PREFERENCES.shortcuts,
    brandColor: normalizeBrandColor(get<string>(KEY_BRAND_COLOR)),
    customThemeEnabled:
      get<boolean>(KEY_CUSTOM_THEME_ENABLED) ?? DEFAULT_PREFERENCES.customThemeEnabled,
    customTheme: normalizeCustomTheme(
      get<unknown>(KEY_CUSTOM_THEME),
      DEFAULT_PREFERENCES.customTheme,
    ),
    appOpacity: clampOpacity(get<number>(KEY_APP_OPACITY) ?? DEFAULT_PREFERENCES.appOpacity),
    userThemePresets: (() => {
      const raw = get<unknown>(KEY_USER_THEME_PRESETS);
      if (!Array.isArray(raw)) return DEFAULT_PREFERENCES.userThemePresets;
      // Normalise each entry through `normalizeCustomTheme` so a corrupt
      // / partial preset doesn't crash the settings page on load.
      return raw.flatMap((entry) => {
        const p = normalizeCustomTheme(entry, DEFAULT_PREFERENCES.customTheme);
        return typeof p.name === "string" && p.name.length > 0 ? [p] : [];
      });
    })(),
  };
}

export function clampOpacity(value: number): number {
  if (!Number.isFinite(value)) return APP_OPACITY_DEFAULT;
  return Math.min(APP_OPACITY_MAX, Math.max(APP_OPACITY_MIN, value));
}

export async function setTheme(value: ThemePref): Promise<void> {
  await writePref(KEY_THEME, value);
}

export async function setAppOpacity(value: number): Promise<void> {
  await writePref(KEY_APP_OPACITY, clampOpacity(value));
}

export async function setAutostart(value: boolean): Promise<void> {
  await writePref(KEY_AUTOSTART, value);
}

export async function setRestoreWindowState(value: boolean): Promise<void> {
  await writePref(KEY_RESTORE_WINDOW, value);
}

export async function setShortcuts(value: Record<ShortcutId, KeyBinding[]> | {}): Promise<void> {
  await writePref(KEY_SHORTCUTS, value);
}

export async function setCustomThemeEnabled(value: boolean): Promise<void> {
  await writePref(KEY_CUSTOM_THEME_ENABLED, value);
}

export async function setCustomTheme(value: CustomTheme): Promise<void> {
  await writePref(KEY_CUSTOM_THEME, value);
}

export async function setUserThemePresets(value: CustomTheme[]): Promise<void> {
  await writePref(KEY_USER_THEME_PRESETS, value);
}

/**
 * Subscribe to changes from any window (settings to main).
 *
 * ONE channel, not two. Every write goes through `writePref`, which emits
 * PREFS_CHANGED_EVENT after its commit; Tauri v2 self-delivers `emit()`, so the
 * writing window and every other window each see it exactly once and no dedupe
 * is needed.
 */
export async function onPreferencesChange(
  cb: (key: PrefKey, value: unknown) => void,
): Promise<UnlistenFn> {
  const map = Object.fromEntries(
    Object.entries(PREF_STORE_KEYS).map(([pref, storeKey]) => [storeKey, pref as PrefKey]),
  ) as Record<string, PrefKey>;
  return listen<{ key: string; value: unknown }>(PREFS_CHANGED_EVENT, (e) => {
    const mapped = map[e.payload.key];
    if (mapped) cb(mapped, e.payload.value);
  });
}
