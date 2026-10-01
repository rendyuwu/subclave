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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Spinner } from "@/components/ui/spinner";
import { toast } from "@/components/ui/toast";
import { copyToastText } from "@/modules/vault/copy";
import { describeVaultError } from "@/modules/vault/errors";
import { clipCopyField } from "@/modules/vault/ipc";
import { tagCounts } from "@/modules/vault/list/derive";
import { useVaultStore, type EditorRequest } from "@/modules/vault/store";
import { EntryEditorFields } from "./EntryEditorFields";
import { useEntryDraft } from "./useEntryDraft";

// The one entry editor: the dialog shell, its footer actions and the discard
// confirmation. The draft, its load effect and the save live in the hook, and
// the field rows live in `EntryEditorFields`.

export function EntryEditorDialog({
  request,
  onClose,
}: {
  request: EditorRequest | null;
  onClose: () => void;
}) {
  const {
    open,
    draft,
    loading,
    saving,
    discardOpen,
    setDiscardOpen,
    revealKey,
    bumpRevealKey,
    canSave,
    patch,
    attemptClose,
    save,
  } = useEntryDraft(request, onClose);
  const entries = useVaultStore((state) => state.entries);

  function reportCopy(kind: string, storeField: string, id: string): void {
    clipCopyField(id, storeField)
      .then((result) => {
        toast(copyToastText(kind, result.clearsAt, Date.now()), { variant: "success" });
      })
      .catch((error) => toast(describeVaultError(String(error)), { variant: "error" }));
  }

  const id = draft?.id ?? null;
  const tagSuggestions = tagCounts(entries).map((tag) => tag.tag);

  return (
    <>
      <Dialog
        open={open}
        onOpenChange={(next) => {
          if (!next) attemptClose();
        }}
      >
        <DialogContent className="max-w-xl">
          <DialogHeader>
            <DialogTitle>{id === null ? "New entry" : "Edit entry"}</DialogTitle>
            <DialogDescription>Secrets are sealed into the vault when you save.</DialogDescription>
          </DialogHeader>
          {loading || draft === null ? (
            <div className="flex justify-center py-10">
              <Spinner />
            </div>
          ) : (
            <EntryEditorFields
              draft={draft}
              patch={patch}
              id={id}
              tagSuggestions={tagSuggestions}
              revealKey={revealKey}
              onPasswordGenerated={bumpRevealKey}
              reportCopy={reportCopy}
            />
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={attemptClose}>
              Cancel
            </Button>
            <Button type="button" disabled={saving || !canSave} onClick={() => void save()}>
              {saving ? <Spinner /> : null}
              {id === null ? "Create" : "Save"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <AlertDialog open={discardOpen} onOpenChange={setDiscardOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard changes?</AlertDialogTitle>
            <AlertDialogDescription>Your edits to this entry will be lost.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                setDiscardOpen(false);
                onClose();
              }}
            >
              Discard
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
