// The read-time shape of the group list as a forest, and the one place that
// walk lives: `GroupTree.tsx` renders it, and the move-target menu asks for a
// group's descendants rather than walking `parentId` on its own.
//
// A synced `parentId` can name a group this device deleted, name its own
// group, or form a cycle. `effectiveParentId` is the read-time tolerance for
// all three: the offending group resolves to the root instead of vanishing or
// hanging a walk. The write path refuses a cycle in the first place.
//
// The root and Trash groups carry `parentId: null` here rather than being
// pointed at the root, so the forest starts at the root's children and the
// Trash row is appended by the tree separately.

import type { Group } from "@/modules/vault/types";

export const ROOT_ID = "root";
export const TRASH_ID = "trash";

/** One node in the forest {@link buildGroupTree} returns. */
export type GroupNode = {
  group: Group;
  children: GroupNode[];
};

/**
 * This group's resolved parent id. A group with no parent, a parent that names
 * no group, or a group sitting on a cycle resolves to the root.
 *
 * Only the group's own parent is checked for existing, and only a group on the
 * cycle resolves to the root: a group that hangs off a cycle member keeps that
 * member as its parent, the same way it would keep any other parent.
 */
export function effectiveParentId(id: string, byId: Map<string, Group>): string {
  const raw = byId.get(id)?.parentId ?? null;
  if (raw === null || !byId.has(raw)) return ROOT_ID;
  const seen = new Set<string>();
  for (
    let cursor: string | null = raw;
    cursor !== null && !seen.has(cursor);
    cursor = byId.get(cursor)?.parentId ?? null
  ) {
    if (cursor === id) return ROOT_ID;
    seen.add(cursor);
  }
  return raw;
}

/**
 * The root's children as a forest, siblings ordered by name, each nested under
 * its resolved parent. The root and Trash are left out: the root is not a row,
 * and the tree appends the Trash row itself.
 */
export function buildGroupTree(groups: Group[]): GroupNode[] {
  const byId = new Map(groups.map((g) => [g.id, g]));
  const childrenOf = new Map<string, Group[]>();
  for (const group of groups) {
    if (group.id === ROOT_ID || group.id === TRASH_ID) continue;
    const parentId = effectiveParentId(group.id, byId);
    const list = childrenOf.get(parentId);
    if (list) list.push(group);
    else childrenOf.set(parentId, [group]);
  }
  function build(parentId: string): GroupNode[] {
    return (childrenOf.get(parentId) ?? [])
      .sort((a, b) => a.name.localeCompare(b.name))
      .map((group) => ({ group, children: build(group.id) }));
  }
  return build(ROOT_ID);
}

/** `node`'s own id plus every id in its subtree, collected into `into`. */
export function collectIds(node: GroupNode, into: Set<string>): void {
  into.add(node.group.id);
  for (const child of node.children) collectIds(child, into);
}

function findNode(nodes: GroupNode[], id: string): GroupNode | undefined {
  for (const node of nodes) {
    if (node.group.id === id) return node;
    const found = findNode(node.children, id);
    if (found) return found;
  }
  return undefined;
}

/**
 * `groupId` itself plus every id in its subtree, through the same resolved
 * forest. Empty when `groupId` names no group in the tree (the root and Trash
 * included).
 */
export function descendantIds(groupId: string, groups: Group[]): Set<string> {
  const node = findNode(buildGroupTree(groups), groupId);
  const ids = new Set<string>();
  if (node) collectIds(node, ids);
  return ids;
}

/**
 * The names from the root down to `id`, joined with " / ", walking resolved
 * parents so a synced cycle or a dangling parent cannot hang the label. The
 * root itself reads "Root"; an id no group answers to reads as the id.
 */
export function groupPath(id: string, byId: Map<string, Group>): string {
  if (id === ROOT_ID) return "Root";
  const names: string[] = [];
  const seen = new Set<string>();
  let cursor: string | null = id;
  while (cursor !== null && cursor !== ROOT_ID && !seen.has(cursor)) {
    seen.add(cursor);
    const group = byId.get(cursor);
    if (!group) break;
    names.unshift(group.name);
    cursor = effectiveParentId(cursor, byId);
  }
  return names.length > 0 ? names.join(" / ") : id;
}
