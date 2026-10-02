import { open } from "@tauri-apps/plugin-dialog";
import { create } from "zustand";

import { toast } from "@/components/ui/toast";
import { useVaultStore } from "@/modules/vault/store";

/** Which import or export dialog is open. The two imports carry the picked
 *  file, because the native picker runs before the dialog. */
export type BackupDialogState =
  | { kind: "import-csv"; path: string }
  | { kind: "import-backup"; path: string }
  | { kind: "export-backup" }
  | { kind: "export-csv" };

type BackupStore = {
  dialog: BackupDialogState | null;
  setDialog: (dialog: BackupDialogState | null) => void;
};

export const useBackupStore = create<BackupStore>((set) => ({
  dialog: null,
  setDialog: (dialog) => set({ dialog }),
}));

async function pickInto(
  kind: "import-csv" | "import-backup",
  filter: { name: string; extensions: string[] },
): Promise<void> {
  try {
    const path = await open({ multiple: false, directory: false, filters: [filter] });
    if (typeof path !== "string") return;
    // The picker can resolve after the vault locked behind it; opening a
    // dialog then would put an import on the unlock screen.
    if (useVaultStore.getState().status?.locked) return;
    useBackupStore.getState().setDialog({ kind, path });
  } catch (e) {
    toast(String(e), { variant: "error" });
  }
}

export function pickCsvImport(): Promise<void> {
  return pickInto("import-csv", { name: "CSV", extensions: ["csv"] });
}

export function pickBackupImport(): Promise<void> {
  return pickInto("import-backup", { name: "Subclave backup", extensions: ["subclave-backup"] });
}
