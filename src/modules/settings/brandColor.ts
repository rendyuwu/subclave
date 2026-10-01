/**
 * Brand color runtime applier. Overrides primary CSS variables on `:root`
 * so components re-tint without a reload. Derives `--accent` and
 * `--sidebar-accent` from the brand so soft-fill surfaces stay in family.
 */

import { hexToRgb, relLuminance, rgbToHex, type Rgb } from "@/lib/color";
import { readShadow as readStored, writeShadow as writeStoredShadow } from "@/lib/fastPath";
import { BRAND_COLOR_DEFAULT, normalizeBrandColor } from "./schema";

const FAST_PATH_KEY = "subclave-brand-color-shadow";

// CSS vars driven directly by the brand hex. Identical in light and dark.
const PRIMARY_VARS = ["--primary", "--ring", "--sidebar-primary", "--sidebar-ring"] as const;

// Linear interpolation between two colors. `t` is the brand share (0 to 1).
function mix(a: Rgb, b: Rgb, t: number): Rgb {
  return {
    r: a.r * (1 - t) + b.r * t,
    g: a.g * (1 - t) + b.g * t,
    b: a.b * (1 - t) + b.b * t,
  };
}

/**
 * Apply a brand color to the document. Reads the current theme from the
 * `.dark` class on `<html>` so accent derivation matches.
 */
export function applyBrandColor(hex: string): void {
  if (typeof document === "undefined") return;
  const color = normalizeBrandColor(hex);
  const root = document.documentElement;
  const isDark = root.classList.contains("dark");

  for (const v of PRIMARY_VARS) root.style.setProperty(v, color);

  const brand = hexToRgb(color);
  // Light: 85% white + 15% brand for a soft surface (matches #dbe5ff at default).
  // Dark: 56% black + 44% brand for a deep accent (matches #0a2870 at default).
  const accent = isDark
    ? mix({ r: 0, g: 0, b: 0 }, brand, 0.44)
    : mix({ r: 255, g: 255, b: 255 }, brand, 0.15);
  const accentHex = rgbToHex(accent);
  root.style.setProperty("--accent", accentHex);
  root.style.setProperty("--sidebar-accent", accentHex);

  // Contrasting foreground for `--primary`. Black on light brands (pastel yellows etc.) to keep WCAG.
  const fg = relLuminance(color) > 0.6 ? "#000000" : "#ffffff";
  root.style.setProperty("--primary-foreground", fg);
  root.style.setProperty("--sidebar-primary-foreground", fg);

  writeStoredShadow(FAST_PATH_KEY, color);
}

/**
 * Synchronous fast-path. Call before React mounts so the first paint uses
 * the persisted brand color instead of the default blue. The store
 * hydration overrides this shortly after.
 */
export function applyBrandColorFastPath(): void {
  const stored = readStored(FAST_PATH_KEY);
  applyBrandColor(stored ? normalizeBrandColor(stored) : BRAND_COLOR_DEFAULT);
}
