import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { createRecoveredStore } from "@/lib/recoveredStore";
import type { StoreRecovery } from "@/lib/storeRecovery";
import type { KeyBinding, ShortcutId } from "@/modules/shortcuts/shortcuts";
import {
  AUTO_LOCK_MINUTES_MAX,
  clampOpacity,
  clampPref,
  CLIPBOARD_CLEAR_SECONDS_MAX,
  DEFAULT_PREFERENCES,
  KEY_APP_OPACITY,
  KEY_AUTOSTART,
  KEY_AUTO_LOCK_MINUTES,
  KEY_BRAND_COLOR,
  KEY_CLIPBOARD_CLEAR_SECONDS,
  KEY_CLOSE_TO_TRAY,
  KEY_CUSTOM_THEME,
  KEY_CUSTOM_THEME_ENABLED,
  KEY_GENERATOR,
  KEY_LOCK_ON_MINIMIZE,
  KEY_RESTORE_WINDOW,
  KEY_SHORTCUTS,
  KEY_THEME,
  KEY_USER_THEME_PRESETS,
  normalizeBrandColor,
  normalizeGeneratorOptions,
  PREF_STORE_KEYS,
  PREFS_CHANGED_EVENT,
  STORE_PATH,
  type Preferences,
  type PrefKey,
  type ThemePref,
} from "./schema";

import { normalizeCustomTheme } from "./theme/model";

/**
 * Cache invalidation for the store FILE, and nothing else.
 *
 * Separate from `PREFS_CHANGED_EVENT` because `commit()` emits its event
 * with no payload, and `onPreferencesChange` subscribers need the key and the
 * value. One event serving both would deliver twice per write, once empty.
 */
const PREFS_STORE_CHANGED_EVENT = "subclave://prefs-store-changed";

export const io = createRecoveredStore({
  path: STORE_PATH,
  loadKey: KEY_THEME,
  changedEvent: PREFS_STORE_CHANGED_EVENT,
});

/** Startup entry point, for `src/app/hooks/useStoreRecoveryNotices.ts`. */
export const ensureLoaded = (): Promise<StoreRecovery | null> => io.ensureLoaded();
export const takeRecoveryNotice = (): StoreRecovery | null => io.takeRecoveryNotice();
export const onSettingsStoreChanged = (cb: () => void): Promise<() => void> => io.onChanged(cb);

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
    autoLockMinutes: clampPref(
      get<unknown>(KEY_AUTO_LOCK_MINUTES),
      DEFAULT_PREFERENCES.autoLockMinutes,
      AUTO_LOCK_MINUTES_MAX,
    ),
    clipboardClearSeconds: clampPref(
      get<unknown>(KEY_CLIPBOARD_CLEAR_SECONDS),
      DEFAULT_PREFERENCES.clipboardClearSeconds,
      CLIPBOARD_CLEAR_SECONDS_MAX,
    ),
    lockOnMinimize: get<boolean>(KEY_LOCK_ON_MINIMIZE) ?? DEFAULT_PREFERENCES.lockOnMinimize,
    closeToTray: get<boolean>(KEY_CLOSE_TO_TRAY) ?? DEFAULT_PREFERENCES.closeToTray,
    generator: normalizeGeneratorOptions(get<unknown>(KEY_GENERATOR)),
  };
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
