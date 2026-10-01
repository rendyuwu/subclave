import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";

/**
 * The confirmation that turns an empty remote into a new sync vault.
 *
 * Kept apart from the form because creating mints a keyfile on the remote: this
 * is the one moment the consequence can be said out loud, and the answer
 * decides whether the device mints one or joins the one another device just
 * wrote. A rejection is shown HERE rather than under the form, because it is
 * the dialog's own request that failed.
 */
export function FreshRemoteDialog({
  open,
  onOpenChange,
  busy,
  error,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  busy: boolean;
  error: string | null;
  onConfirm: () => void;
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>No Subclave data at this location</AlertDialogTitle>
          <AlertDialogDescription>Create a new sync vault here?</AlertDialogDescription>
        </AlertDialogHeader>
        {error ? (
          <p
            role="alert"
            className="text-destructive text-[11.5px] leading-relaxed break-words whitespace-pre-wrap"
          >
            {error}
          </p>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel>
          {/* A plain Button rather than `AlertDialogAction`: that closes the
              dialog on click, and this request has to be able to fail with the
              dialog still open so the reason is readable. */}
          <Button disabled={busy} onClick={onConfirm}>
            {busy ? "Creating" : "Create"}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
