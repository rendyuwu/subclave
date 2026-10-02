import { save } from "@tauri-apps/plugin-dialog";
import { useState } from "react";

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
import { Spinner } from "@/components/ui/spinner";
import { toast } from "@/components/ui/toast";
import { describeVaultError } from "@/modules/vault/errors";

import { exportCsv } from "./ipc";

/** Confirms a plaintext export before the save picker opens. */
export function ExportCsvDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [busy, setBusy] = useState(false);

  const run = async () => {
    setBusy(true);
    try {
      const target = await save({
        defaultPath: "subclave-export.csv",
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (target) {
        await exportCsv(target);
        toast(`Exported to ${target}.`, { variant: "success" });
      }
    } catch (e) {
      toast(describeVaultError(String(e)), { variant: "error" });
    } finally {
      setBusy(false);
      onClose();
    }
  };

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!next && !busy) onClose();
      }}
    >
      <AlertDialogContent showCloseButton={!busy}>
        <AlertDialogHeader>
          <AlertDialogTitle>Export as CSV</AlertDialogTitle>
          <AlertDialogDescription>
            This file will hold every password in plain text.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
          {/* preventDefault keeps the dialog up while the picker and the write
              run; `run` closes it after either. */}
          <AlertDialogAction
            disabled={busy}
            onClick={(e) => {
              e.preventDefault();
              void run();
            }}
          >
            {busy ? <Spinner className="size-3" /> : null}
            Export
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
