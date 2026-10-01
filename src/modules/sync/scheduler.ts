// When sync runs, and what it does with what comes back.
//
// ONE WEBVIEW, AND IT IS `main`, the only one that may hold a sync session.
// Outside it every entry point is a no-op, and it is a no-op by CONSTRUCTION:
// `createScheduler` returns a different object, so there is no branch inside a
// hot path for a later edit to forget.
//
// THE TWO TRIGGERS. A local edit marks the record dirty in Rust and this file
// debounces the burst into one push. The window regaining focus pulls, behind a
// rate limit. Neither is a poll, and the rate limit is what keeps the focus
// trigger from becoming one under alt-tabbing.
//
// WHAT RUNS WHERE. Every decision (the merge, the prune, the etag skip, the
// dirty set) is in `src-tauri/src/modules/sync/engine.rs`. This file decides
// only WHEN and holds the status the status bar renders. It imports no Tauri
// surface at module scope: the commands and the store arrive as injected ports,
// so a plain node check can drive it.
//
// A LOCKED VAULT SENDS NOTHING. The Rust commands answer an error while locked,
// so a trigger that arrives then is dropped on `paused` rather than attempted.

import type { SyncSettingsStore } from "./store";
import {
  configSubset,
  DEFAULT_SYNC_CONFIG,
  EMPTY_SYNC_STATUS,
  FOCUS_INTERVAL_MS,
  PUSH_DEBOUNCE_MS,
  type SyncCommands,
  type SyncConfig,
  type SyncPhase,
  type SyncStatus,
} from "./types";

export type SchedulerIo = {
  /** This webview's label. Everything is a no-op unless it is `main`. */
  label: string;
  commands: SyncCommands;
  settings: SyncSettingsStore;
  /** The status changed: the port mirrors it, persists it and tells the other
   *  window. */
  onStatus(status: SyncStatus): void;
  /** A pass started or settled, so the pill can show a spinner. */
  onPhase(phase: SyncPhase): void;
  now?(): number;
  /** Injected so a check can fire the debounce without waiting five real
   *  seconds, and so a disposed scheduler's timer is cancellable. */
  setTimer?(fn: () => void, ms: number): unknown;
  clearTimer?(handle: unknown): void;
};

export type Scheduler = {
  /** A local edit landed in the vault: collect it into the next push. */
  markDirty(): void;
  /** The window regained focus. Rate limited. */
  onFocus(): void;
  /** The vault was unlocked: open a session and reconcile. */
  onUnlocked(): void;
  /** The vault locked: pause, and drop the session and the pending push. */
  onLocked(): void;
  /** Reconcile now, then flush what the remote turned out to be missing. */
  pullNow(): Promise<void>;
  /** Publish everything Rust marks dirty, now. */
  pushNow(): Promise<void>;
  /** Forget the memoized session, so the next trigger configures again. */
  invalidateSession(): void;
  /** Drop the pending debounce. */
  dispose(): void;
};

/** Every entry point, doing nothing. What a non-`main` webview gets. */
const INERT: Scheduler = {
  markDirty: () => {},
  onFocus: () => {},
  onUnlocked: () => {},
  onLocked: () => {},
  pullNow: async () => {},
  pushNow: async () => {},
  invalidateSession: () => {},
  dispose: () => {},
};

