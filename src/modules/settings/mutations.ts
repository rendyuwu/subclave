import { emit } from "@tauri-apps/api/event";
import type { KeyBinding, ShortcutId } from "@/modules/shortcuts/shortcuts";
import type { GeneratorOptions } from "@/modules/vault/types";
import { io } from "./load";
import {
  AUTO_LOCK_MINUTES_MAX,
  CLIPBOARD_CLEAR_SECONDS_MAX,
  clampOpacity,
  clampPref,
  DEFAULT_PREFERENCES,
  KEY_APP_OPACITY,
  KEY_AUTOSTART,
  KEY_AUTO_LOCK_MINUTES,
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
  normalizeGeneratorOptions,
  PREFS_CHANGED_EVENT,
  type ThemePref,
} from "./schema";
import type { CustomTheme } from "./theme/model";

// Every setter mirrors its write through this event, so a consumer in another
// window gets the key AND the new value rather than only the news that the file
// moved. Declared in `./schema` so `./load` can listen for it without importing
// this module (which imports that one).

export async function writePref<T>(key: string, value: T): Promise<void> {
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

export async function setAutoLockMinutes(value: number): Promise<void> {
  await writePref(
    KEY_AUTO_LOCK_MINUTES,
    clampPref(value, DEFAULT_PREFERENCES.autoLockMinutes, AUTO_LOCK_MINUTES_MAX),
  );
}

export async function setClipboardClearSeconds(value: number): Promise<void> {
  await writePref(
    KEY_CLIPBOARD_CLEAR_SECONDS,
    clampPref(value, DEFAULT_PREFERENCES.clipboardClearSeconds, CLIPBOARD_CLEAR_SECONDS_MAX),
  );
}

export async function setLockOnMinimize(value: boolean): Promise<void> {
  await writePref(KEY_LOCK_ON_MINIMIZE, value);
}

export async function setCloseToTray(value: boolean): Promise<void> {
  await writePref(KEY_CLOSE_TO_TRAY, value);
}

export async function setGenerator(value: GeneratorOptions): Promise<void> {
  await writePref(KEY_GENERATOR, normalizeGeneratorOptions(value));
}
