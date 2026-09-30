import { useState, type ReactNode } from "react";
import { ChevronRight, Copy, RotateCcw } from "lucide-react";

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
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";
import { formatRelativeTime } from "@/lib/format";
import { SecretField } from "@/modules/vault/editor/SecretField";

import { copyEntryField, runVaultMutation } from "./commands";
import { vaultEntryRestoreVersion, vaultEntryReveal } from "./ipc";
import type { DetailVersion, VersionReason } from "./types";

const REASON_LABELS: Record<VersionReason, string> = {
  edit: "Edited",
  restore: "Restored",
  conflict: "Conflict",
};

/** The entry's past passwords: when each version landed, why and which fields
 *  changed, with reveal, copy and restore per row. */
export function HistoryList({ id, history }: { id: string; history: DetailVersion[] }): ReactNode {
  const [open, setOpen] = useState(false);
  const [confirmAt, setConfirmAt] = useState<number | null>(null);

  return (
    <Collapsible open={open} onOpenChange={setOpen} className="px-3 py-2">
      <CollapsibleTrigger asChild>
        <Button variant="ghost" size="xs" className="w-full justify-start">
          <ChevronRight
            strokeWidth={1.75}
            className={cn("transition-transform", open && "rotate-90")}
          />
          History ({history.length})
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <ul className="mt-1.5 flex flex-col gap-1.5">
          {history.map((version) => (
            <li key={version.updatedAt} className="border-border/60 rounded-md border p-2 text-xs">
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium">{formatRelativeTime(version.updatedAt)}</span>
                <span className="text-muted-foreground">{REASON_LABELS[version.reason]}</span>
              </div>
              {version.changed.length > 0 ? (
                <div className="text-muted-foreground mt-0.5 truncate">
                  {version.changed.join(", ")}
                </div>
              ) : null}
              <div className="mt-1.5">
                <SecretField
                  readOnly
                  value=""
                  ariaLabel={`History password from ${formatRelativeTime(version.updatedAt)}`}
                  resetKey={id}
                  onReveal={() => vaultEntryReveal(id, `history:${version.updatedAt}:password`)}
                />
              </div>
              <div className="mt-1.5 flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="xs"
                  onClick={() =>
                    void copyEntryField(id, `history:${version.updatedAt}:password`, "password")
                  }
                >
                  <Copy strokeWidth={1.75} />
                  Copy
                </Button>
                <Button variant="ghost" size="xs" onClick={() => setConfirmAt(version.updatedAt)}>
                  <RotateCcw strokeWidth={1.75} />
                  Restore
                </Button>
              </div>
            </li>
          ))}
        </ul>
      </CollapsibleContent>

      <AlertDialog
        open={confirmAt !== null}
        onOpenChange={(next) => {
          if (!next) setConfirmAt(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Restore this version?</AlertDialogTitle>
            <AlertDialogDescription>
              The entry&apos;s current password is replaced and the restore is recorded in the
              history.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const at = confirmAt;
                setConfirmAt(null);
                if (at !== null) runVaultMutation(() => vaultEntryRestoreVersion(id, at));
              }}
            >
              Restore
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Collapsible>
  );
}
