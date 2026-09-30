import { Button } from "@/components/ui/button";
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
import { toast } from "@/components/ui/toast";
import { StrengthMeter } from "@/modules/vault/editor/StrengthMeter";
import { describeVaultError } from "@/modules/vault/errors";
import { genStrength, vaultChangeMaster } from "@/modules/vault/ipc";
import { useEffect, useState } from "react";
import { Label } from "../components/Label";

const MIN_LENGTH = 8;
/** Below this the meter's warning is repeated as a line the user cannot miss. */
const WEAK_SCORE = 3;

export function ChangeMasterDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [score, setScore] = useState<number | null>(null);

  // `force` lets the success path close while `busy` is still true in this
  // render's closure; every other close path refuses mid-save.
  const close = (force = false) => {
    if (busy && !force) return;
    setCurrent("");
    setNext("");
    setConfirm("");
    setError(null);
    setScore(null);
    onOpenChange(false);
  };

  // The meter owns the bar and label; this probe is only for the explicit
  // warning line below it, and is debounced the same way.
  useEffect(() => {
    if (!open || next.length === 0) {
      setScore(null);
      return;
    }
    const timer = setTimeout(() => {
      void genStrength(next)
        .then((s) => setScore(s.score))
        .catch(() => setScore(null));
    }, 200);
    return () => clearTimeout(timer);
  }, [open, next]);

  const canSubmit =
    !busy && current.length > 0 && [...next].length >= MIN_LENGTH && next === confirm;

  const submit = async () => {
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      await vaultChangeMaster(current, next);
      toast("Master password changed.", { variant: "success" });
      setBusy(false);
      close(true);
    } catch (e) {
      setError(describeVaultError(String(e)));
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => (o ? onOpenChange(true) : close())}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Change master password</DialogTitle>
          <DialogDescription>The vault is re-encrypted with the new password.</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <label className="flex flex-col gap-1.5">
            <Label>Current password</Label>
            <Input
              type="password"
              autoComplete="current-password"
              value={current}
              onChange={(e) => setCurrent(e.target.value)}
            />
          </label>
          <label className="flex flex-col gap-1.5">
            <Label>New password</Label>
            <Input
              type="password"
              autoComplete="new-password"
              value={next}
              onChange={(e) => setNext(e.target.value)}
            />
            <StrengthMeter value={next} />
            {score !== null && score < WEAK_SCORE ? (
              <span className="text-destructive text-[10.5px]">
                This password is weak. Use a longer one.
              </span>
            ) : null}
          </label>
          <label className="flex flex-col gap-1.5">
            <Label>Confirm new password</Label>
            <Input
              type="password"
              autoComplete="new-password"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
            />
          </label>
          <p className="text-muted-foreground text-[10.5px] leading-relaxed">
            Copies made before now, such as backups and OS backups, still open with the old
            password.
          </p>
          {error ? <p className="text-destructive text-[11px]">{error}</p> : null}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => close()} disabled={busy}>
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={!canSubmit}>
            {busy ? <Spinner /> : null}
            Change password
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
