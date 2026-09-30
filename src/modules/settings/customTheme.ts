/**
 * Custom theme runtime applier.
 *
 * Persisted shape lives in `store.ts` (`Preferences.customTheme`). When
 * enabled, this module overrides the CSS variables on `:root` and renders a
 * fixed-position background image div behind the app surfaces.
 *
 * Stays compatible with the existing `brandColor` flow. When `customTheme`
 * is disabled, nothing is overridden and the brand color path keeps owning
 * `--primary` / `--ring` / `--accent`.
 */

import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { isSecondaryWindow } from "@/lib/platform";
import { ensureVisibleButtonFace } from "./buttonFace";

export type ThemeColors = {
  /** App-wide canvas (`--background`). */
  background: string;
  /** Default text color (`--foreground`). */
  foreground: string;
  /** Card / panel surface (`--card`). */
  card: string;
  /** Card / panel text (`--card-foreground`). */
  cardForeground: string;
  /** Popover surface (`--popover`). */
  popover: string;
  /** Popover text (`--popover-foreground`). */
  popoverForeground: string;
  /** Buttons + accents (`--primary`). Layered with brandColor; this wins. */
  button: string;
  /** Button text (`--primary-foreground`). */
  buttonForeground: string;
  /** Subtle surface (`--secondary`). */
  secondary: string;
  /** Subtle surface text (`--secondary-foreground`). */
  secondaryForeground: string;
  /** Muted text + surface (`--muted` / `--muted-foreground`). */
  muted: string;
  mutedForeground: string;
  /** Soft accent fill for popovers / dropdowns / active tab (`--accent`). */
  accent: string;
  accentForeground: string;
  /** Destructive (`--destructive`). */
  destructive: string;
  /** Default border color across panels (`--border`). */
  border: string;
  /** Input field border (`--input`). */
  input: string;
  /** Fill of the neutral (`outline`) button. It carries no border, so this is
   *  the whole affordance; held to a contrast floor by `buttonFace.ts`. */
  buttonFace: string;
  /** Label on `buttonFace`, held to WCAG AA. Its own key because a face with
   *  good surface contrast can still leave `foreground` unreadable. */
  buttonFaceForeground: string;
  /** Focus bar / focus ring on tab and inputs (`--ring`, also `::after` on active tab). */
  ring: string;
  /** Sidebar surface. */
  sidebar: string;
  sidebarForeground: string;
  /** Sidebar border line. */
  sidebarBorder: string;
  /** Selected workspace / selected file row background (`--sidebar-accent`). */
  sidebarAccent: string;
  /** Text on selected workspace / file row. */
  sidebarAccentForeground: string;
  /** Icon color when the AI / extension is actively working (spinner). */
  iconWorking: string;
  /** Icon color in idle state. */
  iconIdle: string;
  /** Icon color when blocked / awaiting approval. */
  iconBlocked: string;
  /** Icon color for a finished-but-unacknowledged run (the breathing badge
   *  that clears on focus). Distinct from `iconIdle`, which means "nothing
   *  happened here". */
  iconDone: string;
  /** Git branch glyph, wherever a branch NAME is shown: the Source Control
   *  header, the branch switcher, a pane's branch line in Workspaces. Its own
   *  token rather than a reuse of the icon triad, which means AI/CLI activity -
   *  a branch is not a status. The status bar deliberately does NOT read it;
   *  that row is monochrome by design. */
  iconBranch: string;
  /** Semantic green for diff additions, "+N" stats, success indicators. */
  diffAdded: string;
  /** Semantic red for diff removals, "-N" stats. Distinct from `destructive`
   *  which targets actionable danger UI (delete buttons, error text). */
  diffRemoved: string;
  /** Semantic sky/cyan for "info" pills, renamed/copied SCM rows, neutral
   *  status notifications. */
  info: string;
  /**
   * Focus / accent stripe color painted on the active tab in the top
   * tab bar, per tab kind. The stripe is the 3px vertical bar near the
   * left edge of the active tab; it also signals which kind of content
   * lives inside (terminal vs editor vs preview).
   */
  tabAccentTerminal: string;
  tabAccentSsh: string;
  tabAccentEditor: string;
  tabAccentPreview: string;
  tabAccentAiDiff: string;
  tabAccentGitDiff: string;
  /** Color of the drag-to-resize divider between split panes. */
  resizeHandle: string;
  /**
   * Full ANSI 16-colour palette painted by the terminal. The first eight
   * are the "standard" colours; the latter eight are the "bright" set.
   * xterm.js consumes these as `theme.black`, `theme.red`, ...,
   * `theme.brightWhite`. Themable so each preset can ship a matched
   * terminal palette instead of inheriting a generic one.
   */
  ansiBlack: string;
  ansiRed: string;
  ansiGreen: string;
  ansiYellow: string;
  ansiBlue: string;
  ansiMagenta: string;
  ansiCyan: string;
  ansiWhite: string;
  ansiBrightBlack: string;
  ansiBrightRed: string;
  ansiBrightGreen: string;
  ansiBrightYellow: string;
  ansiBrightBlue: string;
  ansiBrightMagenta: string;
  ansiBrightCyan: string;
  ansiBrightWhite: string;
};

