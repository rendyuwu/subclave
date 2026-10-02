/**
 * Self-check for the group forest, the list derivations, the copy toast text
 * and the two shortcut target predicates.
 * Run: `npx tsx scripts/group-tree-verify.ts`.
 *
 * All four are pure and all four are load-bearing in a way the UI cannot show:
 * a cycle in a synced group list must resolve to the root instead of vanishing
 * or hanging the walk; the All / Favourites / Trash scopes must not leak
 * trashed entries into the counts; the copy toast's countdown is the only place
 * the user is told the clipboard will clear itself; and the two predicates are
 * what keeps `Delete`, `Enter` and `Mod+C` from trashing, editing or swallowing
 * a copy while the caret is in a text field.
 *
 * This suite's filename is also the reason a bare file spelling is fine below:
 * everything it names is under the directories it is read from.
 */
import {
  buildGroupTree,
  descendantIds,
  effectiveParentId,
  type GroupNode,
} from "../src/modules/groups/groupTreeModel";
import {
  ALL_SCOPE,
  FAVORITES_SCOPE,
  TRASH_SCOPE,
  groupCounts,
  isExpired,
  tagCounts,
  visibleEntries,
} from "../src/modules/vault/list/derive";
import { copyToastText } from "../src/modules/vault/copy";
import { isEntryListTarget, isTextEntryTarget } from "../src/modules/shortcuts/shortcuts";
import type { EntrySummary, Group } from "../src/modules/vault/types";

let failed = 0;
function check(label: string, ok: boolean, detail?: unknown): void {
  if (ok) {
    console.log(`  ok: ${label}`);
    return;
  }
  console.error(`  FAIL: ${label}`, detail === undefined ? "" : JSON.stringify(detail));
  failed++;
}

function group(id: string, parentId: string | null, name: string): Group {
  return { id, parentId, name, icon: null, color: null, createdAt: 0, updatedAt: 0 };
}

function entry(id: string, over: Partial<EntrySummary> = {}): EntrySummary {
  return {
    id,
    groupId: "root",
    title: id,
    username: "",
    primaryHost: null,
    tags: [],
    icon: null,
    color: null,
    favorite: false,
    hasPassword: true,
    hasTotp: false,
    expiresAt: null,
    updatedAt: 0,
    lastUsedAt: null,
    ...over,
  };
}

const ids = (nodes: GroupNode[]): string[] => nodes.map((n) => n.group.id);

console.log("[forest] nesting, sibling order, and the reserved ids");
{
  const groups = [
    group("root", null, "Root"),
    group("trash", null, "Trash"),
    group("b", "root", "Beta"),
    group("a", "root", "Alpha"),
    group("a2", "a", "Alpha two"),
  ];
  const tree = buildGroupTree(groups);
  check("the root itself is never a node", !ids(tree).includes("root"), ids(tree));
  check("nor is Trash", !ids(tree).includes("trash"), ids(tree));
  check("siblings sort by name", ids(tree).join(",") === "a,b", ids(tree));
  check("children nest under their parent", ids(tree[0].children).join(",") === "a2");
  check(
    "descendantIds spans the subtree",
    [...descendantIds("a", groups)].sort().join(",") === "a,a2",
  );
  check(
    "and is empty for a group that is not in the forest",
    descendantIds("root", groups).size === 0,
  );
}

console.log("[forest] a synced parentId the device never agreed on");
{
  // c1 <-> c2 is a cycle: two devices reparenting the same pair in opposite
  // directions. Both members must show up at the root rather than vanish.
  const cyclic = [
    group("root", null, "Root"),
    group("c1", "c2", "Cyclic one"),
    group("c2", "c1", "Cyclic two"),
  ];
  const tree = buildGroupTree(cyclic);
  check("both cycle members are reachable", ids(tree).join(",") === "c1,c2", ids(tree));
  const byId = new Map(cyclic.map((g) => [g.id, g]));
  check("a group on a cycle resolves to the root", effectiveParentId("c1", byId) === "root");
  // A group hanging off a cycle member keeps that member: only the cycle is
  // repaired, not every parent on the way up.
  const hanging = [...cyclic, group("d", "c1", "Dependent")];
  const hangingTree = buildGroupTree(hanging);
  const c1 = hangingTree.find((n) => n.group.id === "c1");
  check("a child of a cycle member keeps its parent", ids(c1?.children ?? []).join(",") === "d");
}

console.log("[forest] a dangling parent");
{
  const dangling = [group("root", null, "Root"), group("x", "gone", "Orphan")];
  check(
    "resolves to the root so the group stays visible",
    ids(buildGroupTree(dangling)).join(",") === "x",
  );
  check(
    "and effectiveParentId says so directly",
    effectiveParentId("x", new Map(dangling.map((g) => [g.id, g]))) === "root",
  );
}

console.log("[list] scope rules");
{
  const rows = [
    entry("live", { groupId: "a", favorite: true }),
    entry("plain", { groupId: "a" }),
    entry("other", { groupId: "b" }),
    entry("trashed", { groupId: TRASH_SCOPE, favorite: true }),
  ];
  const show = (
    scope: string,
    tagFilter: string[] = [],
    searchIds: readonly string[] | null = null,
  ) => visibleEntries({ entries: rows, scope, tagFilter, searchIds, now: 0 }).map((e) => e.id);

  check("All is every entry outside Trash", show(ALL_SCOPE).join(",") === "live,other,plain");
  check("Favourites is the favourites outside Trash", show(FAVORITES_SCOPE).join(",") === "live");
  check("the Trash scope is exactly that group", show(TRASH_SCOPE).join(",") === "trashed");
  check("a group id is that group", show("b").join(",") === "other");
  check("an unknown scope matches nothing", show("nope").length === 0);
}

