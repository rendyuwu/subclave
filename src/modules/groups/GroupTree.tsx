// The left pane: the group list as a tree, plus its create / rename / move /
// delete UI. Zero prop and store-driven, so the pane owner only has to mount
// it. The Trash row lives here too, since its only action (empty it) is a group
// action.

import { EntryGlyph } from "./AppearancePicker";
import { GroupEditorDialog } from "./GroupEditorDialog";
import { ROOT_ID, TRASH_ID, buildGroupTree, descendantIds, type GroupNode } from "./groupTreeModel";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { toast } from "@/components/ui/toast";
import { cn } from "@/lib/utils";
import { describeVaultError } from "@/modules/vault/errors";
import {
  vaultEntryDelete,
  vaultEntryMove,
  vaultGroupDelete,
  vaultGroupUpsert,
} from "@/modules/vault/ipc";
import { groupCounts, ALL_SCOPE, FAVORITES_SCOPE, TRASH_SCOPE } from "@/modules/vault/list/derive";
import { useVaultStore } from "@/modules/vault/store";
import type { Group } from "@/modules/vault/types";
import { treeKeyAction, type TreeKeyItem } from "./treeKeyboard";
import { ChevronRight, FolderInput, MoreHorizontal, Pencil, Plus, Trash2 } from "lucide-react";
import { useMemo, useRef, useState, type ReactNode } from "react";

/** The save-login target the extension writes into. Reserved like the root. */
const BROWSER_ID = "browser";

/** A pill toggle: label, an optional tabular-nums count, pressed state. */
export function Chip({
  label,
  count,
  selected,
  onClick,
}: {
  label: string;
  count?: number;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium transition-colors",
        selected
          ? "bg-accent text-accent-foreground border-transparent"
          : "border-border text-muted-foreground hover:bg-muted/50",
      )}
    >
      {label}
      {count !== undefined ? (
        <span className={cn("tabular-nums", selected ? "opacity-80" : "text-muted-foreground/70")}>
          {count}
        </span>
      ) : null}
    </button>
  );
}

/** One rendered row: the two pseudo rows, a group, or the Trash row. */
type Row = {
  id: string;
  label: string;
  count: number;
  level: number;
  parentId: string | null;
  hasChildren: boolean;
  expanded: boolean;
  group: Group | null;
};

/** Where a group can move to: the root, plus every group that is not itself,
 *  one of its own descendants, its current parent, or Trash. */
function moveTargets(
  group: Group,
  tree: GroupNode[],
  groups: Group[],
): { id: string; label: string }[] {
  const excluded = descendantIds(group.id, groups);
  const currentParent = group.parentId ?? ROOT_ID;
  const out: { id: string; label: string }[] = [];
  if (currentParent !== ROOT_ID) out.push({ id: ROOT_ID, label: "Root" });
  function walk(nodes: GroupNode[], depth: number): void {
    for (const node of nodes) {
      // The current parent is skipped as a target but still descended into:
      // its other children are the moved group's siblings, and they are legal
      // targets, so pruning the whole subtree hides them.
      if (!excluded.has(node.group.id) && node.group.id !== currentParent) {
        out.push({ id: node.group.id, label: `${"\u00a0\u00a0".repeat(depth)}${node.group.name}` });
      }
      walk(node.children, depth + 1);
    }
  }
  walk(tree, 0);
  return out;
}

