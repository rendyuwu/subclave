// The left pane: the group list as a tree, plus its create / rename / move /
// delete UI. Zero prop and store-driven, so the pane owner only has to mount
// it. The Trash row lives here too, since its only action (empty it) is a group
// action. The rows live in `GroupRow.tsx`, their menus and dialogs in
// `GroupRowActions.tsx`, and the mutations they run in `groupActions.ts`.

import { GroupEditorDialog, type GroupEditorRequest } from "./GroupEditorDialog";
import { GroupRow } from "./GroupRow";
import {
  DeleteGroupDialog,
  EmptyTrashDialog,
  GroupRowActions,
  TrashRowActions,
} from "./GroupRowActions";
import { deleteGroup, emptyTrash, moveGroup, runVaultAction } from "./groupActions";
import { ROOT_ID, buildGroupTree, treeRows, type TreeRow } from "./groupTreeModel";
import { Button } from "@/components/ui/button";
import { vaultEntryMove } from "@/modules/vault/ipc";
import { groupCounts, TRASH_SCOPE } from "@/modules/vault/list/derive";
import { useVaultStore } from "@/modules/vault/store";
import type { Group } from "@/modules/vault/types";
import { treeKeyAction } from "./treeKeyboard";
import { Plus } from "lucide-react";
import { useMemo, useRef, useState, type ReactNode } from "react";

export function GroupTree(): ReactNode {
  const groups = useVaultStore((s) => s.groups);
  const entries = useVaultStore((s) => s.entries);
  const scope = useVaultStore((s) => s.scope);
  const expanded = useVaultStore((s) => s.expanded);
  const selectScope = useVaultStore((s) => s.selectScope);
  const toggleExpanded = useVaultStore((s) => s.toggleExpanded);

  const [request, setRequest] = useState<GroupEditorRequest | null>(null);
  const [deleting, setDeleting] = useState<Group | null>(null);
  const [emptyTrashOpen, setEmptyTrashOpen] = useState(false);
  const [dragOverId, setDragOverId] = useState<string | null>(null);

  const containerRef = useRef<HTMLDivElement>(null);
  const counts = useMemo(() => groupCounts(entries, groups), [entries, groups]);
  const tree = useMemo(() => buildGroupTree(groups), [groups]);
  const rows = useMemo(() => treeRows({ groups, counts, expanded }), [groups, counts, expanded]);
  const rovingId = rows.some((row) => row.id === scope) ? scope : rows[0]?.id;

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const current = rows.findIndex((item) => item.id === scope);
    const action = treeKeyAction(event.key, rows, current);
    if (!action) return;
    event.preventDefault();
    const target = rows[action.index];
    if (action.kind === "toggle") toggleExpanded(target.id);
    else selectScope(target.id);
    containerRef.current?.querySelector<HTMLElement>(`[data-tree-row="${target.id}"]`)?.focus();
  };

  const dropOnGroup = (event: React.DragEvent<HTMLDivElement>, groupId: string) => {
    event.preventDefault();
    setDragOverId(null);
    const id = event.dataTransfer.getData("text/plain");
    if (!id) return;
    const entry = entries.find((item) => item.id === id);
    if (!entry || entry.groupId === groupId) return;
    void runVaultAction(async () => {
      await vaultEntryMove([id], groupId);
      await useVaultStore.getState().refresh();
    });
  };
  const rowActions = (row: TreeRow) =>
    row.id === TRASH_SCOPE ? (
      <TrashRowActions onEmptyTrash={() => setEmptyTrashOpen(true)} />
    ) : row.group ? (
      <GroupRowActions
        group={row.group}
        tree={tree}
        groups={groups}
        onRequest={setRequest}
        onMove={moveGroup}
        onDelete={setDeleting}
      />
    ) : null;

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
          const isDropTarget = row.group !== null && row.group.id !== TRASH_SCOPE;
          return (
            <GroupRow
              key={row.id}
              row={row}
              selected={scope === row.id}
              roving={row.id === rovingId}
              dragOver={dragOverId === row.id}
              actions={rowActions(row)}
              onSelect={() => selectScope(row.id)}
              onToggleExpand={() => toggleExpanded(row.id)}
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
            />
          );
        })}
      </div>

      <GroupEditorDialog request={request} onClose={() => setRequest(null)} />

      <DeleteGroupDialog
        group={deleting}
        onClose={() => setDeleting(null)}
        onConfirm={deleteGroup}
      />

      <EmptyTrashDialog
        open={emptyTrashOpen}
        count={counts.get(TRASH_SCOPE) ?? 0}
        onOpenChange={setEmptyTrashOpen}
        onConfirm={() => {
          setEmptyTrashOpen(false);
          void emptyTrash(entries);
        }}
      />
    </div>
  );
}
