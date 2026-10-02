import type { ThemeColors } from "../theme/model";

export const EMPTY_BG = {
  enabled: false,
  path: "",
  dataUrl: "",
  blur: 0,
  darken: 0,
  opacity: 1,
} as const;

/**
 * Generic ANSI accent palette tuned for dark surfaces. Each preset spreads
 * this so it picks up the full set, then can override individual slots
 * where the canonical preset specifies different ANSI colors.
 */
const ANSI_DARK = {
  ansiRed: "#ef4444",
  ansiGreen: "#22c55e",
  ansiYellow: "#eab308",
  ansiBlue: "#3b82f6",
  ansiMagenta: "#a855f7",
  ansiCyan: "#06b6d4",
} satisfies Pick<
  ThemeColors,
  "ansiRed" | "ansiGreen" | "ansiYellow" | "ansiBlue" | "ansiMagenta" | "ansiCyan"
>;

/** Generic ANSI accent palette tuned for light surfaces (less neon, more contrast). */
const ANSI_LIGHT = {
  ansiRed: "#dc2626",
  ansiGreen: "#16a34a",
  ansiYellow: "#ca8a04",
  ansiBlue: "#2563eb",
  ansiMagenta: "#9333ea",
  ansiCyan: "#0891b2",
} satisfies typeof ANSI_DARK;

export const DARK_COLORS: ThemeColors = {
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
  diffAdded: "#4ade80",
  info: "#38bdf8",
  resizeHandle: "#2b2b2b",
  ...ANSI_DARK,
};

export const LIGHT_COLORS: ThemeColors = {
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
  mutedForeground: "#5d646e",
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
  diffAdded: "#16a34a",
  info: "#0284c7",
  resizeHandle: "#e5e7eb",
  ...ANSI_LIGHT,
};
