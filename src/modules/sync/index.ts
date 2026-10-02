// The one place this module touches the running app: the real store file, the
// real commands, the injected ports, and the events.
//
// EVERY IMPORT EDGE POINTS THIS WAY. `modules/sync` knows about `modules/vault`;
// the vault module knows nothing about this one. That is why the push trigger
// arrives as an event (`subclave:vault-changed`, with a `sync` origin ignored)
// rather than a call from the vault store: a store importing a scheduler would
// put a network module behind every entry edit, and every check that builds a
// store would have to construct one.

import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

import { createFileKeyValueStore } from "@/lib/fileKeyValueStore";
import { IPC_EVENTS } from "@/lib/ipc";
import { tauriStoreFileIo } from "@/lib/storeFileIo";
import { useVaultStore } from "@/modules/vault/store";

import { syncCommands } from "./ipc";
import { createScheduler } from "./scheduler";
import {
  createSyncSettingsStore,
  setSyncSettingsStore,
  useSyncStore,
  type SyncSettingsStore,
} from "./store";
import {
  SYNC_CONFIG_EVENT,
  SYNC_REQUEST_EVENT,
  SYNC_STATUS_EVENT,
  SYNC_STORE_PATH,
  type SyncRequest,
} from "./types";

/** The teardown for the scheduler currently running in this webview. */
let stop: (() => void) | null = null;

/**
 * Re-read the stored configuration into the status bar's mirror.
 *
 * The settings window is the other writer of the file and nothing in the main
 * window notices a change to it, so the mirror is refreshed after every status
 * write and on both settings events.
 *
 * A file that cannot be read leaves the mirror as it was.
 */
async function refreshConfigMirror(settings: SyncSettingsStore): Promise<void> {
  try {
    useSyncStore.getState().setConfig(await settings.readConfig());
  } catch {
    // The next read is what fixes it.
  }
}

/**
 * Start sync for this webview, and answer with how to stop it.
 *
 * SAFE IN EVERY WEBVIEW: `createScheduler` returns an inert object outside
 * `main`, so calling this from a shared entry point costs one store handle and a
 * few listeners that never fire anything.
 *
 * A SECOND CALL ANSWERS WITH THE FIRST CALL'S TEARDOWN rather than a no-op: a
 * no-op is the shape that leaves sync unstoppable, with whoever holds it
 * believing it can stop what it started.
 */
export function startSync(): () => void {
  if (stop) return stop;
  const settings = createSyncSettingsStore(
    createFileKeyValueStore(SYNC_STORE_PATH, tauriStoreFileIo),
  );
  setSyncSettingsStore(settings);
  const scheduler = createScheduler({
    label: getCurrentWebviewWindow().label,
    commands: syncCommands,
    settings,
    onStatus: (status) => {
      useSyncStore.getState().setStatus(status);
      // AFTER the status change, so a listener that re-reads sees the file.
      void settings.writeStatus(status);
      void emit(SYNC_STATUS_EVENT, status);
      void refreshConfigMirror(settings);
    },
    onPhase: (phase) => useSyncStore.getState().setPhase(phase),
  });
  void useSyncStore.getState().hydrate();
  void refreshConfigMirror(settings);

  // LOCK TRANSITIONS ARE THE PULL AND PUSH TRIGGER. A pull on unlock is the
  // app's startup reconcile; a lock pauses every later trigger until the vault
  // is open again.
  const unsubscribeLocks = useVaultStore.subscribe((state, previous) => {
    if (previous.status?.locked === state.status?.locked) return;
    if (state.status?.locked) scheduler.onLocked();
    else scheduler.onUnlocked();
  });

  // Caught at construction as well as at teardown: an unhandled rejection here
  // would surface as a console error with no owner, on a path that is allowed to
  // fail, because a webview with no such event is a webview that never pulls on
  // focus.
  const unlistenFocus = listen(IPC_EVENTS.SYNC_FOCUSED, () => scheduler.onFocus()).catch(
    () => () => {},
  );
  // A landed pull emits `subclave:vault-changed` with the `sync` origin; that is
  // not a local edit and must not schedule a push.
  const unlistenChanged = listen<{ origin?: string }>(IPC_EVENTS.VAULT_CHANGED, (event) => {
    if (event.payload?.origin !== "sync") scheduler.markDirty();
  }).catch(() => () => {});
  // The settings window asking for something only `main` may do. A pull covers
  // "the configuration changed" as well: a new configuration is only observable
  // by reconciling against it.
  const unlistenRequest = listen<SyncRequest>(SYNC_REQUEST_EVENT, (event) => {
    // THE MEMO AND THE MIRROR ARE BOTH DROPPED, because the settings window may
    // have just changed the configuration or opened the session itself.
    void refreshConfigMirror(settings);
    scheduler.invalidateSession();
    if (event.payload === "push") void scheduler.pushNow();
    else void scheduler.pullNow();
  }).catch(() => () => {});
  // A toggle-off (and the configure that writes the config) emits no request, so
  // this is what reaches the pill.
  const unlistenConfig = listen(SYNC_CONFIG_EVENT, () => {
    void refreshConfigMirror(settings);
  }).catch(() => () => {});

  stop = () => {
    stop = null;
    unsubscribeLocks();
    scheduler.dispose();
    for (const pending of [unlistenFocus, unlistenChanged, unlistenRequest, unlistenConfig]) {
      void pending.then((off) => off());
    }
  };
  return stop;
}
