/**
 * Self-check for `src/modules/groups/treeKeyboard.ts`.
 * Run: `npx tsx scripts/tree-keyboard-verify.ts`.
 *
 * The group tree's keyboard handling delegates entirely to `treeKeyAction`;
 * a silent wrong answer here (e.g. ArrowLeft jumping to the wrong parent)
 * ships straight into the UI with nothing else catching it.
 */
import { treeKeyAction, type TreeKeyItem } from "../src/modules/groups/treeKeyboard";

let failed = 0;
function check(label: string, got: unknown, want: unknown): void {
  const ok = JSON.stringify(got) === JSON.stringify(want);
  if (ok) {
    console.log(`  ok: ${label}`);
    return;
  }
  console.error(
    `  FAIL: ${label}\n    got:  ${JSON.stringify(got)}\n    want: ${JSON.stringify(want)}`,
  );
  failed++;
}

// A(root, children, expanded), A1(parent A, leaf), A2(parent A, children, collapsed), B(root, leaf)
const items: TreeKeyItem[] = [
  { id: "A", parentId: null, hasChildren: true, expanded: true },
  { id: "A1", parentId: "A", hasChildren: false, expanded: false },
  { id: "A2", parentId: "A", hasChildren: true, expanded: false },
  { id: "B", parentId: null, hasChildren: false, expanded: false },
];

check("Down with -1", treeKeyAction("ArrowDown", items, -1), { kind: "select", index: 0 });
check("Down at 3", treeKeyAction("ArrowDown", items, 3), { kind: "select", index: 3 });
check("Up with -1", treeKeyAction("ArrowUp", items, -1), { kind: "select", index: 3 });
check("Up at 0", treeKeyAction("ArrowUp", items, 0), { kind: "select", index: 0 });
check("Right at 0", treeKeyAction("ArrowRight", items, 0), { kind: "select", index: 1 });
check("Right at 2", treeKeyAction("ArrowRight", items, 2), { kind: "toggle", index: 2 });
check("Right at 1", treeKeyAction("ArrowRight", items, 1), null);
check("Left at 0", treeKeyAction("ArrowLeft", items, 0), { kind: "toggle", index: 0 });
check("Left at 1", treeKeyAction("ArrowLeft", items, 1), { kind: "select", index: 0 });
check("Left at 2", treeKeyAction("ArrowLeft", items, 2), { kind: "select", index: 0 });
check("Left at 3", treeKeyAction("ArrowLeft", items, 3), null);
check("Enter at 2", treeKeyAction("Enter", items, 2), { kind: "toggle", index: 2 });
check("Enter at 1", treeKeyAction("Enter", items, 1), { kind: "activate", index: 1 });
check("Enter at -1", treeKeyAction("Enter", items, -1), null);
check('key "a"', treeKeyAction("a", items, 0), null);
check("Down on []", treeKeyAction("ArrowDown", [], -1), null);

if (failed > 0) process.exit(1);
console.log("tree-keyboard-verify: all checks passed");
