import { save } from "@tauri-apps/plugin-dialog";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Spinner } from "@/components/ui/spinner";
import { describeVaultError } from "@/modules/vault/errors";

import { backupExport, backupImportApply, backupImportPreview, type BackupPreview } from "./ipc";

/** Import already has the file: the native picker runs before this dialog, so
 *  the passphrase is asked for a file the user has already chosen. */
export type BackupMode = { kind: "export" } | { kind: "import"; path: string };

const FILE_FILTER = { name: "Subclave backup", extensions: ["subclave-backup"] };

/** Today's local date as `YYYY-MM-DD`, for the default file name. */
function localDate(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/**
 * Write an encrypted `.subclave-backup`, or open one and merge it into the
 * vault. Every step runs in Rust; this dialog holds the passphrase only while
 * it is open and shows counts, never an entry.
 */
export function BackupDialog({ mode, onClose }: { mode: BackupMode | null; onClose: () => void }) {
  const [passphrase, setPassphrase] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [preview, setPreview] = useState<BackupPreview | null>(null);
  const [done, setDone] = useState<string | null>(null);
  // Reset on each opening, during render so no frame shows the last opening's
  // passphrase or result. The caller hands a new `mode` object per opening.
  // `shown` outlives the close so the content does not change while it fades.
  const [shown, setShown] = useState<BackupMode>({ kind: "export" });
  if (mode !== null && mode !== shown) {
    setShown(mode);
    setPassphrase("");
    setConfirm("");
    setBusy(false);
    setError(null);
    setPreview(null);
    setDone(null);
  }

  const isExport = shown.kind === "export";
  const mismatch = isExport && confirm.length > 0 && passphrase !== confirm;
  const canSubmit =
    !busy &&
    done === null &&
    (preview !== null ||
      (passphrase.length > 0 && (!isExport || (confirm.length > 0 && !mismatch))));

  const run = async () => {
    setError(null);
    setBusy(true);
    try {
      if (shown.kind === "export") {
        const target = await save({
          defaultPath: `subclave-${localDate()}.subclave-backup`,
          filters: [FILE_FILTER],
        });
        if (!target) return;
        await backupExport(target, passphrase);
        setDone(`Backup saved to ${target}.`);
      } else if (preview === null) {
        setPreview(await backupImportPreview(shown.path, passphrase));
      } else {
        const applied = await backupImportApply(preview.handle);
        setDone(
          `Added ${applied.added} ${applied.added === 1 ? "entry" : "entries"}, updated ${applied.updated}.`,
        );
      }
    } catch (e) {
      setError(describeVaultError(String(e)));
    } finally {
      setBusy(false);
    }
  };

  const onEnter = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && canSubmit) void run();
  };

  return (
    <Dialog
      open={mode !== null}
      // Every close route (Escape, outside click, the X, Cancel) lands here,
      // so this is the one place that can refuse a close while a write runs.
      onOpenChange={(next) => {
        if (!next && !busy) onClose();
      }}
    >
      <DialogContent className="sm:max-w-md" showCloseButton={!busy}>
        <DialogHeader>
          <DialogTitle>{isExport ? "Export backup" : "Import backup"}</DialogTitle>
          <DialogDescription>
            {isExport
              ? "The file is encrypted with this passphrase (Argon2id, AES-256-GCM). Without it the backup cannot be opened, and there is no recovery. A weak passphrase is refused."
              : "Each entry is merged with the one in the vault: the newer copy stays current and the other goes into its history. Nothing is deleted. An entry deleted here after the backup was written stays deleted, unless that delete is more than 90 days old."}
          </DialogDescription>
        </DialogHeader>

        {shown.kind === "import" ? (
          <p className="text-muted-foreground truncate font-mono text-[10.5px]" title={shown.path}>
            {shown.path}
          </p>
        ) : null}

        <div className="flex flex-col gap-3">
          <label className="flex flex-col gap-1.5">
            <span className="text-muted-foreground text-[11px] font-medium tracking-tight">
              Passphrase
            </span>
            <Input
              type="password"
              autoFocus
              value={passphrase}
              disabled={busy || preview !== null || done !== null}
              onChange={(e) => setPassphrase(e.target.value)}
              onKeyDown={onEnter}
              className="h-8 font-mono text-[12px]"
            />
          </label>

          {isExport ? (
            <label className="flex flex-col gap-1.5">
              <span className="text-muted-foreground text-[11px] font-medium tracking-tight">
                Confirm passphrase
              </span>
              <Input
                type="password"
                value={confirm}
                disabled={busy || done !== null}
                onChange={(e) => setConfirm(e.target.value)}
                onKeyDown={onEnter}
                className="h-8 font-mono text-[12px]"
              />
              {mismatch ? (
                <span className="text-destructive text-[10.5px]">
                  The two passphrases do not match.
                </span>
              ) : null}
            </label>
          ) : null}

          {preview !== null && done === null ? (
            <p className="text-[12px]">
              In this backup: {preview.added} new, {preview.newer} newer, {preview.older} older,{" "}
              {preview.same} identical.
            </p>
          ) : null}
          {error ? <span className="text-destructive text-[11px]">{error}</span> : null}
          {done ? <span className="text-[12px]">{done}</span> : null}
        </div>

        <DialogFooter>
          <DialogClose asChild>
            <Button variant="outline" size="sm" disabled={busy}>
              {done ? "Close" : "Cancel"}
            </Button>
          </DialogClose>
          {done === null ? (
            <Button size="sm" disabled={!canSubmit} onClick={() => void run()} className="gap-1.5">
              {busy ? <Spinner className="size-3" /> : null}
              {isExport ? "Export" : preview === null ? "Open" : "Import"}
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