export function GroupTree(): ReactNode {
  const groups = useVaultStore((s) => s.groups);
  const entries = useVaultStore((s) => s.entries);
  const scope = useVaultStore((s) => s.scope);
  const expanded = useVaultStore((s) => s.expanded);
  const selectScope = useVaultStore((s) => s.selectScope);
  const toggleExpanded = useVaultStore((s) => s.toggleExpanded);

  const [request, setRequest] = useState<{ group: Group | null; parentId: string | null } | null>(
    null,
  );
  const [deleting, setDeleting] = useState<Group | null>(null);
  const [emptyTrashOpen, setEmptyTrashOpen] = useState(false);
  const [dragOverId, setDragOverId] = useState<string | null>(null);

  const containerRef = useRef<HTMLDivElement>(null);
  const counts = useMemo(() => groupCounts(entries, groups), [entries, groups]);
  const tree = useMemo(() => buildGroupTree(groups), [groups]);
  const trashGroup = groups.find((group) => group.id === TRASH_SCOPE) ?? null;

  const rows = useMemo(() => {
    const out: Row[] = [
      {
        id: ALL_SCOPE,
        label: "All",
        count: counts.get(ALL_SCOPE) ?? 0,
        level: 0,
        parentId: null,
        hasChildren: false,
        expanded: false,
        group: null,
      },
      {
        id: FAVORITES_SCOPE,
        label: "Favourites",
        count: counts.get(FAVORITES_SCOPE) ?? 0,
        level: 0,
        parentId: null,
        hasChildren: false,
        expanded: false,
        group: null,
      },
    ];
    function walk(nodes: GroupNode[], level: number, parentId: string | null): void {
      for (const node of nodes) {
        const hasChildren = node.children.length > 0;
        const isExpanded = expanded.includes(node.group.id);
        out.push({
          id: node.group.id,
          label: node.group.name,
          count: counts.get(node.group.id) ?? 0,
          level,
          parentId,
          hasChildren,
          expanded: isExpanded,
          group: node.group,
        });
        if (hasChildren && isExpanded) walk(node.children, level + 1, node.group.id);
      }
    }
    walk(tree, 0, null);
    out.push({
      id: TRASH_SCOPE,
      label: "Trash",
      count: counts.get(TRASH_SCOPE) ?? 0,
      level: 0,
      parentId: null,
      hasChildren: false,
      expanded: false,
      group: trashGroup,
    });
    return out;
  }, [tree, counts, expanded, trashGroup]);

  const flat: TreeKeyItem[] = rows.map((row) => ({
    id: row.id,
    parentId: row.parentId,
    hasChildren: row.hasChildren,
    expanded: row.expanded,
  }));
  const rovingId = rows.some((row) => row.id === scope) ? scope : rows[0]?.id;

  const run = async (action: () => Promise<void>) => {
    try {
      await action();
    } catch (error) {
      toast(describeVaultError(String(error)), { variant: "error" });
    }
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const current = flat.findIndex((item) => item.id === scope);
    const action = treeKeyAction(event.key, flat, current);
    if (!action) return;
    event.preventDefault();
    const target = flat[action.index];
    if (action.kind === "toggle") toggleExpanded(target.id);
    else selectScope(target.id);
    containerRef.current?.querySelector<HTMLElement>(`[data-tree-row="${target.id}"]`)?.focus();
  };

  const moveGroup = (group: Group, parentId: string) =>
    run(async () => {
      await vaultGroupUpsert({
        id: group.id,
        parentId,
        name: group.name,
        icon: group.icon,
        color: group.color,
      });
      await useVaultStore.getState().refresh();
    });

  const deleteGroup = (group: Group) =>
    run(async () => {
      await vaultGroupDelete(group.id);
      await useVaultStore.getState().refresh();
    });

  const emptyTrash = () =>
    run(async () => {
      const ids = entries.filter((entry) => entry.groupId === TRASH_SCOPE).map((entry) => entry.id);
      if (ids.length === 0) return;
      await vaultEntryDelete(ids);
      await useVaultStore.getState().refresh();
    });

  const dropOnGroup = (event: React.DragEvent<HTMLDivElement>, groupId: string) => {
    event.preventDefault();
    setDragOverId(null);
    const id = event.dataTransfer.getData("text/plain");
    if (!id) return;
    const entry = entries.find((item) => item.id === id);
    if (!entry || entry.groupId === groupId) return;
    void run(async () => {
      await vaultEntryMove([id], groupId);
      await useVaultStore.getState().refresh();
    });
  };

  const groupMenu = (group: Group) => {
    const reserved = group.id === BROWSER_ID;
    return (
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            aria-label={`Actions for ${group.name}`}
            onClick={(event) => event.stopPropagation()}
            className="text-muted-foreground hover:text-foreground focus-visible:ring-ring/40 size-5 shrink-0 rounded outline-none focus-visible:ring-1"
          >
            <MoreHorizontal size={14} strokeWidth={2} />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start">
          <DropdownMenuItem onSelect={() => setRequest({ group: null, parentId: group.id })}>
            <Plus size={14} strokeWidth={1.75} />
            New sub-group
          </DropdownMenuItem>
          {reserved ? null : (
            <>
              <DropdownMenuItem onSelect={() => setRequest({ group, parentId: group.parentId })}>
                <Pencil size={14} strokeWidth={1.75} />
                Rename
              </DropdownMenuItem>
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  <FolderInput size={14} strokeWidth={1.75} />
                  Move to…
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent>
                  {moveTargets(group, tree, groups).map((target) => (
                    <DropdownMenuItem
                      key={target.id}
                      onSelect={() => void moveGroup(group, target.id)}
                    >
                      {target.label}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuSubContent>
              </DropdownMenuSub>
              <DropdownMenuItem variant="destructive" onSelect={() => setDeleting(group)}>
                <Trash2 size={14} strokeWidth={1.75} />
                Delete
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
    );
  };

  const trashMenu = (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label="Actions for Trash"
          onClick={(event) => event.stopPropagation()}
          className="text-muted-foreground hover:text-foreground focus-visible:ring-ring/40 size-5 shrink-0 rounded outline-none focus-visible:ring-1"
        >
          <MoreHorizontal size={14} strokeWidth={2} />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        <DropdownMenuItem variant="destructive" onSelect={() => setEmptyTrashOpen(true)}>
          <Trash2 size={14} strokeWidth={1.75} />
          Empty trash
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );

  const trashCount = counts.get(TRASH_SCOPE) ?? 0;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex items-center justify-between px-2 py-1.5">
        <span className="text-muted-foreground text-[11px] font-medium">Groups</span>
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label="New group"
          onClick={() => setRequest({ group: null, parentId: ROOT_ID })}
        >
          <Plus size={13} strokeWidth={2} />
        </Button>
      </div>

      <div
        ref={containerRef}
        role="tree"
        aria-label="Groups"
        onKeyDown={onKeyDown}
        className="flex min-h-0 flex-col gap-0.5 overflow-y-auto px-1.5 pb-2"
      >
        {rows.map((row) => {
          const selected = scope === row.id;
          const isDropTarget = row.group !== null && row.group.id !== TRASH_ID;
          return (
            <div
              key={row.id}
              data-tree-row={row.id}
              role="treeitem"
              aria-selected={selected}
              aria-level={row.level + 1}
              aria-expanded={row.hasChildren ? row.expanded : undefined}
              tabIndex={row.id === rovingId ? 0 : -1}
              onClick={() => selectScope(row.id)}
              onDragOver={
                isDropTarget
                  ? (event) => {
                      event.preventDefault();
                      event.dataTransfer.dropEffect = "move";
                      setDragOverId(row.id);
                    }
                  : undefined
              }
              onDragLeave={isDropTarget ? () => setDragOverId(null) : undefined}
              onDrop={isDropTarget ? (event) => dropOnGroup(event, row.id) : undefined}
              style={{ paddingLeft: row.level * 14 + 6 }}
              className={cn(
                "group/row focus-visible:ring-ring/40 flex cursor-pointer items-center gap-1.5 rounded-md py-1 pr-1 text-[12.5px] outline-none select-none focus-visible:ring-1",
                selected
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-muted/50 hover:text-foreground",
                dragOverId === row.id && "bg-accent/60",
              )}
            >
              {row.hasChildren ? (
                <button
                  type="button"
                  tabIndex={-1}
                  aria-label={row.expanded ? `Collapse ${row.label}` : `Expand ${row.label}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    toggleExpanded(row.id);
                  }}
                  className="text-muted-foreground flex size-4 shrink-0 items-center justify-center"
                >
                  <ChevronRight
                    size={12}
                    strokeWidth={2.25}
                    className={cn("transition-transform", row.expanded && "rotate-90")}
                  />
                </button>
              ) : (
                <span className="size-4 shrink-0" />
              )}

              {row.group ? <EntryGlyph icon={row.group.icon} color={row.group.color} /> : null}

              <span className="min-w-0 flex-1 truncate">{row.label}</span>
              <span className="text-[11px] tabular-nums opacity-70">{row.count}</span>

              {row.id === TRASH_SCOPE ? trashMenu : row.group ? groupMenu(row.group) : null}
            </div>
          );
        })}
      </div>

      <GroupEditorDialog request={request} onClose={() => setRequest(null)} />

      <AlertDialog open={deleting !== null} onOpenChange={(open) => !open && setDeleting(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete group &quot;{deleting?.name}&quot;?</AlertDialogTitle>
            <AlertDialogDescription>
              This cannot be undone. A group that still holds entries or sub-groups cannot be
              deleted.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                const target = deleting;
                setDeleting(null);
                if (target) void deleteGroup(target);
              }}
            >
              Delete group
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={emptyTrashOpen} onOpenChange={setEmptyTrashOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Empty trash?</AlertDialogTitle>
            <AlertDialogDescription>
              {trashCount} {trashCount === 1 ? "entry" : "entries"} will be deleted permanently.
              This cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setEmptyTrashOpen(false);
                void emptyTrash();
              }}
            >
              Empty trash
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
