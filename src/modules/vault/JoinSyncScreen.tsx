import { emit } from "@tauri-apps/api/event";
import { ChevronDown } from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Spinner } from "@/components/ui/spinner";
import { createFileKeyValueStore } from "@/lib/fileKeyValueStore";
import { tauriStoreFileIo } from "@/lib/storeRecovery";
import { SyncField } from "@/settings/components/SyncField";
import { syncJoin } from "@/modules/sync/ipc";
import { createSyncSettingsStore } from "@/modules/sync/store";
import {
  configSubset,
  connectionFieldsReady,
  DEFAULT_SYNC_CONFIG,
  SYNC_CONFIG_EVENT,
  SYNC_PROVIDERS,
  SYNC_STORE_PATH,
  type SyncConfig,
} from "@/modules/sync/types";
import { StrengthMeter } from "./editor/StrengthMeter";
import { describeVaultError } from "./errors";
import { PasswordField } from "./PasswordField";
import { useVaultStore } from "./store";

const MIN_PASSWORD_LENGTH = 8;

/**
 * First-run join: open a vault another device already syncs. The storage and
 * the sync passphrase identify the remote, the master password protects the
 * local copy, and `sync_join` writes no local file until the pull has landed.
 *
 * A remote with no keyfile answers `fresh`: nothing was written and the only
 * way forward is to create a vault here, which is why that answer offers it.
 *
 * A LANDED JOIN WRITES THE CONFIGURATION, switched on. The scheduler reads it
 * on the unlock that follows, and an off configuration makes it call
 * `sync_disable`, which clears the credentials and root key the join just
 * stored: the vault would keep its entries and silently stop syncing.
 */
