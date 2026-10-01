import { useEffect, useRef, useState } from "react";

import { toast } from "@/components/ui/toast";
import { describeVaultError } from "@/modules/vault/errors";
import { vaultEntryGet, vaultEntryReveal, vaultEntryUpsert } from "@/modules/vault/ipc";
import { visibleEntries } from "@/modules/vault/list/derive";
import { useVaultStore, type EditorRequest } from "@/modules/vault/store";
import {
  customFieldsReady,
  draftFromCreate,
  draftFromDetail,
  isDirty,
  toDraftPayload,
  type EditorDraft,
} from "./draft";

// The entry editor's draft lifecycle, kept out of the dialog so the dialog is
// only markup. `request === null` means closed. On open it loads the entry (and,
// when it has one, the stored TOTP URI on demand), builds a draft whose secrets
// are marked "not loaded", and saves through `vault_entry_upsert`.

export function useEntryDraft(request: EditorRequest | null, onClose: () => void) {
  const [draft, setDraft] = useState<EditorDraft | null>(null);
  const [initial, setInitial] = useState<EditorDraft | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [discardOpen, setDiscardOpen] = useState(false);
  // Bumped when the generator fills the password, so `SecretField` hides it again.
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

  function bumpRevealKey(): void {
    setRevealKey((key) => key + 1);
  }

  function attemptClose(): void {
    if (draft && initial && isDirty(draft, initial)) {
      setDiscardOpen(true);
      return;
    }
    closeRef.current();
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

  return {
    open,
    draft,
    loading,
    saving,
    discardOpen,
    setDiscardOpen,
    revealKey,
    bumpRevealKey,
    canSave: draft !== null && customFieldsReady(draft.customFields),
    patch,
    attemptClose,
    save,
  };
}
