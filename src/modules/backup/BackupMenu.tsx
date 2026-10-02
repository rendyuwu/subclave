import { ArchiveRestore, DatabaseBackup, FileInput, FileOutput } from "lucide-react";
import { useEffect, useMemo } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { IconTooltip } from "@/components/ui/icon-tooltip";
import { TOOLBAR_HOVER } from "@/lib/toolbarButton";
import { cn } from "@/lib/utils";

import { BackupDialog, type BackupMode } from "./BackupDialog";
import { ExportCsvDialog } from "./ExportCsvDialog";
import { ImportCsvDialog } from "./ImportCsvDialog";
import { pickBackupImport, pickCsvImport, useBackupStore } from "./store";

/**
 * The header's import and export menu, and the dialogs it opens. App mounts
 * it only while unlocked, and unmounting clears the open dialog, so no dialog
 * outlives a lock or reopens after the next unlock.
 */
export function BackupMenu() {
  const dialog = useBackupStore((s) => s.dialog);
  const setDialog = useBackupStore((s) => s.setDialog);
  useEffect(() => () => setDialog(null), [setDialog]);

  // One `mode` object per opening: BackupDialog resets on a new identity.
  const backupMode = useMemo<BackupMode | null>(() => {
    if (dialog?.kind === "export-backup") return { kind: "export" };
    if (dialog?.kind === "import-backup") return { kind: "import", path: dialog.path };
    return null;
  }, [dialog]);
  const close = () => setDialog(null);

  return (
    <>
      <DropdownMenu>
        <IconTooltip label="Import and export">
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className={cn("text-muted-foreground", TOOLBAR_HOVER, "size-7 shrink-0 rounded-md")}
              aria-label="Import and export"
            >
              <DatabaseBackup size={15} strokeWidth={1.75} />
            </Button>
          </DropdownMenuTrigger>
        </IconTooltip>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => void pickCsvImport()}>
            <FileInput size={14} strokeWidth={1.75} />
            Import CSV…
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void pickBackupImport()}>
            <ArchiveRestore size={14} strokeWidth={1.75} />
            Import backup…
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => setDialog({ kind: "export-backup" })}>
            <DatabaseBackup size={14} strokeWidth={1.75} />
            Export backup…
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => setDialog({ kind: "export-csv" })}>
            <FileOutput size={14} strokeWidth={1.75} />
            Export CSV…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <ImportCsvDialog path={dialog?.kind === "import-csv" ? dialog.path : null} onClose={close} />
      <BackupDialog mode={backupMode} onClose={close} />
      <ExportCsvDialog open={dialog?.kind === "export-csv"} onClose={close} />
    </>
  );
}