type ThemeBackground = {
  /** When false, the background image is not painted. */
  enabled: boolean;
  /**
   * Original path of the image on disk, kept for display in the UI
   * ("Source: /Users/.../wall.png"). Loading uses `dataUrl`.
   */
  path: string;
  /**
   * Image source as a CSS-valid `data:image/...;base64,...` URL, produced by
   * `fs_read_file` from the picked local file. Empty string when no wallpaper
   * is set.
   */
  dataUrl: string;
  /** Gaussian blur on the wallpaper (0..40 px). */
  blur: number;
  /**
   * Dark overlay strength painted on top of the wallpaper (0..1). 0 = no
   * overlay, 1 = fully black. Higher values darken the image so light
   * text reads better over a busy picture.
   */
  darken: number;
  /**
   * Opacity of the wallpaper IMAGE layer itself (0..1). 1 = the image fully
   * hides the desktop behind the (transparent) window; lower lets the desktop
   * show THROUGH the image. Distinct from the app-wide `appOpacity`, which
   * fades the UI surfaces toward this layer. Default 1.
   */
  opacity: number;
};

export type CustomTheme = {
  /** User-visible name (preset id, or "Custom"). */
  name: string;
  /** Color set applied when the resolved theme is light. */
  light: ThemeColors;
  /** Color set applied when the resolved theme is dark. */
  dark: ThemeColors;
  background: ThemeBackground;
};

const THEME_FILE_VERSION = 1;

type ThemeFileV1 = {
  $schema?: "subclave-theme";
  version: typeof THEME_FILE_VERSION;
} & CustomTheme;

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

const FAST_PATH_KEY = "subclave-custom-theme-shadow";

