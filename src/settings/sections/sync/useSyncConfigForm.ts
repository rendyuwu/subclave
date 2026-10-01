import { emit, listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useState } from "react";

import { createFileKeyValueStore } from "@/lib/fileKeyValueStore";
import { tauriStoreFileIo } from "@/lib/storeFileIo";
import { syncConfigure, syncDisable } from "@/modules/sync/ipc";
import { createSyncSettingsStore } from "@/modules/sync/store";
import {
  configSubset,
  DEFAULT_SYNC_CONFIG,
  EMPTY_SYNC_STATUS,
  SYNC_CONFIG_EVENT,
  SYNC_REQUEST_EVENT,
  SYNC_STATUS_EVENT,
  SYNC_STORE_PATH,
  type SyncConfig,
  type SyncRequest,
  type SyncStatus,
} from "@/modules/sync/types";
import { credentialsArg, EMPTY_SECRETS, errorText, type SecretDraft } from "./form";

/**
 * Everything the sync tab's form holds and everything it can do: the one
 * explicit save, the create confirmation behind it, and the switch that must
 * not wait for either. The status `main` writes back is read here too, because
 * a save is what asks for it.
 */
export function useSyncConfigForm() {
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

  /** The status panel's Refresh: re-read the file rather than keep a copy. */
  const refreshStatus = async () => {
    setStatus(await settings.readStatus());
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

  return {
    config,
    setConfig,
    secrets,
    setSecrets,
    status,
    busy,
    error,
    saved,
    setSaved,
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
  };
}