export function JoinSyncScreen({ onCreateInstead }: { onCreateInstead: () => void }) {
  const refreshStatus = useVaultStore((s) => s.refreshStatus);
  const refresh = useVaultStore((s) => s.refresh);
  const settings = useMemo(
    () => createSyncSettingsStore(createFileKeyValueStore(SYNC_STORE_PATH, tauriStoreFileIo)),
    [],
  );

  const [config, setConfig] = useState<SyncConfig>(DEFAULT_SYNC_CONFIG);
  const [accessKeyId, setAccessKeyId] = useState("");
  const [secretAccessKey, setSecretAccessKey] = useState("");
  const [username, setUsername] = useState("");
  const [webdavPassword, setWebdavPassword] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [acknowledged, setAcknowledged] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fresh, setFresh] = useState(false);

  const webdav = config.provider === "webdav";
  const providerLabel =
    SYNC_PROVIDERS.find((p) => p.id === config.provider)?.label ?? config.provider;
  const longEnough = [...password].length >= MIN_PASSWORD_LENGTH;
  const credentialsReady = webdav
    ? username.length > 0 && webdavPassword.length > 0
    : accessKeyId.length > 0 && secretAccessKey.length > 0;
  const canSubmit =
    longEnough &&
    password === confirm &&
    acknowledged &&
    connectionFieldsReady(config, credentialsReady) &&
    passphrase.length > 0 &&
    !busy;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    setFresh(false);
    try {
      const result = await syncJoin({
        masterPassword: password,
        config: configSubset(config),
        // Only the selected provider reads its own pair out of this; the other
        // pair is ignored, so neither provider can be handed the other's secret.
        credentials: { accessKeyId, secretAccessKey, username, password: webdavPassword },
        passphrase,
      });
      if (result.remote === "fresh") {
        setFresh(true);
        return;
      }
      // Before the status refresh below, which is what makes the scheduler run
      // its unlock path: that path reads this file, and a file still saying
      // "off" would have it disable the session the join just opened.
      await settings.writeConfig({ ...config, enabled: true });
      await emit(SYNC_CONFIG_EVENT);
      // `sync_join` installs the pulled vault already unlocked, so the store
      // refreshes straight into the workspace, no second unlock.
      await refreshStatus();
      await refresh();
    } catch (err) {
      setError(describeVaultError(String(err)));
    } finally {
      setBusy(false);
    }
  };

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

        <div className="border-border/60 bg-card flex items-start justify-between gap-4 rounded-lg border px-3 py-2.5">
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="text-[12.5px] font-medium">Provider</span>
            <span className="text-muted-foreground text-[10.5px] leading-relaxed">
              S3-compatible storage, or a WebDAV server such as Nextcloud or ownCloud. This must be
              the same kind of storage the device that already syncs this vault uses.
            </span>
          </div>
          <div className="flex shrink-0 items-center">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" className="h-9 justify-between gap-2 px-2.5 text-[12px]">
                  <span>{providerLabel}</span>
                  <ChevronDown size={12} strokeWidth={2} className="opacity-70" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="min-w-[200px]">
                {SYNC_PROVIDERS.map((p) => (
                  <DropdownMenuItem
                    key={p.id}
                    onSelect={() => setConfig({ ...config, provider: p.id })}
                    className="text-[12px]"
                  >
                    {p.label}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>

        <SyncField
          id="join-endpoint"
          label="Endpoint"
          description={
            webdav
              ? "The full URL of the collection the other device stored its files under, not the server's home page. There is no default: this is the one field that decides whose servers your records go to."
              : "The full URL of the storage service. There is no default: this is the one field that decides whose servers your records go to."
          }
          value={config.endpoint}
          onChange={(e) => setConfig({ ...config, endpoint: e.target.value })}
        />
        {webdav && config.endpoint.trimStart().toLowerCase().startsWith("http://") ? (
          // Gated on the PROVIDER as well as the scheme: a WebDAV request
          // carries the password itself, where an S3 request carries a
          // signature computed from the secret and never the secret. Allowed
          // rather than refused, because a server on a home network with no
          // certificate is a real arrangement and which networks are worth
          // trusting is the user's call.
          <div
            role="status"
            className="border-border/60 bg-card flex flex-col gap-1 rounded-lg border px-3 py-2.5"
          >
            <span className="text-[11.5px] font-semibold">
              Note: this endpoint is not encrypted
            </span>
            <span className="text-muted-foreground text-[10.5px] leading-relaxed">
              The password below is sent with every single request, in a form that can be read back,
              and anything between this device and that server can read it: other machines on the
              same network, and whatever the traffic passes through on the way. Your records
              themselves stay encrypted with the passphrase either way. Use an https:// address
              instead if the server offers one.
            </span>
          </div>
        ) : null}

        {webdav ? null : (
          <>
            <SyncField
              id="join-region"
              label="Region"
              value={config.region}
              onChange={(e) => setConfig({ ...config, region: e.target.value })}
            />
            <SyncField
              id="join-bucket"
              label="Bucket"
              value={config.bucket}
              onChange={(e) => setConfig({ ...config, bucket: e.target.value })}
            />
          </>
        )}

        <SyncField
          id="join-prefix"
          label="Prefix"
          description="Where in that storage the other device put this inventory. May be left empty, which puts it at the root."
          value={config.prefix}
          onChange={(e) => setConfig({ ...config, prefix: e.target.value })}
        />

        {webdav ? (
          <>
            <SyncField
              id="join-webdav-username"
              label="Username"
              description="The account on the WebDAV server. Stored only inside this vault, encrypted with the master password below."
              autoComplete="off"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
            />
            <SyncField
              id="join-webdav-password"
              label="Password"
              description="Stored only inside this vault, encrypted with the master password below. If your server offers app passwords, one of those is worth more here than your account password: it can be revoked on its own."
              type="password"
              autoComplete="off"
              value={webdavPassword}
              onChange={(e) => setWebdavPassword(e.target.value)}
            />
          </>
        ) : (
          <>
            <SyncField
              id="join-access-key-id"
              label="Access key ID"
              description="Stored only inside this vault, encrypted with the master password below."
              autoComplete="off"
              value={accessKeyId}
              onChange={(e) => setAccessKeyId(e.target.value)}
            />
            <SyncField
              id="join-secret-access-key"
              label="Secret access key"
              description="Stored only inside this vault, encrypted with the master password below."
              type="password"
              autoComplete="off"
              value={secretAccessKey}
              onChange={(e) => setSecretAccessKey(e.target.value)}
            />
          </>
        )}

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

        <div className="flex flex-col gap-1">
          <PasswordField
            id="join-password"
            label="Master password"
            value={password}
            onChange={setPassword}
            autoComplete="new-password"
          />
          <span className="text-muted-foreground text-[10.5px] leading-relaxed">
            Protects the copy of this vault on this device. It never leaves it.
          </span>
        </div>
        <div className="flex flex-col gap-1.5">
          <PasswordField
            id="join-confirm"
            label="Confirm master password"
            value={confirm}
            onChange={setConfirm}
            autoComplete="new-password"
          />
          <StrengthMeter value={password} />
        </div>

        <div className="flex items-start gap-2">
          <Checkbox
            id="join-acknowledge"
            checked={acknowledged}
            onCheckedChange={(checked) => setAcknowledged(checked === true)}
            className="mt-0.5"
          />
          <label
            htmlFor="join-acknowledge"
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
