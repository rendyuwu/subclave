import { useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { describeVaultError } from "./errors";
import { MasterPasswordFields, useMasterPassword } from "./MasterPasswordFields";
import { useVaultStore } from "./store";

/** First-run create: master password, confirmation, strength and the no-recovery acknowledgement. */
export function CreateVaultScreen() {
  const create = useVaultStore((s) => s.create);
  const masterPassword = useMasterPassword();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const canSubmit = masterPassword.ready && !busy;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      // `vault_create` leaves the vault unlocked; the store refreshes straight
      // into the workspace, no second unlock.
      await create(masterPassword.password);
    } catch (err) {
      setError(describeVaultError(String(err)));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full flex-1 items-center justify-center p-6">
      <form onSubmit={submit} className="flex w-full max-w-sm flex-col gap-4">
        <div className="flex flex-col gap-1">
          <h1 className="text-lg font-semibold">Create a vault</h1>
          <p className="text-muted-foreground text-xs">
            Choose a master password. It never leaves this device.
          </p>
        </div>

        <MasterPasswordFields draft={masterPassword} idPrefix="create" error={error} autoFocus />

        <Button type="submit" disabled={!canSubmit}>
          {busy ? <Spinner /> : null}
          Create vault
        </Button>
      </form>
    </div>
  );
}
