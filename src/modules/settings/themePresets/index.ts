/**
 * Built-in theme presets, one file per family.
 *
 * A preset covers BOTH domains it can reach: the app chrome tokens and the
 * ANSI 16 entry colours, so names must stay unique.
 *
 * Adding one: copy the closest family file, adjust, export it here. Every field
 * is required - `scripts/theme-verify.ts` fails on a preset that misses one.
 */
import type { CustomTheme } from "../customTheme";
import { DARK_COLORS, EMPTY_BG, LIGHT_COLORS } from "./base";
import { TOKYO_NIGHT } from "./tokyoNight";
import { NORD } from "./nord";
import { CATPPUCCIN } from "./catppuccin";
import { SOLARIZED } from "./solarized";
import { MONOKAI } from "./monokai";
import { MATRIX } from "./matrix";
import { KANAGAWA } from "./kanagawa";
import { NEBULA } from "./nebula";

export const DEFAULT_CUSTOM_THEME: CustomTheme = {
  name: "Default",
  light: LIGHT_COLORS,
  dark: DARK_COLORS,
  background: { ...EMPTY_BG },
};

export const THEME_PRESETS: CustomTheme[] = [
  {
    name: "Default",
    light: LIGHT_COLORS,
    dark: DARK_COLORS,
    background: { ...EMPTY_BG },
  },
  TOKYO_NIGHT,
  NORD,
  CATPPUCCIN,
  SOLARIZED,
  MONOKAI,
  MATRIX,
  KANAGAWA,
  NEBULA,
];
