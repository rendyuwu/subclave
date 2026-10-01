import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { DEFAULT_SYNC_CONFIG, type SyncConfig } from "@/modules/sync/types";
import { MasterPasswordFields, useMasterPassword } from "./MasterPasswordFields";
import { PasswordField } from "./PasswordField";
import { JoinCredentialFields } from "./join/JoinCredentialFields";
import { JoinProviderFields } from "./join/JoinProviderFields";
import { EMPTY_JOIN_CREDENTIALS, useJoinSync, type JoinCredentials } from "./join/useJoinSync";

/**
 * First-run join: open a vault another device already syncs. The storage and
 * the sync passphrase identify the remote, the master password protects the
 * local copy, and `sync_join` writes no local file until the pull has landed.
 *
 * A remote with no keyfile answers `fresh`: nothing was written and the only
 * way forward is to create a vault here, which is why that answer offers it.
 */
export function JoinSyncScreen({ onCreateInstead }: { onCreateInstead: () => void }) {
  const [config, setConfig] = useState<SyncConfig>(DEFAULT_SYNC_CONFIG);
  const [credentials, setCredentials] = useState<JoinCredentials>(EMPTY_JOIN_CREDENTIALS);
  const [passphrase, setPassphrase] = useState("");
  const masterPassword = useMasterPassword();

  const { submit, busy, error, fresh, canSubmit } = useJoinSync({
    config,
    credentials,
    passphrase,
    password: masterPassword.password,
    masterPasswordReady: masterPassword.ready,
  });

  return (
    <div className="flex h-full items-center justify-center p-6">
      <form
        onSubmit={submit}
        className="flex max-h-full w-full max-w-sm flex-col gap-4 overflow-y-auto"
      >
        <div className="flex flex-col gap-1">
          <h1 className="text-lg font-semibold">Join a synced vault</h1>
          <p className="text-muted-foreground text-xs">
            Open the vault another device already syncs, using the same storage and passphrase.
          </p>
        </div>

        <JoinProviderFields config={config} onChange={setConfig} />

        <JoinCredentialFields
          provider={config.provider}
          credentials={credentials}
          onChange={setCredentials}
        />

        <div className="flex flex-col gap-1">
          <PasswordField
            id="join-passphrase"
            label="Sync passphrase"
            value={passphrase}
            onChange={setPassphrase}
            autoComplete="off"
          />
          <span className="text-muted-foreground text-[10.5px] leading-relaxed">
            Every device that syncs this vault is given the same one. Records are encrypted with it
            before they leave this device, so the storage provider never sees them.
          </span>
        </div>

        <MasterPasswordFields
          draft={masterPassword}
          idPrefix="join"
          error={error}
          description="Protects the copy of this vault on this device. It never leaves it."
        />

        {fresh ? (
          <div role="alert" className="flex flex-col gap-2">
            <p className="text-destructive text-xs">No Subclave data at this location.</p>
            <Button type="button" variant="outline" size="sm" onClick={onCreateInstead}>
              Create a new vault instead
            </Button>
          </div>
        ) : null}

        <Button type="submit" disabled={!canSubmit}>
          {busy ? <Spinner /> : null}
          Join vault
        </Button>
      </form>
    </div>
  );
}
