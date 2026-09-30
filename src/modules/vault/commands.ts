import { useMemo } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { toast } from "@/components/ui/toast";
import { ROOT_ID } from "@/modules/groups/groupTree";
import type { ShortcutHandlers } from "@/modules/shortcuts";

import { copyToastText } from "./copy";
import { describeVaultError } from "./errors";
import * as vault from "./ipc";
import { ALL_SCOPE, FAVORITES_SCOPE, TRASH_SCOPE } from "./list/derive";
import { useVaultStore } from "./store";

/**
 * The group a new entry should land in for the current scope: the scope itself
 * when it names a real group other than Trash, and the root group otherwise
 * (All, Favourites and Trash all create at the root).
 */
export function defaultGroupForScope(scope: string): string {
  if (scope === ALL_SCOPE || scope === FAVORITES_SCOPE || scope === TRASH_SCOPE) {
    return ROOT_ID;
  }
  return scope;
}

/** Run a vault mutation, refresh the list on success, toast a rejection. */
export function runVaultMutation(action: () => Promise<unknown>): void {
  void action()
    .then(() => useVaultStore.getState().refresh())
    .catch((error) => toast(describeVaultError(String(error)), { variant: "error" }));
}

/**
 * Copy one field through Rust, which owns the clipboard and its clear timer.
 * The toast says what was copied and, when the clipboard will clear, how long
 * is left.
 */
export async function copyEntryField(id: string, field: string, kind: string): Promise<void> {
  try {
    const { clearsAt } = await vault.clipCopyField(id, field);
    toast(copyToastText(kind, clearsAt, Date.now()), { variant: "success" });
  } catch (error) {
    toast(describeVaultError(String(error)), { variant: "error" });
  }
}

/** Open an external URL, reporting the opener's refusal instead of failing mute. */
export async function openEntryUrl(url: string): Promise<void> {
  try {
    await openUrl(url);
  } catch (error) {
    toast(describeVaultError(String(error)), { variant: "error" });
  }
}

/** The entry's first URL, loading its detail when the pane has not already. */
export async function openEntryUrlById(id: string): Promise<void> {
  let detail = useVaultStore.getState().detail;
  if (!detail || detail.id !== id) {
    try {
      detail = await vault.vaultEntryGet(id);
    } catch (error) {
      toast(describeVaultError(String(error)), { variant: "error" });
      return;
    }
  }
  const url = detail.urls[0]?.url;
  if (!url) {
    toast("This entry has no URL.", { variant: "error" });
    return;
  }
  await openEntryUrl(url);
}

/** Copy one of the selected entry's secret fields, guarding absent ones. */
async function copySelected(field: "password" | "username" | "totp"): Promise<void> {
  const { selectedId, entries } = useVaultStore.getState();
  if (selectedId === null) return;
  const entry = entries.find((candidate) => candidate.id === selectedId);
  if (!entry) return;
  if (field === "password" && !entry.hasPassword) return;
  if (field === "totp" && !entry.hasTotp) return;
  await copyEntryField(entry.id, field, field);
}

/**
 * The keyboard commands the vault workspace owns. Handlers read the store
 * imperatively so a registered closure never captures stale state. The search
 * field registers `search.focus` itself, because only it holds the input ref.
 */
export function useVaultCommands(): ShortcutHandlers {
  return useMemo<ShortcutHandlers>(
    () => ({
      "vault.lock": () => {
        void useVaultStore.getState().lock();
      },
      "entry.new": () => {
        const { scope, openEditor } = useVaultStore.getState();
        openEditor(null, defaultGroupForScope(scope));
      },
      "entry.edit": () => {
        const { selectedId, entries, openEditor } = useVaultStore.getState();
        if (selectedId === null) return;
        const entry = entries.find((candidate) => candidate.id === selectedId);
        if (!entry || entry.groupId === TRASH_SCOPE) return;
        openEditor(entry.id, entry.groupId);
      },
      "entry.trash": () => {
        const { selectedId, entries } = useVaultStore.getState();
        if (selectedId === null) return;
        const entry = entries.find((candidate) => candidate.id === selectedId);
        if (!entry || entry.groupId === TRASH_SCOPE) return;
        runVaultMutation(() => vault.vaultEntryTrash([selectedId]));
      },
      "entry.copyPassword": () => void copySelected("password"),
      "entry.copyUsername": () => void copySelected("username"),
      "entry.copyTotp": () => void copySelected("totp"),
      "entry.openUrl": () => {
        const { selectedId } = useVaultStore.getState();
        if (selectedId === null) return;
        void openEntryUrlById(selectedId);
      },
    }),
    [],
  );
}
