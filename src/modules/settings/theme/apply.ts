/**
 * Custom theme runtime applier: writes the theme's colours onto `:root` and
 * paints the wallpaper layer behind the app.
 *
 * Stays compatible with the existing `brandColor` flow. When `customTheme`
 * is disabled, nothing is overridden and the brand color path keeps owning
 * `--primary` / `--ring` / `--accent`.
 */

import { isSecondaryWindow } from "@/lib/platform";
import {
  normalizeCustomTheme,
  type CustomTheme,
  type ThemeBackground,
  type ThemeColors,
} from "./model";
import { readShadow, writeShadow } from "./shadow";
import { DARK_COLORS, LIGHT_COLORS } from "../themePresets/base";

const BG_ELEMENT_ID = "subclave-bg-layer";

/**
 * Mapping of `ThemeColors` keys to the CSS variables they drive.
 * Multiple variables can be backed by a single token (e.g. `button` writes
 * both `--primary` and `--sidebar-primary`).
 */
const COLOR_VAR_MAP: Record<keyof ThemeColors, readonly string[]> = {
  background: ["--background"],
  foreground: ["--foreground"],
  card: ["--card"],
  cardForeground: ["--card-foreground"],
  popover: ["--popover"],
  popoverForeground: ["--popover-foreground"],
  button: ["--primary", "--sidebar-primary"],
  buttonForeground: ["--primary-foreground", "--sidebar-primary-foreground"],
  secondary: ["--secondary"],
  secondaryForeground: ["--secondary-foreground"],
  muted: ["--muted"],
  mutedForeground: ["--muted-foreground"],
  accent: ["--accent"],
  accentForeground: ["--accent-foreground"],
  destructive: ["--destructive"],
  border: ["--border"],
  input: ["--input"],
  buttonFace: ["--subclave-button-face"],
  buttonFaceForeground: ["--subclave-button-face-foreground"],
  ring: ["--ring", "--sidebar-ring"],
  sidebar: ["--sidebar"],
  sidebarForeground: ["--sidebar-foreground"],
  sidebarBorder: ["--sidebar-border"],
  sidebarAccent: ["--sidebar-accent"],
  sidebarAccentForeground: ["--sidebar-accent-foreground"],
  iconWorking: ["--subclave-icon-working"],
  diffAdded: ["--subclave-diff-added"],
  info: ["--subclave-info"],
  resizeHandle: ["--subclave-resize-handle"],
  ansiRed: ["--subclave-ansi-red"],
  ansiGreen: ["--subclave-ansi-green"],
  ansiYellow: ["--subclave-ansi-yellow"],
  ansiBlue: ["--subclave-ansi-blue"],
  ansiMagenta: ["--subclave-ansi-magenta"],
  ansiCyan: ["--subclave-ansi-cyan"],
};

function ensureBgElement(): HTMLDivElement | null {
  if (typeof document === "undefined") return null;
  let el = document.getElementById(BG_ELEMENT_ID) as HTMLDivElement | null;
  if (!el) {
    el = document.createElement("div");
    el.id = BG_ELEMENT_ID;
    el.setAttribute("aria-hidden", "true");
    el.style.position = "fixed";
    el.style.inset = "0";
    el.style.zIndex = "-1";
    el.style.pointerEvents = "none";
    el.style.overflow = "hidden";
    el.style.backgroundSize = "cover";
    el.style.backgroundPosition = "center";
    el.style.backgroundRepeat = "no-repeat";
    document.body.prepend(el);
  }
  return el;
}

function clearCssVars(): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  for (const vars of Object.values(COLOR_VAR_MAP)) {
    for (const v of vars) root.style.removeProperty(v);
  }
  // Clean up the canvas base colour; the globals.css default takes over.
  root.style.removeProperty("--subclave-canvas-bg");
}

// Wallpaper is intentionally main-window only (isSecondaryWindow): the utility
// windows have their own roots and we don't want a busy image behind their
// controls. Colors still apply so their UI stays in palette.
/**
 * Paint (or clear) the wallpaper layer behind the app. A static image only:
 * the CSP blocks remote images, media and frames, and the source is a local
 * file read as a `data:` URL by `fs_read_file`. Independent of the colour
 * theme: it shows whenever a source is set, regardless of
 * `customThemeEnabled`. Transparency itself is the single "App opacity"
 * control; the translucent surfaces reveal this layer (or the desktop when
 * none is set). Settings window opts out.
 */
