import { useState, type FormEvent } from "react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Spinner } from "@/components/ui/spinner";
import { StrengthMeter } from "./editor/StrengthMeter";
import { describeVaultError } from "./errors";
import { PasswordField } from "./PasswordField";
import { useVaultStore } from "./store";

const MIN_PASSWORD_LENGTH = 8;

/** First-run create: master password, confirmation, strength and the no-recovery acknowledgement. */
export function CreateVaultScreen() {
  const create = useVaultStore((s) => s.create);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [acknowledged, setAcknowledged] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const longEnough = [...password].length >= MIN_PASSWORD_LENGTH;
  const canSubmit = longEnough && password === confirm && acknowledged && !busy;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      // `vault_create` leaves the vault unlocked; the store refreshes straight
      // into the workspace, no second unlock.
      await create(password);
    } catch (err) {
      setError(describeVaultError(String(err)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full items-center justify-center p-6">
      <form onSubmit={submit} className="flex w-full max-w-sm flex-col gap-4">
        <div className="flex flex-col gap-1">
          <h1 className="text-lg font-semibold">Create a vault</h1>
          <p className="text-muted-foreground text-xs">
            Choose a master password. It never leaves this device.
          </p>
        </div>

        <PasswordField
          id="create-password"
          label="Master password"
          value={password}
          onChange={setPassword}
          autoComplete="new-password"
          autoFocus
        />
        <div className="flex flex-col gap-1.5">
          <PasswordField
            id="create-confirm"
            label="Confirm master password"
            value={confirm}
            onChange={setConfirm}
            autoComplete="new-password"
          />
          <StrengthMeter value={password} />
        </div>

        <div className="flex items-start gap-2">
          <Checkbox
            id="create-acknowledge"
            checked={acknowledged}
            onCheckedChange={(checked) => setAcknowledged(checked === true)}
            className="mt-0.5"
          />
          <label
            htmlFor="create-acknowledge"
            className="text-muted-foreground cursor-pointer text-xs leading-relaxed"
          >
            I understand there is no recovery if I forget this password.
          </label>
        </div>

        {error ? (
          <p role="alert" className="text-destructive text-xs">
            {error}
          </p>
        ) : null}

        <Button type="submit" disabled={!canSubmit}>
          {busy ? <Spinner /> : null}
          Create vault
        </Button>
      </form>
    </div>
  );
}
