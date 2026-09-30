import { IS_MAC, MOD_PROP } from "@/lib/platform";

/** Keyboard shortcut catalog. */

export type ShortcutId = "settings.open" | "commandPalette.open";

export type ShortcutGroup = "General" | "Command Palette";

export type KeyBinding = {
  key: string;
  ctrl?: boolean;
  shift?: boolean;
  alt?: boolean;
  meta?: boolean;
};

export type Shortcut = {
  id: ShortcutId;
  label: string;
  group: ShortcutGroup;
  defaultBindings: KeyBinding[];
  /** List in settings but disable recorder + reset. For component-hardcoded
   *  keys (e.g. textarea Enter) shown for documentation. */
  readOnly?: boolean;
};

export const SHORTCUTS: Shortcut[] = [
  {
    id: "settings.open",
    label: "Open settings",
    group: "General",
    defaultBindings: [{ [MOD_PROP]: true, key: "," }],
  },
  {
    // Opens the Command Palette — a searchable list of all commands. VS Code
    // parity: Cmd+Shift+P on macOS, Ctrl+Shift+P on Win/Linux.
    id: "commandPalette.open",
    label: "Command Palette",
    group: "Command Palette",
    defaultBindings: [{ [MOD_PROP]: true, shift: true, key: "p" }],
  },
];

export const SHORTCUT_GROUPS: ShortcutGroup[] = ["General", "Command Palette"];

/**
 * Layout-independent key canonicalization. Uses `e.code` for letters/digits
 * because `e.key` varies with layout and modifiers:
 *   - macOS Option produces composed glyphs (`Option+Z` -> "Omega"), so a
 *     binding `{ alt: true, key: "z" }` would never match.
 *   - Non-Latin layouts (Cyrillic, Greek, Arabic) emit non-Latin `key`
 *     values, breaking Latin-letter defaults.
 * `e.code` is stable across layouts (`KeyT`, `Digit5`, `BracketLeft`).
 * For everything else (punctuation, function/navigation/named keys) fall
 * back to `e.key`. Same hybrid VS Code and CodeMirror use.
 */
function canonicalKey(e: KeyboardEvent): string {
  const code = e.code;
  // KeyA..KeyZ -> "a".."z"
  if (code.length === 4 && code.startsWith("Key")) {
    return code.slice(3).toLowerCase();
  }
  // Digit0..Digit9 -> "0".."9". Skip Numpad0..9 so a top-row digit binding
  // doesn't fire from numpad input.
  if (code.length === 6 && code.startsWith("Digit")) {
    return code.slice(5);
  }
  return e.key.toLowerCase();
}

/** Returns true if the KeyboardEvent matches the KeyBinding. */
export function matchBinding(e: KeyboardEvent, binding: KeyBinding): boolean {
  const eventKey = canonicalKey(e);
  const bindingKey = binding.key.toLowerCase();

  if (eventKey !== bindingKey) return false;

  return (
    !!e.ctrlKey === !!binding.ctrl &&
    !!e.shiftKey === !!binding.shift &&
    !!e.altKey === !!binding.alt &&
    !!e.metaKey === !!binding.meta
  );
}

/**
 * Recorder counterpart. Returns the canonical key so bindings recorded with
 * Option held or on non-Latin layouts still match on replay.
 */
export function canonicalKeyFromEvent(e: KeyboardEvent): string {
  return canonicalKey(e);
}

/**
 * Parses an extension's `contributes.keybindings[].key` string
 * (e.g. "Mod+Shift+E", "Ctrl+K", "Alt+Shift+ArrowLeft") into a `KeyBinding`.
 * VS Code grammar:
 *   `Mod` is `meta` on macOS, `ctrl` elsewhere (matches `MOD_PROP`).
 *   Modifiers (case-insensitive): ctrl/control, shift, alt/option/opt,
 *   meta/cmd/command/win/super, mod. Separated by `+`. Trailing token is the key.
 *   Single chars are lowercased; named keys pass through.
 * Returns `null` when input is empty or has no key token. Unknown modifiers
 * are skipped silently.
 */
export function parseKeybindingString(input: string): KeyBinding | null {
  if (typeof input !== "string") return null;
  const parts = input
    .split("+")
    .map((p) => p.trim())
    .filter((p) => p.length > 0);
  if (parts.length === 0) return null;
  const binding: KeyBinding = { key: "" };
  for (let i = 0; i < parts.length; i++) {
    const token = parts[i];
    const isLast = i === parts.length - 1;
    const lower = token.toLowerCase();
    if (!isLast) {
      switch (lower) {
        case "ctrl":
        case "control":
          binding.ctrl = true;
          break;
        case "shift":
          binding.shift = true;
          break;
        case "alt":
        case "option":
        case "opt":
          binding.alt = true;
          break;
        case "meta":
        case "cmd":
        case "command":
        case "win":
        case "super":
          binding.meta = true;
          break;
        case "mod":
          // VS Code alias: Cmd on Mac, Ctrl elsewhere. Aligns with `MOD_PROP`.
          binding[MOD_PROP] = true;
          break;
        default:
          // Unknown modifier: drop it so a single typo doesn't kill the binding.
          break;
      }
      continue;
    }
    // Last token is the key. Lowercase single chars so `matchBinding`'s
    // canonical comparison matches regardless of manifest casing.
    binding.key = token.length === 1 ? token.toLowerCase() : token;
  }
  if (!binding.key) return null;
  return binding;
}

/** Display tokens for a binding (platform-specific glyphs on macOS). */
export function getBindingTokens(binding?: KeyBinding): string[] {
  if (!binding) return [];
  const tokens: string[] = [];
  if (IS_MAC) {
    if (binding.ctrl) tokens.push("⌃");
    if (binding.alt) tokens.push("⌥");
    if (binding.shift) tokens.push("⇧");
    if (binding.meta) tokens.push("⌘");
  } else {
    if (binding.ctrl) tokens.push("Ctrl");
    if (binding.alt) tokens.push("Alt");
    if (binding.shift) tokens.push("Shift");
    if (binding.meta) tokens.push("Win");
  }

  // Compare case-insensitively: defaults store "ArrowLeft" but the recorder
  // stores the canonical lowercase ("arrowleft"), so a rebind to an arrow must
  // still render as a glyph.
  let keyLabel = binding.key;
  const lowerKey = keyLabel.toLowerCase();
  if (lowerKey === " ") keyLabel = "Space";
  else if (lowerKey === "arrowup") keyLabel = "↑";
  else if (lowerKey === "arrowdown") keyLabel = "↓";
  else if (lowerKey === "arrowleft") keyLabel = "←";
  else if (lowerKey === "arrowright") keyLabel = "→";
  else if (keyLabel.length === 1) keyLabel = keyLabel.toUpperCase();

  tokens.push(keyLabel);
  return tokens;
}
