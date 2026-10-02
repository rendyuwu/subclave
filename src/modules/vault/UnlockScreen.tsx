import { useRef, useState, type FormEvent } from "react";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { TriangleAlert } from "lucide-react";
import { describeVaultError } from "./errors";
import { PasswordField } from "./PasswordField";
import { useVaultStore } from "./store";

/**
 * The locked screen. A wrong password and a corrupt file are deliberately the
 * same sentence, and the restore offer only appears after a successful unlock
 * that came from the backup file.
 */
export function UnlockScreen() {
  const unlock = useVaultStore((s) => s.unlock);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [capsLock, setCapsLock] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy || password.length === 0) return;
    setBusy(true);
    setError(null);
    try {
      await unlock(password);
    } catch (err) {
      setError(describeVaultError(String(err)));
      // Focus and select so a retry is one keystroke, not another click.
      inputRef.current?.focus();
      inputRef.current?.select();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full flex-1 items-center justify-center p-6">
      <form onSubmit={submit} className="flex w-full max-w-sm flex-col gap-4">
        <div className="flex flex-col items-center gap-2 text-center">
          <img src="/icon.png" alt="" aria-hidden draggable={false} className="size-10" />
          <h1 className="text-lg font-semibold">Unlock Subclave</h1>
          <p className="text-muted-foreground text-xs">
            Enter your master password to open the vault.
          </p>
        </div>

        <PasswordField
          id="unlock-password"
          label="Master password"
          value={password}
          onChange={setPassword}
          autoComplete="current-password"
          autoFocus
          inputRef={inputRef}
          onKeyDown={(e) => setCapsLock(e.getModifierState("CapsLock"))}
          onKeyUp={(e) => setCapsLock(e.getModifierState("CapsLock"))}
        />

        {capsLock ? (
          <p className="text-muted-foreground flex items-center gap-1.5 text-xs">
            <TriangleAlert size={13} strokeWidth={2} />
            Caps Lock is on.
          </p>
        ) : null}

        {error ? (
          <p role="alert" className="text-destructive text-xs">
            {error}
          </p>
        ) : null}

        <Button type="submit" disabled={busy || password.length === 0}>
          {busy ? <Spinner /> : null}
          Unlock
        </Button>
      </form>
    </div>
  );
}
