import { useState, type ReactNode } from "react";
import { Copy, ExternalLink, Pencil, RotateCcw, Trash2 } from "lucide-react";

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
import { Badge } from "@/components/ui/badge";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { EntryGlyph } from "@/modules/groups/AppearancePicker";
import { groupPath } from "@/modules/groups/groupTree";
import { cn } from "@/lib/utils";

import { copyEntryField, openEntryUrlById, runVaultMutation } from "./commands";
import * as vault from "./ipc";
import { isExpired, TRASH_SCOPE } from "./list/derive";
import { useVaultStore } from "./store";
import type { EntrySummary, Group } from "./types";

export type EntryRowProps = {
  entry: EntrySummary;
  groups: Group[];
  selected: boolean;
  /** This row is the list's one tab stop (roving tabindex). */
  tabStop: boolean;
  onSelect: () => void;
  rowRef: (el: HTMLDivElement | null) => void;
};

/** Every group this entry can move to: Trash is a refusal and the current group
 *  is a no-op, so both are left out of the submenu. */
function moveTargets(entry: EntrySummary, groups: Group[]): { id: string; label: string }[] {
  const byId = new Map(groups.map((group) => [group.id, group]));
  return groups
    .filter((group) => group.id !== TRASH_SCOPE && group.id !== entry.groupId)
    .map((group) => ({ id: group.id, label: groupPath(group.id, byId) }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

export function EntryRow({
  entry,
  groups,
  selected,
  tabStop,
  onSelect,
  rowRef,
}: EntryRowProps): ReactNode {
  const [confirmDelete, setConfirmDelete] = useState(false);
  const inTrash = entry.groupId === TRASH_SCOPE;
  const expired = isExpired(entry, Date.now());
  const targets = moveTargets(entry, groups);

  const edit = () => {
    useVaultStore.getState().openEditor(entry.id, entry.groupId);
  };
  const trash = () => runVaultMutation(() => vault.vaultEntryTrash([entry.id]));
  const restore = () => runVaultMutation(() => vault.vaultEntryRestore([entry.id]));

  const row = (
    <div
      ref={rowRef}
      role="option"
      aria-selected={selected}
      tabIndex={tabStop ? 0 : -1}
      draggable={!inTrash}
      onDragStart={(e) => e.dataTransfer.setData("text/plain", entry.id)}
      onClick={onSelect}
      className={cn(
        "vault-row focus-visible:ring-ring/50 flex cursor-default items-center gap-2 rounded-md px-2 py-1.5 outline-none focus-visible:ring-2",
        selected ? "bg-accent text-accent-foreground" : "hover:bg-muted/50",
      )}
    >
      <EntryGlyph icon={entry.icon} color={entry.color} />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-sm font-medium">{entry.title}</span>
          {expired ? (
            <Badge variant="destructive" className="h-4 shrink-0 px-1.5 text-[10px]">
              Expired
            </Badge>
          ) : null}
        </div>
        <div className="text-muted-foreground truncate text-xs">
          {[entry.username, entry.primaryHost].filter(Boolean).join(" · ")}
        </div>
      </div>
    </div>
  );

  return (
    <>
      <ContextMenu>
        <ContextMenuTrigger asChild>{row}</ContextMenuTrigger>
        <ContextMenuContent className="w-52">
          {inTrash ? (
            <>
              <ContextMenuItem onSelect={restore}>
                <RotateCcw strokeWidth={1.75} />
                Restore
              </ContextMenuItem>
              <ContextMenuItem variant="destructive" onSelect={() => setConfirmDelete(true)}>
                <Trash2 strokeWidth={1.75} />
                Delete permanently
              </ContextMenuItem>
            </>
          ) : (
            <>
              <ContextMenuItem
                onSelect={() => void copyEntryField(entry.id, "username", "username")}
              >
                <Copy strokeWidth={1.75} />
                Copy username
              </ContextMenuItem>
              {entry.hasPassword ? (
                <ContextMenuItem
                  onSelect={() => void copyEntryField(entry.id, "password", "password")}
                >
                  <Copy strokeWidth={1.75} />
                  Copy password
                </ContextMenuItem>
              ) : null}
              {entry.hasTotp ? (
                <ContextMenuItem onSelect={() => void copyEntryField(entry.id, "totp", "totp")}>
                  <Copy strokeWidth={1.75} />
                  Copy TOTP
                </ContextMenuItem>
              ) : null}
              {entry.primaryHost ? (
                <ContextMenuItem onSelect={() => void openEntryUrlById(entry.id)}>
                  <ExternalLink strokeWidth={1.75} />
                  Open URL
                </ContextMenuItem>
              ) : null}
              <ContextMenuSeparator />
              <ContextMenuItem onSelect={edit}>
                <Pencil strokeWidth={1.75} />
                Edit
              </ContextMenuItem>
              {targets.length > 0 ? (
                <ContextMenuSub>
                  <ContextMenuSubTrigger>Move to…</ContextMenuSubTrigger>
                  <ContextMenuSubContent className="max-h-72 w-56 overflow-y-auto">
                    {targets.map((target) => (
                      <ContextMenuItem
                        key={target.id}
                        onSelect={() =>
                          runVaultMutation(() => vault.vaultEntryMove([entry.id], target.id))
                        }
                      >
                        {target.label}
                      </ContextMenuItem>
                    ))}
                  </ContextMenuSubContent>
                </ContextMenuSub>
              ) : null}
              <ContextMenuSeparator />
              <ContextMenuItem variant="destructive" onSelect={trash}>
                <Trash2 strokeWidth={1.75} />
                Move to Trash
              </ContextMenuItem>
            </>
          )}
        </ContextMenuContent>
      </ContextMenu>

      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete permanently?</AlertDialogTitle>
            <AlertDialogDescription>This cannot be undone.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                setConfirmDelete(false);
                runVaultMutation(() => vault.vaultEntryDelete([entry.id]));
              }}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
