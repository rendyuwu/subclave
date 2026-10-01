/**
 * localStorage shadow of the persisted custom theme. `applyCustomTheme`
 * writes it on every change; the boot fast path reads it before React mounts.
 * Kept dependency-free of `./apply` so neither module imports the other.
 */

import type { CustomTheme } from "./model";

const FAST_PATH_KEY = "subclave-custom-theme-shadow";

export function readShadow(): CustomTheme | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(FAST_PATH_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return null;
    // Accept either the new shape (`light`/`dark`) or the legacy single-mode
    // `colors` payload. The fast-path applier resolves the variant later.
    const obj = parsed as Record<string, unknown>;
    if (obj.light || obj.dark || obj.colors) return parsed as CustomTheme;
    return null;
  } catch {
    return null;
  }
}

export function writeShadow(theme: CustomTheme | null): void {
  try {
    if (!theme) {
      window.localStorage.removeItem(FAST_PATH_KEY);
      return;
    }
    // Strip the `data:` blob from the localStorage shadow. Idle memory stays
    // low (the shadow is read on every boot of the same webview) and
    // `applyCustomTheme` will re-add the dataUrl from the settings store
    // payload once it resolves.
    const slim = { ...theme, background: { ...theme.background, dataUrl: "" } };
    window.localStorage.setItem(FAST_PATH_KEY, JSON.stringify(slim));
  } catch {
    /* ignore */
  }
}
