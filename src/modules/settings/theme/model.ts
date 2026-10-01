/**
 * Custom theme data model: the persisted shape and the `.subclave` file
 * format, with the pure normalise / parse / serialise helpers. No DOM or
 * Tauri here, so node-side checks can import it directly. The persisted
 * shape lives in `../schema.ts` (`Preferences.customTheme`).
 */

import { ensureVisibleButtonFace } from "../buttonFace";

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

export type ThemeBackground = {
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
