// The sync tab: the storage configuration this window writes, and the status
// `main` writes back.
//
// ONE EXPLICIT SAVE, unlike the rest of this window. Every other section
// persists a preference per keystroke because a preference write is one store
// `set`; this one opens a session against a remote and rewrites the vault, both
// of which are round trips worth batching behind one button.
//
// SECRETS ARE WRITE-ONLY HERE. Nothing in the app reads a stored passphrase or
// credential back out, and re-displaying one would buy nothing anyway, so every
// secret field renders empty and A BLANK FIELD MEANS "stored, unchanged":
// treating blank as a clear would wipe a working passphrase every time somebody
// opened this tab to correct a bucket name.

import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { StrengthMeter } from "@/modules/vault/editor/StrengthMeter";
import { Label } from "../components/Label";
import { SectionHeader } from "../components/SectionHeader";
import { SettingRow } from "../components/SettingRow";
import { SyncField } from "../components/SyncField";
import { SyncBehaviour } from "../components/SyncBehaviour";
import { FreshRemoteDialog } from "./FreshRemoteDialog";
import { KEEP_STORED } from "./sync/form";
import { SyncProviderFields } from "./sync/SyncProviderFields";
import { SyncStatusPanel } from "./sync/SyncStatusPanel";
import { useSyncConfigForm } from "./sync/useSyncConfigForm";

export function SyncSection() {
  const {
    config,
    setConfig,
    secrets,
    setSecrets,
    setSaved,
    status,
    busy,
    error,
    saved,
    joined,
    fresh,
    setFresh,
    dialogError,
    setDialogError,
    onSave,
    onCreateRemote,
    onToggleEnabled,
    request,
    refreshStatus,
  } = useSyncConfigForm();

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader
        title="Sync"
        description="Keep this vault's entries in storage you own, and reconcile them with your other devices."
      />

      <div className="flex flex-col gap-2">
        <Label>Sync</Label>
        <SettingRow
          title="Enable sync"
          description="Off means no network: nothing is uploaded, downloaded or listed, and no credential leaves this device. On, this device reconciles with the storage below on launch, on focus and after an edit."
        >
          <Switch
            checked={config.enabled}
            disabled={busy}
            onCheckedChange={(v) => void onToggleEnabled(v)}
            aria-label="Enable sync"
          />
        </SettingRow>
      </div>

      <div className="flex flex-col gap-2">
        <Label>Storage</Label>
        <SyncProviderFields
          config={config}
          secrets={secrets}
          setConfig={setConfig}
          setSecrets={setSecrets}
          clearSaved={() => setSaved(false)}
        />
      </div>

      <div className="flex flex-col gap-2">
        <Label>Encryption</Label>
        <SyncField
          id="sync-passphrase"
          label="Passphrase"
          description="Everything is encrypted with this before it is uploaded, so the storage provider never sees a title or a password. It is written into the vault on this device and never uploaded, and every device you sync must be given the same one. Leave it blank to reuse the one already stored."
          type="password"
          autoComplete="off"
          placeholder={KEEP_STORED}
          value={secrets.passphrase}
          onChange={(e) => setSecrets({ ...secrets, passphrase: e.target.value })}
        />
        <StrengthMeter value={secrets.passphrase} className="px-1" />
      </div>

      <SyncBehaviour
        config={config}
        onChange={(next) => {
          setConfig(next);
          setSaved(false);
        }}
      />

      <div className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            className="h-8 px-3 text-[11px]"
            disabled={busy}
            onClick={() => void onSave()}
          >
            {busy ? "Saving" : "Save"}
          </Button>
          {saved ? (
            <span role="status" className="text-muted-foreground text-[10.5px]">
              Saved. A pull has been requested so the new settings take effect.
            </span>
          ) : null}
        </div>
        {joined ? (
          <div
            role="status"
            className="border-border/60 bg-card flex flex-col gap-1 rounded-lg border px-3 py-2.5"
          >
            <span className="text-[11.5px] font-semibold">
              Another device created the keyfile. This device joined it.
            </span>
            <span className="text-muted-foreground text-[10.5px] leading-relaxed">
              The passphrase you entered opened the keyfile that appeared at this location between
              your save and your confirmation, so this device is now using that vault rather than
              starting an empty one.
            </span>
          </div>
        ) : null}
        {error ? (
          <div
            role="alert"
            className="border-destructive/40 bg-destructive/5 flex flex-col gap-1 rounded-lg border px-3 py-2.5"
          >
            <span className="text-destructive text-[11.5px] font-semibold">Error</span>
            <span className="text-[10.5px] leading-relaxed break-words whitespace-pre-wrap">
              {error}
            </span>
          </div>
        ) : null}
      </div>

      <div className="flex flex-col gap-2">
        <Label>Status</Label>
        <SyncStatusPanel
          status={status}
          enabled={config.enabled}
          onRequest={request}
          onRefresh={refreshStatus}
        />
      </div>

      <FreshRemoteDialog
        open={fresh}
        onOpenChange={(next) => {
          if (busy) return;
          setFresh(next);
          if (!next) setDialogError(null);
        }}
        busy={busy}
        error={dialogError}
        onConfirm={() => void onCreateRemote()}
      />
    </div>
  );
}
