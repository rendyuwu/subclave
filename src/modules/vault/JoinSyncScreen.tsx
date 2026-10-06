import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { DEFAULT_SYNC_CONFIG, type SyncConfig } from "@/modules/sync/types";
import { SyncBehaviour } from "@/settings/components/SyncBehaviour";
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

  const { submit, busy, error, fresh, canSubmit, blockers } = useJoinSync({
    config,
    credentials,
    passphrase,
    password: masterPassword.password,
    masterPasswordProblems: masterPassword.problems,
  });

  // The whole screen scrolls, not the form, so the wheel works anywhere on it.
  // `m-auto` centres a short form and collapses to 0 on a tall one, which keeps
  // its top reachable. `max-w-3xl` is the Settings window's content width, so
  // the rows shared with Settings > Sync wrap the same in both.
  return (
    <div className="flex h-full flex-1 overflow-y-auto p-6">
      <form onSubmit={submit} className="m-auto flex w-full max-w-3xl flex-col gap-4">
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

        <SyncBehaviour config={config} onChange={setConfig} />

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

        <div className="flex flex-col gap-2">
          <Button
            type="submit"
            disabled={!canSubmit}
            aria-describedby={blockers.length > 0 ? "join-blockers" : undefined}
          >
            {busy ? <Spinner /> : null}
            Join vault
          </Button>
          {blockers.length > 0 ? (
            <ul id="join-blockers" className="text-muted-foreground flex flex-col gap-0.5 text-xs">
              {blockers.map((blocker) => (
                <li key={blocker}>{blocker}</li>
              ))}
            </ul>
          ) : null}
        </div>
      </form>
    </div>
  );
}