console.log("[list] query intersection, tag AND, and title ordering");
{
  const rows = [
    entry("Item 10", { tags: ["Web", "work"] }),
    entry("Item 2", { tags: ["web"] }),
    entry("Alpha", { tags: ["web", "work"] }),
  ];
  const titles = (tagFilter: string[], searchIds: readonly string[] | null) =>
    visibleEntries({ entries: rows, scope: ALL_SCOPE, tagFilter, searchIds, now: 0 }).map(
      (e) => e.title,
    );

  check(
    "titles sort with numeric collation",
    titles([], null).join(" | ") === "Alpha | Item 2 | Item 10",
  );
  check("tag matching is case-insensitive", titles(["web"], null).length === 3);
  check(
    "every selected tag must be present (AND)",
    titles(["web", "work"], null).join(" | ") === "Alpha | Item 10",
  );
  check("a query narrows to its id set", titles([], ["Item 2"]).join(" | ") === "Item 2");
  check("a query with no hits filters everything out", titles([], []).length === 0);
}

console.log("[list] counts are per group and direct-entry only");
{
  const groups = [group("root", null, "Root"), group("a", "root", "A"), group("b", "root", "B")];
  const rows = [
    entry("1", { groupId: "a", favorite: true }),
    entry("2", { groupId: "a" }),
    entry("3", { groupId: "b" }),
    entry("4", { groupId: TRASH_SCOPE, favorite: true }),
  ];
  const counts = groupCounts(rows, groups);
  check("All excludes Trash", counts.get(ALL_SCOPE) === 3, counts.get(ALL_SCOPE));
  check("Favourites counts live favourites only", counts.get(FAVORITES_SCOPE) === 1);
  check("Trash counts itself", counts.get(TRASH_SCOPE) === 1);
  check("a group counts its own entries", counts.get("a") === 2 && counts.get("b") === 1);
  check("an empty group is seeded with zero, not absent", counts.get("root") === 0);
}

console.log("[list] tag counts and expiry");
{
  const rows = [
    entry("1", { tags: ["Web", "work"] }),
    entry("2", { tags: ["web"] }),
    entry("3", { groupId: TRASH_SCOPE, tags: ["gone"] }),
  ];
  const tags = tagCounts(rows);
  check("one row per canonical spelling", tags.map((t) => t.tag).join(",") === "Web,work", tags);
  check("counts are case-insensitive", tags[0].count === 2);
  check("trashed entries are left out", !tags.some((t) => t.tag === "gone"));
  check(
    "expiry is a boundary, not a window",
    isExpired(entry("e", { expiresAt: 100 }), 100) &&
      !isExpired(entry("e", { expiresAt: 101 }), 100) &&
      !isExpired(entry("e"), 100),
  );
}

console.log("[copy] the toast says what the countdown is doing");
{
  const now = 1_000_000;
  check(
    "password",
    copyToastText("password", now + 30_000, now) === "Password copied. Clears in 30 seconds.",
  );
  check(
    "username",
    copyToastText("username", now + 30_000, now) === "Username copied. Clears in 30 seconds.",
  );
  check(
    "totp reads as a code",
    copyToastText("totp", now + 30_000, now) === "Code copied. Clears in 30 seconds.",
  );
  check(
    "a custom field uses its own name",
    copyToastText("API key", now + 5_000, now) === "API key copied. Clears in 5 seconds.",
  );
  check(
    "a partial second rounds up",
    copyToastText("password", now + 1, now) === "Password copied. Clears in 1 second.",
  );
  check("one second is singular", copyToastText("password", now + 1, now).endsWith("1 second."));
  check(
    "never clears omits the countdown",
    copyToastText("password", null, now) === "Password copied.",
  );
}

console.log("[shortcuts] the two target predicates");
{
  check(
    "an input is a text target",
    isTextEntryTarget({ tagName: "input" } as unknown as EventTarget),
  );
  check("a textarea is", isTextEntryTarget({ tagName: "TEXTAREA" } as unknown as EventTarget));
  check("a select is", isTextEntryTarget({ tagName: "select" } as unknown as EventTarget));
  check(
    "a contenteditable is",
    isTextEntryTarget({ isContentEditable: true } as unknown as EventTarget),
  );
  check("a button is not", !isTextEntryTarget({ tagName: "BUTTON" } as unknown as EventTarget));
  check("nor is nothing at all", !isTextEntryTarget(null));

  const inList = {
    closest: (sel: string) => (sel === "[data-vault-list]" ? ({} as Element) : null),
  };
  const outside = { closest: () => null };
  check(
    "a target inside the entry list is a list target",
    isEntryListTarget(inList as unknown as EventTarget),
  );
  check("one outside it is not", !isEntryListTarget(outside as unknown as EventTarget));
  check(
    "an object with no closest method is refused rather than throwing",
    !isEntryListTarget({} as unknown as EventTarget) && !isEntryListTarget(null),
  );
}

if (failed > 0) throw new Error(`group-tree-verify: ${failed} check(s) failed`);
console.log("\ngroup-tree-verify: all checks passed");
