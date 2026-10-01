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
  iconIdle: ["--subclave-icon-idle"],
  iconBlocked: ["--subclave-icon-blocked"],
  iconDone: ["--subclave-icon-done"],
  iconBranch: ["--subclave-icon-branch"],
  diffAdded: ["--subclave-diff-added"],
  diffRemoved: ["--subclave-diff-removed"],
  info: ["--subclave-info"],
  tabAccentTerminal: ["--subclave-tab-terminal"],
  tabAccentSsh: ["--subclave-tab-ssh"],
  tabAccentEditor: ["--subclave-tab-editor"],
  tabAccentPreview: ["--subclave-tab-browser"],
  tabAccentAiDiff: ["--subclave-tab-ai-diff"],
  tabAccentGitDiff: ["--subclave-tab-git-diff"],
  resizeHandle: ["--subclave-resize-handle"],
  ansiBlack: ["--subclave-ansi-black"],
  ansiRed: ["--subclave-ansi-red"],
  ansiGreen: ["--subclave-ansi-green"],
  ansiYellow: ["--subclave-ansi-yellow"],
  ansiBlue: ["--subclave-ansi-blue"],
  ansiMagenta: ["--subclave-ansi-magenta"],
  ansiCyan: ["--subclave-ansi-cyan"],
  ansiWhite: ["--subclave-ansi-white"],
  ansiBrightBlack: ["--subclave-ansi-bright-black"],
  ansiBrightRed: ["--subclave-ansi-bright-red"],
  ansiBrightGreen: ["--subclave-ansi-bright-green"],
  ansiBrightYellow: ["--subclave-ansi-bright-yellow"],
  ansiBrightBlue: ["--subclave-ansi-bright-blue"],
  ansiBrightMagenta: ["--subclave-ansi-bright-magenta"],
  ansiBrightCyan: ["--subclave-ansi-bright-cyan"],
  ansiBrightWhite: ["--subclave-ansi-bright-white"],
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
  // a minimal default so both light + dark variants are present before the
  // applier picks one by class.
  const defaults: CustomTheme = {
    name: "Default",
    light: SAFE_LIGHT_FALLBACK,
    dark: SAFE_DARK_FALLBACK,
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

/**
 * Minimal fallback palettes baked into this module so the fast path stays
 * synchronous and doesn't pull in `themePresets/` (which has the rich presets
 * and depends on this file's types). Any field missing from a legacy shadow
 * falls back to these.
 */
const SAFE_LIGHT_FALLBACK: ThemeColors = {
  background: "#ffffff",
  foreground: "#1f2328",
  card: "#f6f7f9",
  cardForeground: "#1f2328",
  popover: "#ffffff",
  popoverForeground: "#1f2328",
  button: "#0057fe",
  buttonForeground: "#ffffff",
  secondary: "#eceef2",
  secondaryForeground: "#1f2328",
  muted: "#f1f3f5",
  mutedForeground: "#6b7280",
  accent: "#dbe5ff",
  accentForeground: "#1f2328",
  destructive: "#dc2626",
  border: "#e4e7ec",
  input: "#dce1e7",
  buttonFace: "#b8babd",
  buttonFaceForeground: "#1f2328",
  ring: "#0057fe",
  sidebar: "#eef0f3",
  sidebarForeground: "#1f2328",
  sidebarBorder: "#e4e7ec",
  sidebarAccent: "#dbe5ff",
  sidebarAccentForeground: "#1f2328",
  iconWorking: "#ca8a04",
  iconIdle: "#059669",
  iconBlocked: "#dc2626",
  iconDone: "#2563eb",
  iconBranch: "#7c3aed",
  diffAdded: "#16a34a",
  diffRemoved: "#dc2626",
  info: "#0284c7",
  tabAccentTerminal: "#10b981",
  tabAccentSsh: "#0ea5e9",
  tabAccentEditor: "#0057fe",
  tabAccentPreview: "#06b6d4",
  tabAccentAiDiff: "#8b5cf6",
  tabAccentGitDiff: "#f59e0b",
  resizeHandle: "#e5e7eb",
  ansiBlack: "#3f3f46",
  ansiRed: "#dc2626",
  ansiGreen: "#16a34a",
  ansiYellow: "#ca8a04",
  ansiBlue: "#2563eb",
  ansiMagenta: "#9333ea",
  ansiCyan: "#0891b2",
  ansiWhite: "#e4e4e7",
  ansiBrightBlack: "#71717a",
  ansiBrightRed: "#ef4444",
  ansiBrightGreen: "#22c55e",
  ansiBrightYellow: "#eab308",
  ansiBrightBlue: "#3b82f6",
  ansiBrightMagenta: "#a855f7",
  ansiBrightCyan: "#06b6d4",
  ansiBrightWhite: "#fafafa",
};

const SAFE_DARK_FALLBACK: ThemeColors = {
  background: "#1a1a1a",
  foreground: "#cccccc",
  card: "#2b2b2b",
  cardForeground: "#cccccc",
  popover: "#363636",
  popoverForeground: "#e6e6e6",
  button: "#0057fe",
  buttonForeground: "#ffffff",
  secondary: "#3a3a3a",
  secondaryForeground: "#cccccc",
  muted: "#333333",
  mutedForeground: "#9d9d9d",
  accent: "#0a2870",
  accentForeground: "#ffffff",
  destructive: "#f14c4c",
  border: "#383838",
  input: "#3f3f3f",
  buttonFace: "#5d5d5d",
  buttonFaceForeground: "#e6e6e6",
  ring: "#0057fe",
  sidebar: "#141414",
  sidebarForeground: "#cccccc",
  sidebarBorder: "#383838",
  sidebarAccent: "#37373d",
  sidebarAccentForeground: "#ffffff",
  iconWorking: "#facc15",
  iconIdle: "#34d399",
  iconBlocked: "#f87171",
  iconDone: "#60a5fa",
  iconBranch: "#a78bfa",
  diffAdded: "#4ade80",
  diffRemoved: "#f87171",
  info: "#38bdf8",
  tabAccentTerminal: "#34d399",
  tabAccentSsh: "#38bdf8",
  tabAccentEditor: "#5b8bff",
  tabAccentPreview: "#22d3ee",
  tabAccentAiDiff: "#a78bfa",
  tabAccentGitDiff: "#fbbf24",
  resizeHandle: "#2b2b2b",
  ansiBlack: "#18181b",
  ansiRed: "#ef4444",
  ansiGreen: "#22c55e",
  ansiYellow: "#eab308",
  ansiBlue: "#3b82f6",
  ansiMagenta: "#a855f7",
  ansiCyan: "#06b6d4",
  ansiWhite: "#e4e4e7",
  ansiBrightBlack: "#52525b",
  ansiBrightRed: "#f87171",
  ansiBrightGreen: "#4ade80",
  ansiBrightYellow: "#facc15",
  ansiBrightBlue: "#60a5fa",
  ansiBrightMagenta: "#c084fc",
  ansiBrightCyan: "#22d3ee",
  ansiBrightWhite: "#fafafa",
};
