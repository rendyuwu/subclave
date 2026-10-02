// The Browser tab: the two native-messaging switches, the browsers detected on
// this machine, and the browsers already paired with the vault.
//
// The switches read from the shared preferences store and write through
// `setBrowserFamily`, which persists the preference and then asks Rust to write
// (or remove) the manifests. The detected-browser and paired-client lists are
// per-command fetches, refreshed after each write.

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { toast } from "@/components/ui/toast";
import { describeVaultError } from "@/modules/vault/errors";
import {
  browserClientRename,
  browserClientRevoke,
  browserClientsList,
  browserIntegrationStatus,
  type BrowserClientSummary,
  type BrowserFamily,
  type BrowserIntegrationStatus,
} from "@/modules/browser/ipc";
import { setBrowserFamily } from "@/modules/settings/mutations";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { useCallback, useEffect, useState } from "react";
import { Label } from "../components/Label";
import { SectionHeader } from "../components/SectionHeader";
import { SettingRow } from "../components/SettingRow";

const FAMILY_ROWS: { family: BrowserFamily; title: string; description: string }[] = [
  {
    family: "chromium",
    title: "Chromium browsers",
    description: "Chrome, Chromium, Edge, Brave and Vivaldi, each on its own profile directory.",
  },
  {
    family: "firefox",
    title: "Firefox",
    description: "Mozilla Firefox, including a Snap or Flatpak build whose profile is detected.",
  },
];

/** One paired browser: name, family, last seen, and an inline rename. */
function ClientRow({
  client,
  onRename,
  onRevoke,
}: {
  client: BrowserClientSummary;
  onRename: (id: string, name: string) => void;
  onRevoke: (client: BrowserClientSummary) => void;
}) {
  const [draft, setDraft] = useState(client.name);

  useEffect(() => setDraft(client.name), [client.name]);

  const commit = () => {
    const next = draft.trim();
    if (next.length === 0) {
      setDraft(client.name);
      return;
    }
    if (next !== client.name) onRename(client.id, next);
  };

  const lastSeen =
    client.lastSeenAt === null ? "Never used" : new Date(client.lastSeenAt).toLocaleString();

  return (
    <div
      role="listitem"
      className="border-border/60 bg-card flex items-center justify-between gap-3 rounded-lg border px-3 py-2.5"
    >
      <div className="flex min-w-0 flex-col gap-0.5">
        <Input
          value={draft}
          maxLength={64}
          aria-label={`Name for the paired ${client.family} browser`}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
          }}
          className="h-7 max-w-56 text-[12px]"
        />
        <span className="text-muted-foreground text-[10.5px]">
          {client.family} · {lastSeen}
        </span>
      </div>
      <Button variant="outline" size="sm" onClick={() => onRevoke(client)}>
        Revoke
      </Button>
    </div>
  );
}

