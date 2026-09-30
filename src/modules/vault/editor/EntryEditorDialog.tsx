import { useEffect, useRef, useState } from "react";

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
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { Textarea } from "@/components/ui/textarea";
import { toast } from "@/components/ui/toast";
import { copyToastText } from "@/modules/vault/copy";
import { describeVaultError } from "@/modules/vault/errors";
import {
  clipCopyField,
  vaultEntryGet,
  vaultEntryReveal,
  vaultEntryUpsert,
} from "@/modules/vault/ipc";
import { tagCounts, visibleEntries } from "@/modules/vault/list/derive";
import { useVaultStore, type EditorRequest } from "@/modules/vault/store";
import { AppearancePicker } from "@/modules/groups/AppearancePicker";
import { GroupPicker } from "@/modules/groups/GroupPicker";
import { CustomFieldsEditor } from "./CustomFieldsEditor";
import { Field } from "./FormControls";
import { GeneratorPopover } from "./GeneratorPopover";
import { SecretField } from "./SecretField";
import { TagsInput } from "./TagsInput";
import { TotpField } from "./TotpField";
import { UrlListField } from "./UrlListField";
import {
  customFieldsReady,
  dateInputToEpoch,
  draftFromCreate,
  draftFromDetail,
  epochToDateInput,
  isDirty,
  toDraftPayload,
  type EditorDraft,
} from "./draft";

// The one entry editor. `request === null` means closed. On open it loads the
// entry (and, when it has one, the stored TOTP URI on demand), builds a draft
// whose secrets are marked "not loaded", and saves through `vault_entry_upsert`.

