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

import { emit, listen } from "@tauri-apps/api/event";
import { ChevronDown } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Switch } from "@/components/ui/switch";
import { createFileKeyValueStore } from "@/lib/fileKeyValueStore";
import { tauriStoreFileIo } from "@/lib/storeRecovery";
import { syncConfigure, syncDisable } from "@/modules/sync/ipc";
import { createSyncSettingsStore } from "@/modules/sync/store";
import {
  configSubset,
  DEFAULT_SYNC_CONFIG,
  EMPTY_SYNC_STATUS,
  SYNC_CONFIG_EVENT,
  SYNC_PROVIDERS,
  SYNC_REQUEST_EVENT,
  SYNC_STATUS_EVENT,
  SYNC_STORE_PATH,
  type SyncConfig,
  type SyncRequest,
  type SyncStatus,
} from "@/modules/sync/types";
import { StrengthMeter } from "@/modules/vault/editor/StrengthMeter";
import { describeVaultError } from "@/modules/vault/errors";
import { Label } from "../components/Label";
import { SectionHeader } from "../components/SectionHeader";
import { SettingRow } from "../components/SettingRow";
import { SyncField } from "../components/SyncField";
import { FreshRemoteDialog } from "./FreshRemoteDialog";

/**
 * What this session typed into the secret fields. Never read back out of the
 * vault, and never written into {@link SyncConfig}: it is the argument of one
 * `syncConfigure` call and then dropped.
 *
 * EVERY PROVIDER'S FIELDS IN ONE FLAT SHAPE, not one shape per provider. Only
 * the selected provider's fields are rendered, so the rest stay empty, and
 * every write below is already gated on "the user actually typed something",
 * which makes a per-provider union earn nothing but the narrowing it would then
 * demand at each call site.
 */
type SecretDraft = {
  passphrase: string;
  accessKeyId: string;
  secretAccessKey: string;
  webdavUsername: string;
  webdavPassword: string;
};

const EMPTY_SECRETS: SecretDraft = {
  passphrase: "",
  accessKeyId: "",
  secretAccessKey: "",
  webdavUsername: "",
  webdavPassword: "",
};

/** The credentials half of `SyncConfigureArgs`, whichever provider they belong
 *  to. Only filled fields are sent: a blank one would write the empty string,
 *  which every stored-credential check reads as a real secret. */
type CredentialsArg = {
  accessKeyId?: string;
  secretAccessKey?: string;
  username?: string;
  password?: string;
};

function credentialsArg(provider: string, draft: SecretDraft): CredentialsArg {
  const out: CredentialsArg = {};
  if (provider === "webdav") {
    if (draft.webdavUsername) out.username = draft.webdavUsername;
    if (draft.webdavPassword) out.password = draft.webdavPassword;
  } else {
    if (draft.accessKeyId) out.accessKeyId = draft.accessKeyId;
    if (draft.secretAccessKey) out.secretAccessKey = draft.secretAccessKey;
  }
  return out;
}

/** What a rejected `invoke` or store call is worth showing. Rust answers with
 *  `"<module>: <sentence>"`, which every other surface in the app reads as a
 *  sentence through `describeVaultError`, so this one does too. */
function errorText(e: unknown): string {
  const raw = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  return describeVaultError(raw);
}

/** The one placeholder every secret input wears, unconditionally. There is no
 *  presence read to vary it: no command exposes whether the vault holds one. */
const KEEP_STORED = "Leave blank to keep the stored value.";

