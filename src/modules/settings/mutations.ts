import { emit } from "@tauri-apps/api/event";
import { browserIntegrationSet } from "@/modules/browser/ipc";
import type { KeyBinding, ShortcutId } from "@/modules/shortcuts/shortcuts";
import type { GeneratorOptions } from "@/modules/vault/types";
import { io } from "./load";
import { usePreferencesStore } from "./preferences";
import {
  AUTO_LOCK_MINUTES_MAX,
  CLIPBOARD_CLEAR_SECONDS_MAX,
  clampOpacity,
  clampPref,
  DEFAULT_PREFERENCES,
  normalizeBrowserPrefs,
  normalizeGeneratorOptions,
  PREF_STORE_KEYS,
  PREFS_CHANGED_EVENT,
  type BrowserPrefs,
  type ThemePref,
} from "./schema";
import type { CustomTheme } from "./theme/model";

// Every setter mirrors its write through this event, so a consumer in another
// window gets the key AND the new value rather than only the news that the file
// moved. Declared in `./schema` so `./load` can listen for it without importing
// this module (which imports that one).

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

export async function setTheme(value: ThemePref): Promise<void> {
  await writePref(PREF_STORE_KEYS.theme, value);
}

export async function setAppOpacity(value: number): Promise<void> {
  await writePref(PREF_STORE_KEYS.appOpacity, clampOpacity(value));
}

export async function setAutostart(value: boolean): Promise<void> {
  await writePref(PREF_STORE_KEYS.autostart, value);
}

export async function setRestoreWindowState(value: boolean): Promise<void> {
  await writePref(PREF_STORE_KEYS.restoreWindowState, value);
}

export async function setShortcuts(value: Record<ShortcutId, KeyBinding[]> | {}): Promise<void> {
  await writePref(PREF_STORE_KEYS.shortcuts, value);
}

export async function setCustomThemeEnabled(value: boolean): Promise<void> {
  await writePref(PREF_STORE_KEYS.customThemeEnabled, value);
}

export async function setCustomTheme(value: CustomTheme): Promise<void> {
  await writePref(PREF_STORE_KEYS.customTheme, value);
}

export async function setUserThemePresets(value: CustomTheme[]): Promise<void> {
  await writePref(PREF_STORE_KEYS.userThemePresets, value);
}

export async function setAutoLockMinutes(value: number): Promise<void> {
  await writePref(
    PREF_STORE_KEYS.autoLockMinutes,
    clampPref(value, DEFAULT_PREFERENCES.autoLockMinutes, AUTO_LOCK_MINUTES_MAX),
  );
}

export async function setClipboardClearSeconds(value: number): Promise<void> {
  await writePref(
    PREF_STORE_KEYS.clipboardClearSeconds,
    clampPref(value, DEFAULT_PREFERENCES.clipboardClearSeconds, CLIPBOARD_CLEAR_SECONDS_MAX),
  );
}

export async function setLockOnMinimize(value: boolean): Promise<void> {
  await writePref(PREF_STORE_KEYS.lockOnMinimize, value);
}

export async function setCloseToTray(value: boolean): Promise<void> {
  await writePref(PREF_STORE_KEYS.closeToTray, value);
}

export async function setGenerator(value: GeneratorOptions): Promise<void> {
  await writePref(PREF_STORE_KEYS.generator, normalizeGeneratorOptions(value));
}

/**
 * Flip one browser family's switch, then write (or remove) that family's native
 * messaging manifests.
 *
 * The preference lands FIRST: the Rust side reads `browser` from the settings
 * file, so a later startup refresh re-writes the manifests for every enabled
 * family. The command's own rejection propagates, because the manifest write is
 * what the user asked for and the Settings tab reports it.
 */
export async function setBrowserFamily(
  family: keyof BrowserPrefs,
  enabled: boolean,
): Promise<void> {
  const current = usePreferencesStore.getState().browser;
  await writePref(
    PREF_STORE_KEYS.browser,
    normalizeBrowserPrefs({ ...current, [family]: enabled }),
  );
  await browserIntegrationSet(family, enabled);
}
