import { emit } from "@tauri-apps/api/event";
import { useMemo, useState, type FormEvent } from "react";

import { createFileKeyValueStore } from "@/lib/fileKeyValueStore";
import { tauriStoreFileIo } from "@/lib/storeFileIo";
import { syncJoin } from "@/modules/sync/ipc";
import { createSyncSettingsStore } from "@/modules/sync/store";
import {
  configSubset,
  missingConnectionFields,
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

/** The labels of the credential fields the selected provider reads that are still blank. */
function missingCredentials(credentials: JoinCredentials, provider: string): string[] {
  const fields: [label: string, value: string][] =
    provider === "webdav"
      ? [
          ["Username", credentials.username],
          ["Password", credentials.webdavPassword],
        ]
      : [
          ["Access key ID", credentials.accessKeyId],
          ["Secret access key", credentials.secretAccessKey],
        ];
  return fields.filter(([, value]) => value.length === 0).map(([label]) => label);
}

/**
 * Everything the join screen's form does: the one submit, its busy flag, its
 * error, the `fresh` answer a remote with no keyfile gives, and `blockers`,
 * every reason other than `busy` that Submit is disabled, which the screen
 * lists under it.
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
  masterPasswordProblems,
}: {
  config: SyncConfig;
  credentials: JoinCredentials;
  passphrase: string;
  password: string;
  /** `problems` from `useMasterPassword`: length, confirmation and the no-recovery acknowledgement. */
  masterPasswordProblems: string[];
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

  // Every reason Join vault is disabled, in the order the fields render. The
  // list and the button cannot disagree: while it is non-empty Submit is
  // disabled, and the only other reason is `busy`, which shows the spinner.
  const empty = [
    ...missingConnectionFields(config),
    ...missingCredentials(credentials, config.provider),
    ...(passphrase.length === 0 ? ["Sync passphrase"] : []),
  ];
  const blockers = [
    ...(empty.length > 0 ? [`Still empty: ${empty.join(", ")}.`] : []),
    ...masterPasswordProblems,
  ];
  const canSubmit = blockers.length === 0 && !busy;

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

  return { submit, busy, error, fresh, canSubmit, blockers };
}
