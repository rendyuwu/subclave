// The trailing menus each tree row can carry: the per-group actions (new
// sub-group, rename, move, delete), the Trash row's empty action, and the two
// AlertDialogs those destructive actions open. The pane owns the state these
// close over, so every action is a callback.

import type { GroupEditorRequest } from "./GroupEditorDialog";
import { moveTargets, type GroupNode } from "./groupTreeModel";
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { Group } from "@/modules/vault/types";
import { FolderInput, MoreHorizontal, Pencil, Plus, Trash2 } from "lucide-react";

/** The save-login target the extension writes into. Reserved like the root. */
const BROWSER_ID = "browser";

/** The per-group dropdown. A reserved group only offers a sub-group: its name
 *  and place come from the extension, not this pane. */
export function GroupRowActions({
  group,
  tree,
  groups,
  onRequest,
  onMove,
  onDelete,
}: {
  group: Group;
  tree: GroupNode[];
  groups: Group[];
  onRequest: (request: GroupEditorRequest) => void;
  onMove: (group: Group, targetId: string) => void;
  onDelete: (group: Group) => void;
}) {
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
        <DropdownMenuItem onSelect={() => onRequest({ group: null, parentId: group.id })}>
          <Plus size={14} strokeWidth={1.75} />
          New sub-group
        </DropdownMenuItem>
        {reserved ? null : (
          <>
            <DropdownMenuItem onSelect={() => onRequest({ group, parentId: group.parentId })}>
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
                  <DropdownMenuItem key={target.id} onSelect={() => onMove(group, target.id)}>
                    {target.label}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuSubContent>
            </DropdownMenuSub>
            <DropdownMenuItem variant="destructive" onSelect={() => onDelete(group)}>
              <Trash2 size={14} strokeWidth={1.75} />
              Delete
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** The Trash row's dropdown: the one action that empties it. */
export function TrashRowActions({ onEmptyTrash }: { onEmptyTrash: () => void }) {
  return (
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
        <DropdownMenuItem variant="destructive" onSelect={onEmptyTrash}>
          <Trash2 size={14} strokeWidth={1.75} />
          Empty trash
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Confirm deleting a group. Open while `group` is non-null. */
export function DeleteGroupDialog({
  group,
  onClose,
  onConfirm,
}: {
  group: Group | null;
  onClose: () => void;
  onConfirm: (group: Group) => void;
}) {
  return (
    <AlertDialog open={group !== null} onOpenChange={(open) => !open && onClose()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete group &quot;{group?.name}&quot;?</AlertDialogTitle>
          <AlertDialogDescription>
            This cannot be undone. A group that still holds entries or sub-groups cannot be deleted.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={() => {
              const target = group;
              onClose();
              if (target) onConfirm(target);
            }}
          >
            Delete group
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** Confirm emptying Trash, the one bulk-destructive group action. */
export function EmptyTrashDialog({
  open,
  count,
  onOpenChange,
  onConfirm,
}: {
  open: boolean;
  count: number;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => void;
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Empty trash?</AlertDialogTitle>
          <AlertDialogDescription>
            {count} {count === 1 ? "entry" : "entries"} will be deleted permanently. This cannot be
            undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={onConfirm}>
            Empty trash
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
