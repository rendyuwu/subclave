// The vault mutations the group tree runs: re-parenting, deleting a group, and
// emptying Trash. Each one touches the store, so they live outside the pure
// model and are called from the pane's menus, dialogs and drop targets.

import { toast } from "@/components/ui/toast";
import { describeVaultError } from "@/modules/vault/errors";
import { vaultEntryDelete, vaultGroupDelete, vaultGroupUpsert } from "@/modules/vault/ipc";
import { TRASH_SCOPE } from "@/modules/vault/list/derive";
import { useVaultStore } from "@/modules/vault/store";
import type { EntrySummary, Group } from "@/modules/vault/types";

/** Run a mutation, surfacing a refusal as an error toast. */
export async function runVaultAction(action: () => Promise<void>): Promise<void> {
  try {
    await action();
  } catch (error) {
    toast(describeVaultError(String(error)), { variant: "error" });
  }
}

/** Re-parent a group in place, keeping its name, icon and colour. */
export function moveGroup(group: Group, parentId: string): void {
  void runVaultAction(async () => {
    await vaultGroupUpsert({
      id: group.id,
      parentId,
      name: group.name,
      icon: group.icon,
      color: group.color,
    });
    await useVaultStore.getState().refresh();
  });
}

/** Delete a group. The backend refuses one that still holds entries or children. */
export function deleteGroup(group: Group): void {
  void runVaultAction(async () => {
    await vaultGroupDelete(group.id);
    await useVaultStore.getState().refresh();
  });
}

/** Permanently delete every entry sitting in Trash. */
export function emptyTrash(entries: EntrySummary[]): void {
  void runVaultAction(async () => {
    const ids = entries.filter((entry) => entry.groupId === TRASH_SCOPE).map((entry) => entry.id);
    if (ids.length === 0) return;
    await vaultEntryDelete(ids);
    await useVaultStore.getState().refresh();
  });
}