/** The message a `configure` answer carries when the remote has no keyfile. */
const FRESH_REMOTE_ERROR = "No Subclave data at this location";

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function createScheduler(io: SchedulerIo): Scheduler {
  if (io.label !== "main") return INERT;

  const now = io.now ?? Date.now;
  const setTimer = io.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
  const clearTimer = io.clearTimer ?? ((handle) => clearTimeout(handle as never));

  /** The push debounce in flight, or nothing. */
  let pending: unknown = null;
  /** The pass in flight, so a second entry point queues behind it rather than
   *  interleaving two passes over the same Rust session. */
  let running: Promise<void> | null = null;
  /** The one-time read that folds the last session's status in. See
   *  {@link hydrate}. */
  let loaded: Promise<void> | null = null;
  /** When the last pull was attempted. `-Infinity` so the unlock pull is never
   *  rate limited away. */
  let lastAttempt = -Infinity;
  /** The configuration the open session was opened with, as a fingerprint.
   *  `null` means no session is known to be open. */
  let session: string | null = null;
  /** True while the vault is locked: every trigger is dropped rather than
   *  attempted against a locked vault. */
  let paused = false;
  let status: SyncStatus = { ...EMPTY_SYNC_STATUS };

  /**
   * Read the configuration FRESH from the file, never from the zustand mirror:
   * the settings window is the other writer of that file and the main window's
   * mirror can lag it.
   */
  async function readConfig(): Promise<SyncConfig> {
    try {
      return await io.settings.readConfig();
    } catch {
      // A file that cannot be read leaves sync off for this pass rather than
      // throwing out of a background path.
      return { ...DEFAULT_SYNC_CONFIG };
    }
  }

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
    const run = io.settings
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

  /** Merge `next` into the held status and hand the whole of it to the port. */
  async function publish(next: Partial<SyncStatus>): Promise<void> {
    await hydrate();
    status = { ...status, ...next };
    io.onStatus(status);
  }

  // Started at construction so the one file read overlaps app startup: the
  // first publish then resolves in a microtask rather than behind a read.
  void hydrate();

  /**
   * Open a session in the Rust process for the stored configuration, unless one
   * is already open for it.
   *
   * MEMOIZED ON A FINGERPRINT of the configure arguments, so a trigger that
   * arrives before the unlock pull does not open a second session, and a
   * configuration the settings window just changed does open a new one. The
   * passphrase and the credentials are NOT in those arguments: Rust re-opens
   * from the root key and credentials already in the vault payload.
   *
   * ONLY A SESSION-OPENING ANSWER IS REMEMBERED. A `"fresh"` remote (no keyfile
   * at this location) or a refusal returns false WITHOUT memoizing, so a keyfile
   * that appears later is noticed on the next trigger.
   */
  async function ensureSession(config: SyncConfig): Promise<boolean> {
    if (paused || !config.enabled) return false;
    const fingerprint = JSON.stringify(configSubset(config));
    if (fingerprint === session) return true;
    try {
      const args = { config: configSubset(config), create: false };
      const answer = await io.commands.configure(args);
      if (answer.remote === "fresh") {
        await publish({ lastError: FRESH_REMOTE_ERROR, paused: false });
        return false;
      }
      session = fingerprint;
      return true;
    } catch (e) {
      await publish({ lastError: message(e) });
      return false;
    }
  }

  /**
   * Reconcile against the remote, then flush what the remote turned out to be
   * missing.
   *
   * ALWAYS FLUSHES, on success and on failure alike: a dirty mark Rust is
   * holding has nothing else that would notice it. The flush is the
   * un-serialized push body below, because `pushNow` would queue behind this
   * very pass.
   */
  async function runPull(): Promise<void> {
    const config = await readConfig();
    if (!config.enabled) return;
    // Stamped before anything can fail, so a refused or hung pull still costs
    // the focus window.
    lastAttempt = now();
    let error: string | null = null;
    let found: Pick<SyncStatus, "lastPullAt" | "pending" | "quarantine" | "stale"> | null = null;
    try {
      if (!(await ensureSession(config))) return;
      const report = await io.commands.pull();
      found = {
        lastPullAt: now(),
        pending: report.pending,
        quarantine: report.quarantine,
        stale: report.stale,
      };
    } catch (e) {
      error = message(e);
    }
    const pushError = await runPush();
    // A FAILED PULL STAMPS NO `lastPullAt` AND NO COUNTS: `found` stays null, so
    // what is reported is the last thing this device actually learned.
    //
    // `paused` IS CLEARED ONLY BY A PASS THAT ANSWERED. A vault that locks while
    // this one is in flight makes the command fail, and writing `paused: false`
    // over the flag `onLocked` just set would let the next focus trigger open a
    // session against a locked vault.
    await publish(
      found
        ? { ...found, lastError: error ?? pushError, paused: false }
        : { lastError: error ?? pushError },
    );
  }

  /**
   * Publish everything Rust marks dirty.
   *
   * Returns what to report, so a pull that ends by calling this writes status
   * once: a second write inside the pull would put its own `null` over the error
   * the pull just recorded.
   */
  async function runPush(): Promise<string | null> {
    const config = await readConfig();
    if (!config.enabled) return null;
    try {
      if (!(await ensureSession(config))) return null;
      const report = await io.commands.push();
      const error = report.failed > 0 ? `sync: ${report.failed} records could not be pushed` : null;
      await publish({
        lastPushAt: now(),
        pending: Math.max(0, status.pending - report.pushed),
        lastError: error,
      });
      return error;
    } catch (e) {
      const reason = message(e);
      await publish({ lastError: reason });
      return reason;
    }
  }

  /**
   * Run `op` after whatever is already running.
   *
   * SERIAL, because every pass is a read-modify-write over the same Rust session
   * and vault payload. Queued rather than dropped: a push carries edits, and
   * discarding one would lose them until the next mutation.
   *
   * NOTHING THAT COMES OUT OF HERE REJECTS. Every caller is `void
   * serialize(...)` or a fire-and-forget event handler, and a failure would
   * otherwise surface as an unhandled rejection with no owner, on a background
   * path that is allowed to fail.
   */
  function serialize(op: () => Promise<unknown>): Promise<void> {
    const wrapped = () => {
      io.onPhase("syncing");
      return op().finally(() => io.onPhase("idle"));
    };
    const next = (running ?? Promise.resolve()).then(wrapped, wrapped).then(
      () => {},
      (e: unknown) => {
        console.error("sync: a pass failed", e);
      },
    );
    running = next;
    return next;
  }

  return {
    markDirty() {
      if (pending !== null) return;
      // The window opens at the FIRST edit of a burst rather than sliding with
      // each one, so a long stream of edits still publishes every five seconds
      // instead of never.
      pending = setTimer(() => {
        pending = null;
        void serialize(runPush);
      }, PUSH_DEBOUNCE_MS);
    },
    onFocus() {
      const at = now();
      if (at - lastAttempt < FOCUS_INTERVAL_MS) return;
      // Stamped BEFORE the pull rather than after it, so a pull that fails or
      // hangs still costs the rate limit instead of being retried on every
      // alt-tab.
      lastAttempt = at;
      void serialize(runPull);
    },
    onUnlocked() {
      // CLEARED FIRST: `ensureSession` refuses to configure while the flag is
      // set, so the pull this queues would otherwise be a no-op.
      paused = false;
      void publish({ paused: false });
      void readConfig().then((config) => {
        if (config.enabled) void serialize(runPull);
        // A disable that happened while the vault was locked leaves the payload
        // residue behind, and this is what clears it.
        else void io.commands.disable().catch(() => {});
      });
    },
    onLocked() {
      paused = true;
      session = null;
      void publish({ paused: true });
      // The dirty set lives in Rust and survives, so the next unlock pulls and
      // then pushes; nothing is owed by holding a timer across the lock.
      if (pending !== null) clearTimer(pending);
      pending = null;
    },
    pullNow: () => serialize(runPull),
    pushNow: () => serialize(runPush),
    invalidateSession() {
      session = null;
    },
    dispose() {
      if (pending !== null) clearTimer(pending);
      pending = null;
    },
  };
}
