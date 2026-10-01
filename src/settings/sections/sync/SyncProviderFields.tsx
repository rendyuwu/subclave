import { ChevronDown } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SYNC_PROVIDERS, type SyncConfig } from "@/modules/sync/types";
import { SettingRow } from "../../components/SettingRow";
import { SyncField } from "../../components/SyncField";
import { KEEP_STORED, type SecretDraft } from "./form";

/**
 * The Storage block: which kind of storage this device talks to, where in it,
 * and the credentials to reach it. The plaintext-http note lives here because
 * it is a property of the endpoint typed above it.
 */
export function SyncProviderFields({
  config,
  secrets,
  setConfig,
  setSecrets,
  clearSaved,
}: {
  config: SyncConfig;
  secrets: SecretDraft;
  setConfig: (next: SyncConfig) => void;
  setSecrets: (next: SecretDraft) => void;
  /** Switching provider changes what will be written, so the saved note above
   *  it stops being true. */
  clearSaved: () => void;
}) {
  const providerLabel =
    SYNC_PROVIDERS.find((p) => p.id === config.provider)?.label ?? config.provider;
  // WebDAV differs from S3 in more than a field list: it authenticates with a
  // username and a password sent on every request, and it has no conditional
  // write to offer, so both the credential fields and the Behaviour block turn
  // on this.
  const webdav = config.provider === "webdav";

  return (
    <>
      <SettingRow
        title="Provider"
        description="Which kind of storage this device talks to: S3 and S3-compatible storage, or a WebDAV server such as Nextcloud or ownCloud. Each keeps its own credentials, so switching between them does not send one's secret to the other."
      >
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
                onSelect={() => {
                  setConfig({ ...config, provider: p.id });
                  clearSaved();
                }}
                className="text-[12px]"
              >
                {p.label}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </SettingRow>

      <SyncField
        id="sync-endpoint"
        label="Endpoint"
        description={
          webdav
            ? "The full URL of the collection your files hang off, not the server's home page. On Nextcloud and ownCloud that is the WebDAV address their own settings screen shows you. There is no default: this is the one field that decides whose servers your records go to."
            : "The full URL of the storage service. There is no default: this is the one field that decides whose servers your records go to."
        }
        value={config.endpoint}
        onChange={(e) => setConfig({ ...config, endpoint: e.target.value })}
      />
      {webdav && config.endpoint.trimStart().toLowerCase().startsWith("http://") ? (
        // Gated on the PROVIDER as well as the scheme, because the sentence is
        // only true of this one: a WebDAV request carries the password itself,
        // where an S3 request carries a signature computed from the secret and
        // never the secret. Allowed rather than refused, because a WebDAV
        // server on a home network with no certificate is a real arrangement
        // and which networks are worth trusting is the user's call.
        //
        // CASE-INSENSITIVE, because the backend's URL parser lowercases a
        // scheme and this warning is the only defence there is, so a pasted
        // uppercase spelling must not be accepted in silence.
        <div
          role="status"
          className="border-border/60 bg-card flex flex-col gap-1 rounded-lg border px-3 py-2.5"
        >
          <span className="text-[11.5px] font-semibold">Note: this endpoint is not encrypted</span>
          <span className="text-muted-foreground text-[10.5px] leading-relaxed">
            The password below is sent with every single request, in a form that can be read back,
            and anything between this device and that server can read it: other machines on the same
            network, and whatever the traffic passes through on the way. Your records themselves
            stay encrypted with the passphrase either way. Use an https:// address instead if the
            server offers one.
          </span>
        </div>
      ) : null}
      {webdav ? null : (
        <>
          <SyncField
            id="sync-region"
            label="Region"
            value={config.region}
            onChange={(e) => setConfig({ ...config, region: e.target.value })}
          />
          <SyncField
            id="sync-bucket"
            label="Bucket"
            value={config.bucket}
            onChange={(e) => setConfig({ ...config, bucket: e.target.value })}
          />
        </>
      )}
      <SyncField
        id="sync-prefix"
        label="Prefix"
        description="Where in that storage this vault's objects live. May be left empty, which puts them at the root."
        value={config.prefix}
        onChange={(e) => setConfig({ ...config, prefix: e.target.value })}
      />
      {webdav ? (
        <>
          <SyncField
            id="sync-webdav-username"
            label="Username"
            description="The account on the WebDAV server. Stored in the vault beside the password, never in the settings file. Leave it blank to keep whatever is stored: this field is write-only, so it never shows what is already there."
            autoComplete="off"
            placeholder={KEEP_STORED}
            value={secrets.webdavUsername}
            onChange={(e) => setSecrets({ ...secrets, webdavUsername: e.target.value })}
          />
          <SyncField
            id="sync-webdav-password"
            label="Password"
            description="Stored in the vault beside the username, never in the settings file. If your server offers app passwords, one of those is worth more here than your account password: it can be revoked on its own."
            type="password"
            autoComplete="off"
            placeholder={KEEP_STORED}
            value={secrets.webdavPassword}
            onChange={(e) => setSecrets({ ...secrets, webdavPassword: e.target.value })}
          />
        </>
      ) : (
        <>
          <SyncField
            id="sync-access-key-id"
            label="Access key ID"
            description="Stored in the vault, never in the settings file. Leave it blank to keep whatever is stored: this field is write-only, so it never shows what is already there."
            autoComplete="off"
            placeholder={KEEP_STORED}
            value={secrets.accessKeyId}
            onChange={(e) => setSecrets({ ...secrets, accessKeyId: e.target.value })}
          />
          <SyncField
            id="sync-secret-access-key"
            label="Secret access key"
            description="Stored in the vault, never in the settings file. Leave it blank to keep whatever is stored."
            type="password"
            autoComplete="off"
            placeholder={KEEP_STORED}
            value={secrets.secretAccessKey}
            onChange={(e) => setSecrets({ ...secrets, secretAccessKey: e.target.value })}
          />
        </>
      )}
    </>
  );
}