export function EntryEditorDialog({
  request,
  onClose,
}: {
  request: EditorRequest | null;
  onClose: () => void;
}) {
  const entries = useVaultStore((state) => state.entries);
  const [draft, setDraft] = useState<EditorDraft | null>(null);
  const [initial, setInitial] = useState<EditorDraft | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [discardOpen, setDiscardOpen] = useState(false);
  const [revealKey, setRevealKey] = useState(0);

  const closeRef = useRef(onClose);
  useEffect(() => {
    closeRef.current = onClose;
  });

  const open = request !== null;
  const entryId = request?.entryId ?? null;
  const groupId = request?.groupId ?? "";

  useEffect(() => {
    if (!open) {
      setDraft(null);
      setInitial(null);
      setDiscardOpen(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void (async () => {
      let base: EditorDraft;
      if (entryId !== null) {
        try {
          const detail = await vaultEntryGet(entryId);
          base = draftFromDetail(detail, groupId);
          if (detail.hasTotp) {
            try {
              base.totp = await vaultEntryReveal(detail.id, "totp");
            } catch {
              // The URI stays empty; the save omits it and keeps the stored one.
            }
          }
        } catch (error) {
          toast(describeVaultError(String(error)), { variant: "error" });
          closeRef.current();
          return;
        }
      } else {
        base = draftFromCreate(groupId);
      }
      if (cancelled) return;
      setDraft(base);
      setInitial(base);
      setRevealKey(0);
      setLoading(false);
    })();
    return () => {
      cancelled = true;
    };
  }, [open, entryId, groupId]);

  function patch(part: Partial<EditorDraft>): void {
    setDraft((current) => (current ? { ...current, ...part } : current));
  }

  function attemptClose(): void {
    if (draft && initial && isDirty(draft, initial)) {
      setDiscardOpen(true);
      return;
    }
    closeRef.current();
  }

  function reportCopy(kind: string, storeField: string, id: string): void {
    clipCopyField(id, storeField)
      .then((result) => {
        toast(copyToastText(kind, result.clearsAt, Date.now()), { variant: "success" });
      })
      .catch((error) => toast(describeVaultError(String(error)), { variant: "error" }));
  }

  async function save(): Promise<void> {
    if (!draft || !customFieldsReady(draft.customFields)) return;
    setSaving(true);
    try {
      const saved = await vaultEntryUpsert(toDraftPayload(draft));
      const store = useVaultStore.getState();
      await store.refresh();
      const current = useVaultStore.getState();
      const visible = visibleEntries({
        entries: current.entries,
        scope: current.scope,
        tagFilter: current.tagFilter,
        searchIds: current.searchIds === null ? null : new Set(current.searchIds),
        now: Date.now(),
      });
      // A create into another group (or a move) must not land off screen.
      if (!visible.some((entry) => entry.id === saved.id)) current.selectScope(saved.groupId);
      useVaultStore.getState().selectEntry(saved.id);
      closeRef.current();
    } catch (error) {
      toast(describeVaultError(String(error)), { variant: "error" });
    } finally {
      setSaving(false);
    }
  }

  const id = draft?.id ?? null;
  const tagSuggestions = tagCounts(entries).map((tag) => tag.tag);
  const canSave = draft !== null && customFieldsReady(draft.customFields);

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
            <div className="flex max-h-[62vh] flex-col gap-3 overflow-y-auto pr-1">
              <Field label="Title">
                <Input
                  value={draft.title}
                  onChange={(e) => patch({ title: e.target.value })}
                  className="h-8"
                />
              </Field>
              <Field label="Username">
                <Input
                  value={draft.username}
                  spellCheck={false}
                  autoComplete="off"
                  onChange={(e) => patch({ username: e.target.value })}
                  className="h-8"
                />
              </Field>
              <Field label="Password">
                <SecretField
                  value={draft.password}
                  ariaLabel="Password"
                  resetKey={id ?? "new"}
                  revealKey={revealKey}
                  onChange={(password) => patch({ password, passwordTouched: true })}
                  onReveal={id === null ? undefined : () => vaultEntryReveal(id, "password")}
                  onCopy={id === null ? undefined : () => reportCopy("password", "password", id)}
                >
                  <GeneratorPopover
                    onUse={(password) => {
                      patch({ password, passwordTouched: true });
                      setRevealKey((key) => key + 1);
                    }}
                  />
                </SecretField>
              </Field>
              <TotpField
                value={draft.totp}
                onChange={(totp) => patch({ totp, totpTouched: true })}
              />
              <UrlListField urls={draft.urls} onChange={(urls) => patch({ urls })} />
              <Field label="Notes">
                <Textarea
                  value={draft.notes}
                  onChange={(e) => patch({ notes: e.target.value })}
                  className="min-h-20 text-[12px]"
                />
              </Field>
              <GroupPicker
                value={draft.groupId}
                onChange={(groupId) => patch({ groupId })}
                exclude={["trash"]}
                label="Group"
              />
              <CustomFieldsEditor
                fields={draft.customFields}
                onChange={(customFields) => patch({ customFields })}
                onReveal={
                  id === null ? undefined : (name) => vaultEntryReveal(id, `custom:${name}`)
                }
                onCopy={id === null ? undefined : (name) => reportCopy(name, `custom:${name}`, id)}
              />
              <TagsInput
                tags={draft.tags}
                onChange={(tags) => patch({ tags: [...tags] })}
                suggestions={tagSuggestions}
              />
              <AppearancePicker
                icon={draft.icon}
                color={draft.color}
                onIconChange={(icon) => patch({ icon })}
                onColorChange={(color) => patch({ color })}
              />
              <label className="flex items-center gap-2 text-[12px]">
                <Checkbox
                  checked={draft.favorite}
                  onCheckedChange={(checked) => patch({ favorite: checked === true })}
                />
                Favourite
              </label>
              <Field label="Expires (optional)">
                <Input
                  type="date"
                  value={epochToDateInput(draft.expiresAt)}
                  onChange={(e) => patch({ expiresAt: dateInputToEpoch(e.target.value) })}
                  className="h-8"
                />
              </Field>
            </div>
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
                closeRef.current();
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
