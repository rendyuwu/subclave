import { createContext, use, useCallback, useEffect, useMemo, useState } from "react";
import { readShadow, writeShadow } from "@/lib/fastPath";
import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  setAppOpacity,
  setCustomThemeEnabled,
  setTheme as persistTheme,
} from "@/modules/settings/mutations";
import type { ThemePref } from "@/modules/settings/schema";
import { applyBrandColor, applyBrandColorFastPath } from "@/modules/settings/brandColor";
import {
  applyAppOpacity,
  applyAppOpacityPreviewCss,
  onAppOpacityPreview,
} from "@/modules/settings/appOpacity";
import { applyBackground, applyCustomTheme } from "@/modules/settings/theme/apply";
import { onWallpaperPreview } from "@/modules/settings/theme/preview";

export type Theme = ThemePref;

type ThemeProviderState = {
  theme: Theme;
  resolvedTheme: "dark" | "light";
  setTheme: (theme: Theme) => void;
};

const ThemeProviderContext = createContext<ThemeProviderState | null>(null);

// Synchronous fast-path so the initial paint isn't unstyled. The persistent
// preference (in the settings store file) overwrites this as soon as the store
// hydrates; we keep a localStorage shadow of the *last applied* theme just for
// first-paint fidelity.
const FAST_PATH_KEY = "subclave-ui-theme-shadow";

function readFastTheme(): Theme {
  const v = readShadow(FAST_PATH_KEY);
  return v === "dark" || v === "light" || v === "system" ? v : "system";
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const hydrated = usePreferencesStore((s) => s.hydrated);
  const storedTheme = usePreferencesStore((s) => s.theme);
  const brandColor = usePreferencesStore((s) => s.brandColor);
  const customThemeEnabled = usePreferencesStore((s) => s.customThemeEnabled);
  const customTheme = usePreferencesStore((s) => s.customTheme);
  const appOpacity = usePreferencesStore((s) => s.appOpacity);

  // Before the store hydrates it still holds the defaults, so paint from the
  // shadow. Every window runs the store's `init()` (App / SettingsApp), which
  // keeps this provider in sync with writes from the other window.
  const [fastTheme] = useState<Theme>(readFastTheme);
  const theme = hydrated ? storedTheme : fastTheme;

  const [systemDark, setSystemDark] = useState<boolean>(() =>
    typeof window === "undefined"
      ? true
      : window.matchMedia("(prefers-color-scheme: dark)").matches,
  );

  useEffect(() => {
    if (hydrated) writeShadow(FAST_PATH_KEY, storedTheme);
  }, [hydrated, storedTheme]);

  // Colour layers. A custom theme owns the palette; otherwise the brand hex
  // re-tints the base. Order matters: clear any leftover custom-theme overrides
  // first so `clearCssVars()` does not wipe the `--primary` / `--ring` /
  // `--accent` values that `applyBrandColor` is about to set.
  useEffect(() => {
    if (!hydrated) return;
    if (customThemeEnabled) {
      applyCustomTheme(customTheme);
    } else {
      applyCustomTheme(null);
      applyBrandColor(brandColor);
    }
  }, [hydrated, customThemeEnabled, customTheme, brandColor]);

  // Wallpaper image is independent of the colour theme so it always paints when
  // set (won't vanish when the custom theme is off).
  useEffect(() => {
    if (hydrated) applyBackground(customTheme.background);
  }, [hydrated, customTheme]);

  useEffect(() => {
    if (hydrated) applyAppOpacity(appOpacity);
  }, [hydrated, appOpacity]);

  // Migration: a wallpaper only shows through translucent surfaces, and it only
  // paints while the custom theme is on. The unified opacity defaults to 1
  // (solid), so a wallpaper saved before the unify would silently vanish. Main
  // window only: if one is enabled, make sure the custom theme is on and
  // opacity drops below solid so it reappears. Persisted, so it self-corrects
  // just once, on the hydration that follows launch.
  useEffect(() => {
    if (!hydrated) return;
    if (document.getElementById("root") === null) return;
    const prefs = usePreferencesStore.getState();
    if (!prefs.customTheme.background.enabled || !prefs.customTheme.background.dataUrl) return;
    if (!prefs.customThemeEnabled) void setCustomThemeEnabled(true);
    if (prefs.appOpacity >= 1) void setAppOpacity(0.5);
  }, [hydrated]);

  // Live drag previews from the settings window: transient, applies CSS only -
  // no store write, so the slider thumb tracks smoothly. The settings window
  // has no wallpaper layer of its own, so it broadcasts just the numbers; we
  // merge them onto the wallpaper we already hold (no image blob crosses IPC)
  // and re-run the same applyBackground path the commit uses.
  useEffect(() => {
    const unlistenOpacity = onAppOpacityPreview((v) => applyAppOpacityPreviewCss(v));
    const unlistenWallpaper = onWallpaperPreview((p) => {
      const bg = usePreferencesStore.getState().customTheme.background;
      applyBackground({ ...bg, blur: p.blur, darken: p.darken, opacity: p.opacity });
    });
    return () => {
      void unlistenOpacity.then((fn) => fn());
      void unlistenWallpaper.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const resolvedTheme: "dark" | "light" =
    theme === "system" ? (systemDark ? "dark" : "light") : theme;

  useEffect(() => {
    const root = document.documentElement;
    root.classList.remove("light", "dark");
    root.classList.add(resolvedTheme);
    // Accent derivation differs per mode, so re-apply on theme flips. When a
    // custom theme is active it owns the palette and wins; otherwise fall back
    // to the cached brand hex (cheaper than re-hitting the store).
    const prefs = usePreferencesStore.getState();
    if (prefs.customThemeEnabled) applyCustomTheme(prefs.customTheme);
    else applyBrandColorFastPath();
  }, [resolvedTheme]);

  const setTheme = useCallback((next: Theme) => {
    // Optimistic: the store only learns the new value after the file write and
    // its broadcast, which would lag the toggle visibly. The write echoes back
    // through the same channel and lands on the same value.
    usePreferencesStore.setState({ theme: next });
    void persistTheme(next);
  }, []);

  const value = useMemo<ThemeProviderState>(
    () => ({ theme, resolvedTheme, setTheme }),
    [theme, resolvedTheme, setTheme],
  );

  return <ThemeProviderContext.Provider value={value}>{children}</ThemeProviderContext.Provider>;
}

export function useTheme(): ThemeProviderState {
  const ctx = use(ThemeProviderContext);
  if (!ctx) throw new Error("useTheme must be used within a <ThemeProvider>");
  return ctx;
}
