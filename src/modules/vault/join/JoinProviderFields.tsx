import { ChevronDown } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SyncField } from "@/settings/components/SyncField";
import { SYNC_PROVIDERS, type SyncConfig } from "@/modules/sync/types";

/**
 * Where the join pulls from: the provider, the endpoint and the path into it,
 * plus the unencrypted-endpoint warning a WebDAV server on `http://` earns.
 */
export function JoinProviderFields({
  config,
  onChange,
}: {
  config: SyncConfig;
  onChange: (config: SyncConfig) => void;
}) {
  const webdav = config.provider === "webdav";
  const providerLabel =
    SYNC_PROVIDERS.find((p) => p.id === config.provider)?.label ?? config.provider;

  return (
    <>
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
                  onSelect={() => onChange({ ...config, provider: p.id })}
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
        onChange={(e) => onChange({ ...config, endpoint: e.target.value })}
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
            id="join-region"
            label="Region"
            value={config.region}
            onChange={(e) => onChange({ ...config, region: e.target.value })}
          />
          <SyncField
            id="join-bucket"
            label="Bucket"
            value={config.bucket}
            onChange={(e) => onChange({ ...config, bucket: e.target.value })}
          />
        </>
      )}

      <SyncField
        id="join-prefix"
        label="Prefix"
        description="Where in that storage the other device put this inventory. May be left empty, which puts it at the root."
        value={config.prefix}
        onChange={(e) => onChange({ ...config, prefix: e.target.value })}
      />
    </>
  );
}