export function applyBackground(bg: ThemeBackground): void {
  if (typeof document === "undefined") return;

  if (isSecondaryWindow()) {
    const existing = document.getElementById(BG_ELEMENT_ID);
    if (existing) existing.remove();
    return;
  }

  const el = ensureBgElement();
  if (!el) return;

  // Opacity of the whole image layer: lets the desktop behind the (transparent)
  // window show THROUGH the wallpaper.
  el.style.opacity = String(Math.max(0, Math.min(1, bg.opacity ?? 1)));

  if (!bg.enabled || !bg.dataUrl) {
    el.style.backgroundImage = "";
    el.style.filter = "";
    if (bg.enabled && !bg.dataUrl) {
      // Wallpaper is configured but its data: blob hasn't been restored yet:
      // the localStorage fast-path shadow strips data: URLs (see writeShadow),
      // so the first frame after a reload has no image. Paint the theme canvas
      // colour as a placeholder so glass surfaces fade toward the THEME
      // background instead of the bare desktop bleeding through, until the
      // async store load re-applies the real image. `--subclave-canvas-bg` is set
      // synchronously on :root before this runs in the fast path.
      el.style.display = "block";
      el.style.backgroundColor = "var(--subclave-canvas-bg)";
    } else {
      el.style.display = "none";
      el.style.backgroundColor = "";
    }
    return;
  }

  el.style.display = "block";
  // Clear any placeholder colour now that a real wallpaper is painting.
  el.style.backgroundColor = "";
  const blur = bg.blur > 0 ? `blur(${Math.max(0, Math.min(40, bg.blur))}px)` : "";
  const darken = Math.max(0, Math.min(1, bg.darken ?? 0));
  const safeUrl = bg.dataUrl.replace(/"/g, '\\"');
  const overlay =
    darken > 0 ? `linear-gradient(rgba(0,0,0,${darken}), rgba(0,0,0,${darken})), ` : "";
  el.style.backgroundImage = `${overlay}url("${safeUrl}")`;
  el.style.filter = blur;
}

/**
 * Apply a `CustomTheme`'s colours to the document. Pass `null` to clear
 * overrides and let the base CSS variables (and `brandColor`) take over again.
 * The wallpaper image is handled separately by `applyBackground` so it is not
 * tied to whether the custom theme is enabled.
 */
export function applyCustomTheme(theme: CustomTheme | null): void {
  if (typeof document === "undefined") return;
  if (!theme) {
    clearCssVars();
    writeShadow(null);
    return;
  }
  const root = document.documentElement;
  // Pick the variant matching the resolved theme: the `.dark` class on
  // <html> is set by ThemeProvider whenever resolvedTheme flips.
  const isDark = root.classList.contains("dark");
  const colors = isDark ? theme.dark : theme.light;
  for (const key of Object.keys(COLOR_VAR_MAP) as Array<keyof ThemeColors>) {
    const value = colors[key];
    if (!value) continue;
    for (const v of COLOR_VAR_MAP[key]) {
      root.style.setProperty(v, value);
    }
  }
  // Canvas base colour the glass tint mixes against (editor/terminal rgba +
  // the panel tints). Removed by clearCssVars when the custom theme turns off,
  // so the base palette default in globals.css takes over.
  root.style.setProperty("--subclave-canvas-bg", colors.background);
  writeShadow(theme);
}

/**
 * Synchronous fast path. Call before React mounts so the first paint uses the
 * persisted custom theme instead of the base palette. Lives here rather than
 * next to the shadow store so `./shadow` never imports this module.
 */
export function applyCustomThemeFastPath(): void {
  const cached = readShadow();
  if (!cached) return;
  // Legacy shadows may still have the old `colors` shape. Normalize against
  // the base palette so both light + dark variants are present before the
  // applier picks one by class.
  const defaults: CustomTheme = {
    name: "Default",
    light: LIGHT_COLORS,
    dark: DARK_COLORS,
    background: {
      enabled: false,
      path: "",
      dataUrl: "",
      blur: 0,
      darken: 0,
      opacity: 1,
    },
  };
  const normalized = normalizeCustomTheme(cached, defaults);
  applyCustomTheme(normalized);
  // Paint the wallpaper on first frame too; a local data: URL is restored
  // after the async store load.
  applyBackground(normalized.background);
}
