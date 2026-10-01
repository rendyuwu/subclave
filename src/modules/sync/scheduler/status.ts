// The status the status bar renders: the one-time read that folds in what the
// last session left behind, and every later merge on top of it.

import type { SyncSettingsStore } from "../store";
import { EMPTY_SYNC_STATUS, type SyncStatus } from "../types";

export type StatusStore = {
  /** Merge `next` into the held status and hand the whole of it to the port. */
  publish(next: Partial<SyncStatus>): Promise<void>;
  /** The held status, for a pass that reads a figure back out of it. */
  read(): SyncStatus;
};

/** Build the held status over the status file store. */
export function createStatusStore(
  settings: SyncSettingsStore,
  onStatus: (status: SyncStatus) => void,
): StatusStore {
  /** The one-time read that folds the last session's status in. See
   *  {@link hydrate}. */
  let loaded: Promise<void> | null = null;
  let status: SyncStatus = { ...EMPTY_SYNC_STATUS };

  /**
   * Fold what the last session left in the file into the held status, ONCE.
   *
   * WITHOUT IT the first write of a session merges over defaults and puts zero
   * pending and no last pull over what the previous session had found, which is
   * exactly what a user opens the status card during an outage to look at.
   *
   * RESET ON FAILURE, or one unreadable file is permanent for the session: the
   * memoized promise would reject on every later write. Swallowed rather than
   * rethrown, because the write it precedes is still worth doing.
   */
  function hydrate(): Promise<void> {
    if (loaded) return loaded;
    const run = settings
      .readStatus()
      .then((stored) => {
        status = stored;
      })
      .catch(() => {
        loaded = null;
      });
    loaded = run;
    return run;
  }

  async function publish(next: Partial<SyncStatus>): Promise<void> {
    await hydrate();
    status = { ...status, ...next };
    onStatus(status);
  }

  // Started at construction so the one file read overlaps app startup: the
  // first publish then resolves in a microtask rather than behind a read.
  void hydrate();

  return { publish, read: () => status };
}