export function BrowserSection() {
  const browser = usePreferencesStore((s) => s.browser);
  const [status, setStatus] = useState<BrowserIntegrationStatus | null>(null);
  const [clients, setClients] = useState<BrowserClientSummary[]>([]);
  const [busyFamily, setBusyFamily] = useState<BrowserFamily | null>(null);
  const [revokeTarget, setRevokeTarget] = useState<BrowserClientSummary | null>(null);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await browserIntegrationStatus());
    } catch (e) {
      toast(describeVaultError(String(e)), { variant: "error" });
    }
  }, []);

  const refreshClients = useCallback(async () => {
    try {
      setClients(await browserClientsList());
    } catch (e) {
      toast(describeVaultError(String(e)), { variant: "error" });
    }
  }, []);

  useEffect(() => {
    void refreshStatus();
    void refreshClients();
  }, [refreshStatus, refreshClients]);

  const onToggle = async (family: BrowserFamily, enabled: boolean) => {
    setBusyFamily(family);
    try {
      await setBrowserFamily(family, enabled);
    } catch (e) {
      toast(describeVaultError(String(e)), { variant: "error" });
    } finally {
      // Re-read even on failure: `enabled` comes from Rust, which reads the
      // same preference file, so this keeps the row and the switch in step.
      await refreshStatus();
      setBusyFamily(null);
    }
  };

  const onRename = async (id: string, name: string) => {
    try {
      await browserClientRename(id, name);
      await refreshClients();
    } catch (e) {
      toast(describeVaultError(String(e)), { variant: "error" });
    }
  };

  const onRevoke = async () => {
    const target = revokeTarget;
    setRevokeTarget(null);
    if (!target) return;
    try {
      await browserClientRevoke(target.id);
      await refreshClients();
    } catch (e) {
      toast(describeVaultError(String(e)), { variant: "error" });
    }
  };

  // The refusal is a server-level fact; Rust repeats it on every family row, so
  // the first non-null one is the whole message.
  const listenError = status?.find((row) => row.listenError)?.listenError ?? null;
  const anySandboxed = status?.some((row) => row.browsers.some((b) => b.sandboxed)) ?? false;

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader
        title="Browser"
        description="Connect the Subclave extension to any Chromium browser or Firefox on this machine."
      />

      {listenError ? (
        <div
          role="alert"
          className="border-destructive/40 bg-destructive/5 flex flex-col gap-1 rounded-lg border px-3 py-2.5"
        >
          <span className="text-destructive text-[11.5px] font-semibold">
            The browser channel is not listening
          </span>
          <span className="text-[10.5px] leading-relaxed break-words">{listenError}</span>
        </div>
      ) : null}

      <div className="flex flex-col gap-2">
        <Label>Browsers</Label>
        <div className="flex flex-col gap-4">
          {FAMILY_ROWS.map(({ family, title, description }) => {
            const row = status?.find((s) => s.family === family);
            const busy = busyFamily === family;
            return (
              <div key={family} className="flex flex-col gap-2">
                <SettingRow title={title} description={description}>
                  <Switch
                    checked={browser[family]}
                    disabled={busy}
                    aria-label={title}
                    onCheckedChange={(v) => void onToggle(family, v)}
                  />
                </SettingRow>
                {row && row.browsers.length > 0 ? (
                  <div className="flex flex-col gap-1.5 pl-3">
                    {row.browsers.map((b) => (
                      <div key={b.browser} className="flex flex-col gap-0.5">
                        <div className="flex items-center gap-2">
                          <span className="text-[11.5px] font-medium">{b.browser}</span>
                          {b.installed ? <Badge variant="secondary">installed</Badge> : null}
                          {b.sandboxed ? <Badge variant="outline">sandboxed</Badge> : null}
                        </div>
                        <span className="text-muted-foreground font-mono text-[10px] break-all">
                          {b.manifestPath}
                        </span>
                      </div>
                    ))}
                  </div>
                ) : row ? (
                  <span className="text-muted-foreground pl-3 text-[10.5px]">
                    No {title.toLowerCase()} profile found.
                  </span>
                ) : null}
              </div>
            );
          })}
        </div>
      </div>

      {anySandboxed ? (
        <div
          role="status"
          className="border-border/60 bg-card flex flex-col gap-1 rounded-lg border px-3 py-2.5"
        >
          <span className="text-[11.5px] font-semibold">Sandboxed browsers</span>
          <span className="text-muted-foreground text-[10.5px] leading-relaxed">
            A Snap or Flatpak browser reads its native messaging manifest from its own profile
            directory. The manifest is written there too, but whether the sandbox lets the browser
            reach Subclave depends on the portal.
          </span>
        </div>
      ) : null}

      <div className="flex flex-col gap-2">
        <Label>Paired browsers</Label>
        {clients.length === 0 ? (
          <span className="text-muted-foreground text-[10.5px]">
            No browsers are paired yet. Open the extension and choose Pair with Subclave.
          </span>
        ) : (
          <div role="list" className="flex flex-col gap-2">
            {clients.map((client) => (
              <ClientRow
                key={client.id}
                client={client}
                onRename={(id, name) => void onRename(id, name)}
                onRevoke={setRevokeTarget}
              />
            ))}
          </div>
        )}
      </div>

      <AlertDialog
        open={revokeTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRevokeTarget(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Revoke {revokeTarget?.name ?? "this browser"}?</AlertDialogTitle>
            <AlertDialogDescription>
              The browser loses access immediately and must pair again. Other paired browsers keep
              their connections.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={() => void onRevoke()}>
              Revoke
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