export function SyncSection() {
  // Every method on this wrapper invalidates before it runs, which is what makes
  // a status `main` wrote visible here and a configuration written here visible
  // there. This window holds no zustand mirror, so the file is the only source.
  const settings = useMemo(
    () => createSyncSettingsStore(createFileKeyValueStore(SYNC_STORE_PATH, tauriStoreFileIo)),
    [],
  );

  const [config, setConfig] = useState<SyncConfig>(DEFAULT_SYNC_CONFIG);
  const [secrets, setSecrets] = useState<SecretDraft>(EMPTY_SECRETS);
  const [status, setStatus] = useState<SyncStatus>(EMPTY_SYNC_STATUS);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  // Set when the create call lost the race and joined another device's keyfile
  // instead; the note below the form is what says so.
  const [joined, setJoined] = useState(false);
  // The create confirmation. Open only after a save answered "fresh".
  const [fresh, setFresh] = useState(false);
  const [dialogError, setDialogError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    void (async () => {
      const [loadedConfig, loadedStatus] = await Promise.all([
        settings.readConfig(),
        settings.readStatus(),
      ]);
      if (!alive) return;
      setConfig(loadedConfig);
      setStatus(loadedStatus);
    })().catch((e: unknown) => {
      if (alive) setError(errorText(e));
    });
    return () => {
      alive = false;
    };
  }, [settings]);

  // The main window writes the status after every pull and push; re-read the
  // file rather than keeping a second copy of it.
  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | null = null;
    void listen(SYNC_STATUS_EVENT, () => {
      void settings.readStatus().then((next) => {
        if (alive) setStatus(next);
      });
    }).then((fn) => {
      if (alive) unlisten = fn;
      else fn();
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [settings]);

  /** Ask `main` to do the thing only `main` may do. Nothing here performs it. */
  const request = async (what: SyncRequest) => {
    await emit(SYNC_REQUEST_EVENT, what);
  };

  /** The arguments both the plain save and the create call send, minus what the
   *  passphrase draft does or does not contribute. */
  const configArguments = (create: boolean) => ({
    config: configSubset(config),
    credentials: credentialsArg(config.provider, secrets),
    ...(secrets.passphrase ? { passphrase: secrets.passphrase } : {}),
    create,
  });

  /** The shared tail of a successful save or create: the configuration takes
   *  effect only once it is written and `main` is asked for a pull. */
  const acceptConfiguration = async (joined: boolean) => {
    await settings.writeConfig({ ...config, enabled: true });
    setSecrets(EMPTY_SECRETS);
    setSaved(true);
    setJoined(joined);
    await emit(SYNC_CONFIG_EVENT);
    await request("pull");
    // LAST, so a failure anywhere above leaves the dialog open with the reason
    // in it rather than closing over an error that has nowhere to appear. This
    // runs on the plain save path too, where there is no dialog to close.
    setFresh(false);
  };

  const onSave = async () => {
    setBusy(true);
    setError(null);
    setSaved(false);
    setJoined(false);
    try {
      const result = await syncConfigure(configArguments(false));
      if (result.remote === "fresh") {
        // Nothing is written: the user has not agreed to create anything yet,
        // and the dialog is where that answer belongs.
        setDialogError(null);
        setFresh(true);
        return;
      }
      await acceptConfiguration(false);
    } catch (e: unknown) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const onCreateRemote = async () => {
    setBusy(true);
    setDialogError(null);
    try {
      const result = await syncConfigure(configArguments(true));
      // "existing" here means another device minted the keyfile in the window
      // between the save and this confirm; the session is already open on the
      // winner's root key, so the only thing left to say is which happened.
      await acceptConfiguration(result.remote === "existing");
    } catch (e: unknown) {
      setDialogError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const onToggleEnabled = async (next: boolean) => {
    const before = config;
    const updated = { ...config, enabled: next };
    setConfig(updated);
    setSaved(false);
    // Turning it ON waits for Save, because the fields beside it may be
    // half-typed. Turning it OFF must not wait for anything: off means no
    // network, and a user who switches it off and closes the window has to get
    // that, not an unsaved intention.
    if (next) return;
    setBusy(true);
    setError(null);
    try {
      await settings.writeConfig(updated);
      await emit(SYNC_CONFIG_EVENT);
      await syncDisable();
    } catch (e: unknown) {
      // THE SWITCH GOES BACK. It was moved optimistically, and if the write or
      // the close failed then sync is still running, which is a worse state
      // than the one that caused the error.
      setConfig(before);
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const providerLabel =
    SYNC_PROVIDERS.find((p) => p.id === config.provider)?.label ?? config.provider;
  // WebDAV differs from S3 in more than a field list: it authenticates with a
  // username and a password sent on every request, and it has no conditional
  // write to offer, so both the credential fields and the Behaviour block turn
  // on this.
  const webdav = config.provider === "webdav";

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
                    setSaved(false);
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

      <div className="flex flex-col gap-2">
        <Label>Behaviour</Label>
        {/* TWO WHOLE RENDERINGS rather than one with the toggle inside it. On a
            provider with no conditional write to offer there is no switch, and
            the note cannot then say "the setting above, which you chose" about
            a control that is not on the screen. */}
        {webdav ? (
          // A switch the user could move with no effect would be worse than no
          // switch: it would read as a promise. WebDAV leaves the conditional
          // write to each server, so this build never asks one for it, and the
          // consequence is stated unconditionally because nothing about it is
          // the user's to change.
          <div
            role="status"
            className="border-border/60 bg-card flex flex-col gap-1 rounded-lg border px-3 py-2.5"
          >
            <span className="text-[11.5px] font-semibold">Note: conditional writes are off</span>
            <span className="text-muted-foreground text-[10.5px] leading-relaxed">
              Two devices that write the same record at the same moment can leave only one of the
              two writes on the remote, and the other is lost without an error. WebDAV does not
              guarantee a server can refuse a write that would do that, so this app never asks one
              to, and there is nothing here to turn on. This is what this provider costs, not a
              setting you got wrong.
            </span>
          </div>
        ) : (
          <>
            <SettingRow
              title="Endpoint honours conditional writes"
              description="Turn this on only if you know your storage supports a write that fails when the object changed underneath it. This app does not test for it."
            >
              <Switch
                checked={config.cas}
                onCheckedChange={(v) => {
                  setConfig({ ...config, cas: v });
                  setSaved(false);
                }}
                aria-label="Endpoint honours conditional writes"
              />
            </SettingRow>
            {!config.cas ? (
              // Worded as a consequence of the SETTING, not as a finding.
              // Nothing in the app probes the endpoint, so a label claiming it
              // detected anything would be a claim no code backs.
              <div
                role="status"
                className="border-border/60 bg-card flex flex-col gap-1 rounded-lg border px-3 py-2.5"
              >
                <span className="text-[11.5px] font-semibold">
                  Note: conditional writes are off
                </span>
                <span className="text-muted-foreground text-[10.5px] leading-relaxed">
                  With this off, two devices that write the same record at the same moment can leave
                  only one of the two writes on the remote, and the other is lost without an error.
                  It is also what protects the sync folder itself: two devices creating it at the
                  same moment both report success, and the second one's folder replaces the first
                  one's, taking every record sealed under the key it held. Turning the switch on is
                  what lets this app notice that refusal instead of writing over it. This app does
                  not check what your endpoint supports; either setting is safe when only one device
                  writes at a time.
                </span>
              </div>
            ) : null}
          </>
        )}
      </div>

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
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            className="h-8 px-2 text-[11px]"
            disabled={!config.enabled}
            onClick={() => void request("pull")}
          >
            Pull now
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="h-8 px-2 text-[11px]"
            disabled={!config.enabled}
            onClick={() => void request("push")}
          >
            Push now
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="h-8 px-2 text-[11px]"
            onClick={() => void settings.readStatus().then((next) => setStatus(next))}
          >
            Refresh
          </Button>
        </div>
        <SettingRow title="Last pull">
          <span className="text-muted-foreground text-[11px]">
            {status.lastPullAt === null ? "Never" : new Date(status.lastPullAt).toLocaleString()}
          </span>
        </SettingRow>
        <SettingRow title="Last push">
          <span className="text-muted-foreground text-[11px]">
            {status.lastPushAt === null ? "Never" : new Date(status.lastPushAt).toLocaleString()}
          </span>
        </SettingRow>
        <SettingRow
          title="Waiting to be pushed"
          description="Records this device has changed that the remote does not hold yet: at least this many."
        >
          <span className="text-muted-foreground text-[11px] tabular-nums">{status.pending}</span>
        </SettingRow>

        {status.quarantine.length > 0 ? (
          <div className="border-border/60 bg-card flex flex-col gap-1.5 rounded-lg border px-3 py-2.5">
            <span className="text-[12.5px] font-medium">Unreadable remote objects</span>
            <span className="text-muted-foreground text-[10.5px] leading-relaxed">
              These could not be decrypted or parsed, so they were left alone. A wrong passphrase on
              one device is the usual cause. Nothing here is deleted.
            </span>
            <ul className="flex flex-col gap-1">
              {status.quarantine.map((q) => (
                <li key={q.name} className="font-mono text-[10.5px] break-all">
                  {q.name} - <span className="text-muted-foreground">{q.reason}</span>
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        {status.stale.length > 0 ? (
          <div className="border-border/60 bg-card flex flex-col gap-1.5 rounded-lg border px-3 py-2.5">
            <span className="text-[12.5px] font-medium">Local records the remote has dropped</span>
            <span className="text-muted-foreground text-[10.5px] leading-relaxed">
              This device still holds these and the remote no longer has an object for them. They
              are reported and never deleted. Each is older than the 90-day window a deletion
              travels in, so the likeliest reading is that it was deleted on another device long
              ago. A pull does not re-publish these: edit one to send it to the remote again, or
              delete it here to accept the removal.
            </span>
            <ul className="flex flex-col gap-1">
              {status.stale.map((s) => (
                <li key={`${s.kind}:${s.id}`} className="font-mono text-[10.5px] break-all">
                  {s.kind} - <span className="text-muted-foreground">{s.id}</span>
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        {status.lastError ? (
          <div
            role="alert"
            className="border-destructive/40 bg-destructive/5 flex flex-col gap-1 rounded-lg border px-3 py-2.5"
          >
            <span className="text-destructive text-[11.5px] font-semibold">
              Error on the last run
            </span>
            {/* In full. A truncated remote error is the one shape that reliably
                hides the clause naming the bucket or the missing permission. */}
            <span className="font-mono text-[10.5px] leading-relaxed break-words whitespace-pre-wrap">
              {status.lastError}
            </span>
          </div>
        ) : null}
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