function readShadow(): CustomTheme | null {
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

function writeShadow(theme: CustomTheme | null): void {
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
 * Transient cross-window channel for live wallpaper blur / darken / opacity
 * dragging.
 *
 * The Theme settings UI runs in its OWN webview, which has no wallpaper layer
 * (`applyBackground` removes it there). So a settings-side slider can't paint
 * the real wallpaper directly - it broadcasts only the in-flight numeric values
 * and the main window re-applies them against the wallpaper it already holds,
 * mirroring the opacity slider's `previewAppOpacity` channel. Deliberately
 * carries NO `dataUrl`: the image blob can be multiple MB, and serialising it
 * over IPC on every drag tick (~60/s) would be very heavy - the main window
 * already has it cached. No store write happens while dragging; the committed
 * value persists on release via the normal `customTheme` path.
 */
const WALLPAPER_PREVIEW_EVENT = "subclave://wallpaper-preview";

export type WallpaperPreview = {
  blur: number;
  darken: number;
  opacity: number;
};

/** Settings window: broadcast in-flight blur/darken/opacity for the main window. */
export function previewWallpaper(p: WallpaperPreview): void {
  void emit(WALLPAPER_PREVIEW_EVENT, p);
}

/** Main window: subscribe to live wallpaper blur/darken/opacity previews. */
export function onWallpaperPreview(cb: (p: WallpaperPreview) => void): Promise<UnlistenFn> {
  return listen<WallpaperPreview>(WALLPAPER_PREVIEW_EVENT, (e) => cb(e.payload));
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
 * Synchronous fast-path. Call before React mounts so the first paint uses
 * the persisted custom theme instead of the base palette. Lazy-import the
 * default palette to keep this module independent of `themePresets/`
 * (the latter imports types from here, so a static import would cycle).
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
 * Minimal fallback palettes baked into this module so the fast-path stays
 * synchronous and doesn't pull in `themePresets/` (which has the rich
 * presets and depends on this file's types). Any field missing from a
 * legacy shadow falls back to these.
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

/**
 * Merge a partial / possibly-stale `CustomTheme` payload (e.g. one
 * persisted by an older build that did not yet have all the token keys)
 * with the supplied default. Guarantees every required field is present
 * so consumers like `ThemeSection` and `applyCustomTheme` never see an
 * `undefined` token. Idempotent and side-effect-free.
 */
export function normalizeCustomTheme(loaded: unknown, defaults: CustomTheme): CustomTheme {
  if (!loaded || typeof loaded !== "object") return defaults;
  const obj = loaded as Record<string, unknown>;
  const bg =
    obj.background && typeof obj.background === "object"
      ? (obj.background as Partial<ThemeBackground>)
      : {};
  const name = typeof obj.name === "string" && obj.name.length > 0 ? obj.name : defaults.name;

  // Legacy single-mode payloads carried a flat `colors` field plus a `mode`
  // hint. Fan them out to both light + dark slots so the runtime always has
  // a variant ready when the resolved theme flips.
  const legacyColors =
    obj.colors && typeof obj.colors === "object" ? (obj.colors as Partial<ThemeColors>) : null;
  const legacyMode = obj.mode === "light" || obj.mode === "dark" ? obj.mode : null;

  const rawLight =
    obj.light && typeof obj.light === "object" ? (obj.light as Partial<ThemeColors>) : null;
  const rawDark =
    obj.dark && typeof obj.dark === "object" ? (obj.dark as Partial<ThemeColors>) : null;

  const lightSource = rawLight ?? (legacyColors && legacyMode === "light" ? legacyColors : null);
  const darkSource = rawDark ?? (legacyColors && legacyMode === "dark" ? legacyColors : null);
  // If only one was supplied (or only legacy), share it across both.
  const lightFinal = lightSource ?? darkSource ?? null;
  const darkFinal = darkSource ?? lightSource ?? null;

  return {
    name,
    light: ensureVisibleButtonFace({ ...defaults.light, ...filterStrings(lightFinal ?? {}) }),
    dark: ensureVisibleButtonFace({ ...defaults.dark, ...filterStrings(darkFinal ?? {}) }),
    background: {
      enabled: typeof bg.enabled === "boolean" ? bg.enabled : defaults.background.enabled,
      path: typeof bg.path === "string" ? bg.path : defaults.background.path,
      // Only an inline `data:` image is a supported wallpaper: the CSP allows no
      // remote URL, and an imported `.subclave` can name one, which would leave
      // an enabled-but-empty wallpaper.
      dataUrl: typeof bg.dataUrl === "string" && bg.dataUrl.startsWith("data:") ? bg.dataUrl : "",
      blur: clampRange(bg.blur, 0, 40, defaults.background.blur),
      darken: clamp01(bg.darken, defaults.background.darken),
      opacity: clamp01(bg.opacity, defaults.background.opacity),
    },
  };
}

function clamp01(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(0, Math.min(1, value))
    : fallback;
}

function clampRange(value: unknown, min: number, max: number, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(min, Math.min(max, value))
    : fallback;
}

/**
 * Validate an imported `.subclave` payload. Throws with a user-readable message
 * when the structure is bad. Lenient with missing/extra keys: fills in
 * defaults from the supplied `fallback` for any field that's absent.
 */
export function parseThemeFile(raw: unknown, fallback: CustomTheme): CustomTheme {
  if (!raw || typeof raw !== "object") throw new Error("Theme file is not a JSON object");
  // Delegate variant + legacy-`colors` + bg field plumbing to the shared
  // normalizer so .subclave parsing stays in lock-step with the runtime store
  // hydration path. Only override `name` (defaults to "Custom" when blank).
  const merged = normalizeCustomTheme(raw, fallback);
  const obj = raw as Record<string, unknown>;
  const name = typeof obj.name === "string" && obj.name.length > 0 ? obj.name : "Custom";
  return { ...merged, name };
}

function filterStrings(obj: Partial<ThemeColors>): Partial<ThemeColors> {
  const out: Partial<ThemeColors> = {};
  for (const [k, v] of Object.entries(obj)) {
    if (typeof v === "string" && v.length > 0) (out as Record<string, string>)[k] = v;
  }
  return out;
}

/** Serialise a theme for `.subclave` export. */
export function serializeThemeFile(theme: CustomTheme): string {
  const payload: ThemeFileV1 = {
    $schema: "subclave-theme",
    version: THEME_FILE_VERSION,
    ...theme,
  };
  return JSON.stringify(payload, null, 2);
}
