import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import { formatDateTime } from "@/lib/format";
import { useState } from "react";
import { CircleAlert } from "lucide-react";
import { describeVaultError } from "./errors";
import { useVaultStore } from "./store";

/**
 * Sits above the panes while a write is failing, parked or refused. Three
 * variants, in priority order: the payload came from the backup file and every
 * save is refused, a write failed and is being retried, or a write is parked
 * with no error.
 */
export function SaveFailedBanner() {
  const status = useVaultStore((s) => s.status);
  const saveError = useVaultStore((s) => s.saveError);
  const retrySave = useVaultStore((s) => s.retrySave);
  const restoreSnapshot = useVaultStore((s) => s.restoreSnapshot);
  // A second restore clicked before the first answers would move the freshly
  // written good primary aside, so both actions disable while one is in flight.
  const [busy, setBusy] = useState(false);

  if (!saveError && !status?.savePending && !status?.saveBlocked) return null;

  const run = (action: () => Promise<void>) => {
    setBusy(true);
    void action()
      .catch((err) => toast(describeVaultError(String(err)), { variant: "error" }))
      .finally(() => setBusy(false));
  };

  return (
    <div
      role="alert"
      className="border-destructive/40 bg-destructive/10 text-foreground mx-1.5 mt-1.5 flex shrink-0 items-center gap-3 rounded-md border px-3 py-2 text-xs"
    >
      <CircleAlert size={14} strokeWidth={2} className="text-destructive shrink-0" />
      <span className="min-w-0 flex-1">
        {status?.saveBlocked
          ? "The vault file could not be read. Saving is paused."
          : saveError
            ? `Could not save the vault: ${describeVaultError(saveError)}`
            : "The vault has unsaved changes; they are retried automatically."}
      </span>
      {status?.saveBlocked ? (
        <Button size="xs" variant="outline" disabled={busy} onClick={() => run(restoreSnapshot)}>
          {status.backupAt === null
            ? "Restore the snapshot"
            : `Restore the snapshot from ${formatDateTime(status.backupAt)}`}
        </Button>
      ) : (
        <Button size="xs" variant="outline" disabled={busy} onClick={() => run(retrySave)}>
          {saveError ? "Retry" : "Retry now"}
        </Button>
      )}
    </div>
  );
}
