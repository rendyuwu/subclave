import { emit } from "@tauri-apps/api/event";
import { useMemo, useState, type FormEvent } from "react";

import { createFileKeyValueStore } from "@/lib/fileKeyValueStore";
import { tauriStoreFileIo } from "@/lib/storeFileIo";
import { syncJoin } from "@/modules/sync/ipc";
import { createSyncSettingsStore } from "@/modules/sync/store";
import {
  configSubset,
  connectionFieldsReady,
  SYNC_CONFIG_EVENT,
  SYNC_STORE_PATH,
  type SyncConfig,
} from "@/modules/sync/types";
import { describeVaultError } from "../errors";
import { useVaultStore } from "../store";

/** The four credential values the join form collects, both provider pairs at once. */
export type JoinCredentials = {
  accessKeyId: string;
  secretAccessKey: string;
  username: string;
  webdavPassword: string;
};

/** A join form with nothing typed into it yet. */
export const EMPTY_JOIN_CREDENTIALS: JoinCredentials = {
  accessKeyId: "",
  secretAccessKey: "",
  username: "",
  webdavPassword: "",
};

/** Whether the credential pair the selected provider reads is filled in. */
export function credentialsReady(credentials: JoinCredentials, provider: string): boolean {
  return provider === "webdav"
    ? credentials.username.length > 0 && credentials.webdavPassword.length > 0
    : credentials.accessKeyId.length > 0 && credentials.secretAccessKey.length > 0;
}

/**
 * Everything the join screen's form does: the one submit, its busy flag, its
 * error, the `fresh` answer a remote with no keyfile gives, and the boolean the
 * Submit button is disabled on.
 *
 * A LANDED JOIN WRITES THE CONFIGURATION, switched on. The scheduler reads it
 * on the unlock that follows, and an off configuration makes it call
 * `sync_disable`, which clears the credentials and root key the join just
 * stored: the vault would keep its entries and silently stop syncing.
 *
 * `fresh` means nothing was written and the only way forward is to create a
 * vault here, which is why the screen offers it.
 */
export function useJoinSync({
  config,
  credentials,
  passphrase,
  password,
  masterPasswordReady,
}: {
  config: SyncConfig;
  credentials: JoinCredentials;
  passphrase: string;
  password: string;
  /** Length, confirmation and the no-recovery acknowledgement, from `useMasterPassword`. */
  masterPasswordReady: boolean;
}) {
  const refreshStatus = useVaultStore((s) => s.refreshStatus);
  const refresh = useVaultStore((s) => s.refresh);
  const settings = useMemo(
    () => createSyncSettingsStore(createFileKeyValueStore(SYNC_STORE_PATH, tauriStoreFileIo)),
    [],
  );

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fresh, setFresh] = useState(false);

  const canSubmit =
    masterPasswordReady &&
    connectionFieldsReady(config, credentialsReady(credentials, config.provider)) &&
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
        credentials: {
          accessKeyId: credentials.accessKeyId,
          secretAccessKey: credentials.secretAccessKey,
          username: credentials.username,
          password: credentials.webdavPassword,
        },
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

  return { submit, busy, error, fresh, canSubmit };
}
