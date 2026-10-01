// Sync's own settings file: the configuration and the status the status bar
// renders.
//
// ITS OWN FILE, never a key inside another store's. `src/lib/fileKeyValueStore.ts`
// records that a contended write eventually gives up and writes this session's
// pending keys over a stale baseline, so every key a store writes is a key it
// can clobber, and a background pull writes the status on every run. Putting
// that beside the preferences would put the preferences inside the blast radius
// of a background pull.
//
// NO CREDENTIALS AND NO PASSPHRASE. This is plain JSON in the app data
// directory; the secret half goes into the vault payload's device state, sealed
// with the rest of the vault. No etag map and no dirty set either: Rust holds
// both, so a restart cannot lose them.

import { create } from "zustand";

import type { FileKeyValueStore } from "@/lib/fileKeyValueStore";

import {
  DEFAULT_SYNC_CONFIG,
  EMPTY_SYNC_STATUS,
  SYNC_CONFIG_KEY,
  SYNC_STATUS_KEY,
  type SyncConfig,
  type SyncPhase,
  type SyncStatus,
} from "./types";

/**
 * Every operation this module's two windows perform on that file.
 *
 * NO `invalidate` ON THIS PORT, deliberately: every method below already drops
 * the cache before it runs, so exposing one would be an invitation to call it
 * somewhere and conclude the rest did not need to.
 */
export type SyncSettingsStore = {
  readConfig(): Promise<SyncConfig>;
  /**
   * Store what the settings window collected.
   *
   * THE ONE KEY THE SETTINGS WINDOW WRITES, and the main window writes none of
   * it. Two windows writing one blob here would make a status write from the
   * main window able to roll back a configuration the user just typed.
   */
  writeConfig(config: SyncConfig): Promise<void>;
  /** What the last pass found, as the main window recorded it. */
  readStatus(): Promise<SyncStatus>;
  writeStatus(status: SyncStatus): Promise<void>;
};

/** A stored value that survived a hand edit of the file, or the default.
 *  Shape-checked rather than cast: this reads a file a user can open. */
function object<T extends object>(raw: unknown, fallback: T): T {
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return fallback;
  return { ...fallback, ...(raw as Partial<T>) };
}

export function createSyncSettingsStore(io: FileKeyValueStore): SyncSettingsStore {
  /**
   * Drop the cache, then do the thing.
   *
   * EVERY METHOD GOES THROUGH HERE, reads and writes alike, and it is one
   * wrapper rather than a line in each body so that a method added later cannot
   * be the one that forgets.
   *
   * On a READ it is what makes the other window's write visible: nothing
   * broadcasts a change event for this file, so a cached copy is otherwise
   * frozen at whatever the file said when this webview launched.
   *
   * On a WRITE it is what stops this window putting that frozen copy back:
   * `createFileKeyValueStore` writes the whole map, so a status write from a
   * background pull built on a baseline read before the user pressed Save would
   * silently revert the configuration they just entered.
   */
  async function fresh<T>(op: () => Promise<T>): Promise<T> {
    io.invalidate();
    return op();
  }

  return {
    readConfig: () => fresh(async () => object(await io.get(SYNC_CONFIG_KEY), DEFAULT_SYNC_CONFIG)),
    writeConfig: (config) =>
      fresh(async () => {
        await io.set(SYNC_CONFIG_KEY, config);
        await io.save();
      }),
    readStatus: () => fresh(async () => object(await io.get(SYNC_STATUS_KEY), EMPTY_SYNC_STATUS)),
    writeStatus: (status) =>
      fresh(async () => {
        await io.set(SYNC_STATUS_KEY, status);
        await io.save();
      }),
  };
}

/**
 * What the status bar and the settings form read, mirrored into zustand.
 *
 * A MIRROR, not the source of truth: the file is, and the main window writes it.
 * `hydrate` fills this once so a pill that mounts before the first pass still
 * shows the last session's figures.
 */
type SyncStoreState = {
  config: SyncConfig;
  status: SyncStatus;
  phase: SyncPhase;
  /** Read the file once. Idempotent; safe to call from more than one place. */
  hydrate(): Promise<void>;
  setConfig(config: SyncConfig): void;
  setStatus(status: SyncStatus): void;
  setPhase(phase: SyncPhase): void;
};

/** The file this mirror reads from, injected once by `startSync`. */
let settings: SyncSettingsStore | null = null;

/** Point the mirror at the store the main window reads and writes. */
export function setSyncSettingsStore(next: SyncSettingsStore): void {
  settings = next;
}

let hydrated: Promise<void> | null = null;

export const useSyncStore = create<SyncStoreState>((set) => ({
  config: DEFAULT_SYNC_CONFIG,
  status: EMPTY_SYNC_STATUS,
  phase: "idle",
  hydrate: async () => {
    const source = settings;
    // Nothing to read from yet: `startSync` injects the store before hydrating,
    // so this only happens for a caller in a webview that never started sync.
    if (!source) return;
    if (!hydrated) {
      hydrated = Promise.all([source.readConfig(), source.readStatus()])
        .then(([config, status]) => {
          set({ config, status });
        })
        .catch((e: unknown) => {
          // Reset on failure, or one rejection is permanent for the session: the
          // memoized promise would reject on every later call. The store layer
          // one level down resets for the same reason.
          hydrated = null;
          throw e;
        });
    }
    await hydrated;
  },
  setConfig: (config) => set({ config }),
  setStatus: (status) => set({ status }),
  setPhase: (phase) => set({ phase }),
}));
