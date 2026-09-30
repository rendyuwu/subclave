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
import { describeVaultError } from "./errors";
import { quitSubclave } from "./ipc";
import { useVaultStore } from "./store";

/**
 * Shown when the app asked to quit while a write is still failing. Both quit
 * paths go through `quit_subclave`, which drops the parked seal before exiting
 * so the exit cannot re-enter this confirmation.
 */
export function QuitConfirmDialog() {
  const quitPrompt = useVaultStore((s) => s.quitPrompt);
  const dismissQuit = useVaultStore((s) => s.dismissQuit);
  const retrySave = useVaultStore((s) => s.retrySave);
  const [busy, setBusy] = useState(false);

  const retryThenQuit = async () => {
    setBusy(true);
    try {
      await retrySave();
      if (!useVaultStore.getState().status?.savePending) {
        void quitSubclave().catch((err) =>
          toast(describeVaultError(String(err)), { variant: "error" }),
        );
        return;
      }
      toast("The vault still has unsaved changes.", { variant: "error" });
    } catch (err) {
      toast(describeVaultError(String(err)), { variant: "error" });
    } finally {
      setBusy(false);
    }
  };

  return (
    <AlertDialog
      open={quitPrompt}
      onOpenChange={(open) => {
        if (!open) dismissQuit();
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Quit Subclave?</AlertDialogTitle>
          <AlertDialogDescription>
            A vault write is still failing. Quitting now loses the changes that have not been saved.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          {/* preventDefault keeps the dialog open on a failed retry; the app
              only leaves through `quitSubclave`. */}
          <AlertDialogAction
            variant="outline"
            disabled={busy}
            onClick={(e) => {
              e.preventDefault();
              void retryThenQuit();
            }}
          >
            {busy ? <Spinner /> : null}
            Retry now
          </AlertDialogAction>
          <AlertDialogAction
            variant="destructive"
            onClick={(e) => {
              e.preventDefault();
              void quitSubclave().catch((err) =>
                toast(describeVaultError(String(err)), { variant: "error" }),
              );
            }}
          >
            Quit anyway
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
