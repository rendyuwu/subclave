import { IS_MAC, MOD_PROP } from "@/lib/platform";

/** Keyboard shortcut catalog. */

export type ShortcutId =
  | "settings.open"
  | "commandPalette.open"
  | "search.focus"
  | "vault.lock"
  | "entry.new"
  | "entry.edit"
  | "entry.trash"
  | "entry.copyPassword"
  | "entry.copyUsername"
  | "entry.copyTotp"
  | "entry.openUrl";

export type ShortcutGroup = "General" | "Entries" | "Vault" | "Command Palette";

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
  /**
   * Extra guard evaluated after a binding matches and before the modal gate.
   * Scopes a chord to its surface: the list-scoped entry chords must not fire
   * from a text field or a tree row, where the same key means something else.
   * A returned false skips this chord and lets the loop try the next one.
   */
  when?: (e: KeyboardEvent) => boolean;
};

/**
 * True when the event target is a text-entry surface (input, textarea, select
 * or a contenteditable host). Duck-typed, so the node-loaded verify scripts can
 * pass a plain object rather than a real Element.
 */
export function isTextEntryTarget(target: EventTarget | null): boolean {
  const el = target as { tagName?: unknown; isContentEditable?: unknown } | null;
  if (!el) return false;
  const tag = typeof el.tagName === "string" ? el.tagName.toUpperCase() : "";
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable === true;
}

/**
 * True when the event target sits inside the entry list (`[data-vault-list]`).
 * Duck-typed: only the `closest` method is assumed, so a node test can pass a
 * stub without a DOM.
 */
export function isEntryListTarget(target: EventTarget | null): boolean {
  const el = target as { closest?: (selector: string) => Element | null } | null;
  if (!el || typeof el.closest !== "function") return false;
  return el.closest("[data-vault-list]") !== null;
}

export const SHORTCUTS: Shortcut[] = [
  {
    id: "settings.open",
    label: "Open settings",
    group: "General",
    defaultBindings: [{ [MOD_PROP]: true, key: "," }],
  },
  {
    id: "search.focus",
    label: "Focus search",
    group: "General",
    defaultBindings: [{ [MOD_PROP]: true, key: "f" }],
  },
  {
    id: "vault.lock",
    label: "Lock the vault",
    group: "Vault",
    defaultBindings: [{ [MOD_PROP]: true, key: "l" }],
  },
  {
    id: "entry.new",
    label: "New entry",
    group: "Entries",
    defaultBindings: [{ [MOD_PROP]: true, key: "n" }],
  },
  {
    id: "entry.edit",
    label: "Edit entry",
    group: "Entries",
    defaultBindings: [{ key: "Enter" }, { [MOD_PROP]: true, key: "e" }],
    when: (e) => isEntryListTarget(e.target),
  },
  {
    id: "entry.trash",
    label: "Move to trash",
    group: "Entries",
    defaultBindings: [{ key: "Delete" }],
    when: (e) => isEntryListTarget(e.target),
  },
  {
    id: "entry.copyPassword",
    label: "Copy password",
    group: "Entries",
    defaultBindings: [{ [MOD_PROP]: true, key: "c" }],
    when: (e) => isEntryListTarget(e.target),
  },
  {
    id: "entry.copyUsername",
    label: "Copy username",
    group: "Entries",
    defaultBindings: [{ [MOD_PROP]: true, key: "b" }],
    when: (e) => !isTextEntryTarget(e.target),
  },
  {
    id: "entry.copyTotp",
    label: "Copy TOTP",
    group: "Entries",
    defaultBindings: [{ [MOD_PROP]: true, key: "t" }],
    when: (e) => !isTextEntryTarget(e.target),
  },
  {
    id: "entry.openUrl",
    label: "Open URL",
    group: "Entries",
    defaultBindings: [{ [MOD_PROP]: true, key: "u" }],
    when: (e) => !isTextEntryTarget(e.target),
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

export const SHORTCUT_GROUPS: ShortcutGroup[] = ["General", "Entries", "Vault", "Command Palette"];

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
 *
 * Exported for the recorder, which stores this key so a binding recorded
 * with Option held or on a non-Latin layout still matches on replay.
 */
export function canonicalKey(e: KeyboardEvent): string {
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
