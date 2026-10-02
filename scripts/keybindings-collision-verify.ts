/**
 * Shortcut-collision audit: no two DIFFERENT catalog actions may share the same
 * chord (intra-app clash).
 * Run: `npx tsx scripts/keybindings-collision-verify.ts`.
 *
 * Two actions on one chord is silent: `useGlobalShortcuts` takes the first match
 * in array order, so the second action stops firing and nothing anywhere says so.
 *
 * Under node, platform() throws so MOD_PROP resolves to "ctrl" (see platform.ts),
 * i.e. this checks exactly the Windows/Linux expansion (Mod = Ctrl). macOS is
 * safer by construction: Mod = Cmd (meta), a different chord.
 */
import { SHORTCUTS, type KeyBinding } from "../src/modules/shortcuts/shortcuts";

function canon(b: KeyBinding): string {
  const mods = [b.ctrl && "Ctrl", b.shift && "Shift", b.alt && "Alt", b.meta && "Meta"]
    .filter(Boolean)
    .join("+");
  return (mods ? mods + "+" : "") + b.key.toLowerCase();
}

let failed = 0;

// --- A. Intra-app duplicate chords ---------------------------------------
console.log("[A] intra-app duplicate chords (same key -> two actions; first in array wins)");
const byChord = new Map<string, string[]>();
for (const s of SHORTCUTS) {
  const bindings = s.defaultBindings;
  for (const b of bindings) {
    const c = canon(b);
    const arr = byChord.get(c) ?? [];
    arr.push(s.id);
    byChord.set(c, arr);
  }
}
let dupes = 0;
for (const [chord, ids] of byChord) {
  const distinct = [...new Set(ids)];
  if (distinct.length > 1) {
    console.error(`  CLASH: ${chord} -> ${distinct.join(", ")}`);
    dupes++;
    failed++;
  }
}
if (dupes === 0) console.log("  ok: no chord is bound to two different actions");

if (failed > 0) throw new Error(`${failed} collision issue(s) found`);
console.log("\nAll checks passed: no chord is bound to two different actions.");
