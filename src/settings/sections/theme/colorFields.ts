import { type ThemeColors } from "@/modules/settings/theme/model";

/**
 * Every editable APP-CHROME theme color, grouped for the tabbed color editor in
 * ThemeSection. The ANSI accents are NOT hand-edited here — they ship with each
 * preset (and carry the entry colours other surfaces read). Order within a
 * group is the display order.
 */
export const COLOR_FIELDS: { key: keyof ThemeColors; label: string; group: string }[] = [
  { key: "background", label: "Background", group: "Base" },
  { key: "foreground", label: "Foreground", group: "Base" },
  { key: "card", label: "Card", group: "Base" },
  { key: "cardForeground", label: "Card text", group: "Base" },
  { key: "popover", label: "Popover", group: "Base" },
  { key: "popoverForeground", label: "Popover text", group: "Base" },
  { key: "button", label: "Primary button", group: "Buttons" },
  { key: "buttonForeground", label: "Primary button text", group: "Buttons" },
  { key: "buttonFace", label: "Neutral button", group: "Buttons" },
  { key: "buttonFaceForeground", label: "Neutral button text", group: "Buttons" },
  { key: "secondary", label: "Secondary", group: "Buttons" },
  { key: "secondaryForeground", label: "Secondary text", group: "Buttons" },
  { key: "border", label: "Border", group: "Borders" },
  { key: "input", label: "Input border", group: "Borders" },
  { key: "ring", label: "Focus / tab bar", group: "Borders" },
  { key: "resizeHandle", label: "Split-pane divider", group: "Borders" },
  { key: "accent", label: "Accent", group: "Highlights" },
  { key: "accentForeground", label: "Accent text", group: "Highlights" },
  { key: "muted", label: "Muted", group: "Highlights" },
  { key: "mutedForeground", label: "Muted text", group: "Highlights" },
  { key: "destructive", label: "Destructive", group: "Highlights" },
  { key: "sidebar", label: "Sidebar", group: "Sidebar" },
  { key: "sidebarForeground", label: "Sidebar text", group: "Sidebar" },
  { key: "sidebarBorder", label: "Sidebar border", group: "Sidebar" },
  { key: "sidebarAccent", label: "Selected workspace / file", group: "Sidebar" },
  { key: "sidebarAccentForeground", label: "Selected workspace / file text", group: "Sidebar" },
  { key: "iconWorking", label: "Icon working", group: "Icons" },
  { key: "diffAdded", label: "Diff added (+)", group: "Highlights" },
  { key: "info", label: "Info", group: "Highlights" },
];

export const GROUPS = ["Base", "Buttons", "Borders", "Highlights", "Sidebar", "Icons"] as const;
export type Group = (typeof GROUPS)[number];
