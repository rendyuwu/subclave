/** Arrow-key navigation over a flattened, visible tree, taken from Tervia's file
 *  explorer so the group tree reuses it. Pure: the caller owns selection,
 *  expansion and focus, and calls `preventDefault` only when this returns non-null. */
export type TreeKeyItem = {
  id: string;
  parentId: string | null;
  hasChildren: boolean;
  expanded: boolean;
};
export type TreeKeyAction =
  | { kind: "select"; index: number }
  | { kind: "toggle"; index: number }
  | { kind: "activate"; index: number };

export function treeKeyAction(
  key: string,
  items: readonly TreeKeyItem[],
  current: number,
): TreeKeyAction | null {
  const last = items.length - 1;
  if (last < 0) return null;
  switch (key) {
    case "ArrowDown":
      return { kind: "select", index: current < 0 ? 0 : Math.min(current + 1, last) };
    case "ArrowUp":
      return { kind: "select", index: current < 0 ? last : Math.max(current - 1, 0) };
    case "ArrowRight": {
      if (current < 0 || !items[current].hasChildren) return null;
      return items[current].expanded
        ? { kind: "select", index: Math.min(current + 1, last) }
        : { kind: "toggle", index: current };
    }
    case "ArrowLeft": {
      if (current < 0) return null;
      const item = items[current];
      if (item.hasChildren && item.expanded) return { kind: "toggle", index: current };
      const parent = item.parentId === null ? -1 : items.findIndex((i) => i.id === item.parentId);
      return parent < 0 ? null : { kind: "select", index: parent };
    }
    case "Enter":
      if (current < 0) return null;
      return items[current].hasChildren
        ? { kind: "toggle", index: current }
        : { kind: "activate", index: current };
    default:
      return null;
  }
}
