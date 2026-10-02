// When a pass runs, and the queue every pass shares. The triggers, the
// debounce and the focus rate limit live here; the merge itself is Rust's.

import type { Scheduler } from "../scheduler";
import type { SyncSettingsStore } from "../store";
import {
  DEFAULT_SYNC_CONFIG,
  FOCUS_INTERVAL_MS,
  PUSH_DEBOUNCE_MS,
  type SyncCommands,
  type SyncConfig,
  type SyncPhase,
  type SyncStatus,
} from "../types";
import { message, type SessionStore } from "./session";
import type { StatusStore } from "./status";

/** What a policy needs from the webview it was built for. */
export type PolicyDeps = {
  commands: SyncCommands;
  settings: SyncSettingsStore;
  /** A pass started or settled, so the pill can show a spinner. */
  onPhase(phase: SyncPhase): void;
  now(): number;
  /** Injected so a check can fire the debounce without waiting five real
   *  seconds, and so a disposed scheduler's timer is cancellable. */
  setTimer(fn: () => void, ms: number): unknown;
  clearTimer(handle: unknown): void;
  status: StatusStore;
  session: SessionStore;
};

/** Every entry point, over the shared status and session stores. */
export function createPolicy(deps: PolicyDeps): Scheduler {
  const { commands, settings, onPhase, now, setTimer, clearTimer, status, session } = deps;

  /** The push debounce in flight, or nothing. */
  let pending: unknown = null;
  /** The pass in flight, so a second entry point queues behind it rather than
   *  interleaving two passes over the same Rust session. */
  let running: Promise<void> | null = null;
  /** When the last pull was attempted. `-Infinity` so the unlock pull is never
   *  rate limited away. */
  let lastAttempt = -Infinity;

  /**
   * Read the configuration FRESH from the file, never from the zustand mirror:
   * the settings window is the other writer of that file and the main window's
   * mirror can lag it.
   */
  async function readConfig(): Promise<SyncConfig> {
    try {
      return await settings.readConfig();
    } catch {
      // A file that cannot be read leaves sync off for this pass rather than
      // throwing out of a background path.
      return { ...DEFAULT_SYNC_CONFIG };
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
      if (!(await session.ensure(config))) return;
      const report = await commands.pull();
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
    await status.publish(
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
      if (!(await session.ensure(config))) return null;
      const report = await commands.push();
      const error = report.failed > 0 ? `sync: ${report.failed} records could not be pushed` : null;
      await status.publish({
        lastPushAt: now(),
        pending: Math.max(0, status.read().pending - report.pushed),
        lastError: error,
      });
      return error;
    } catch (e) {
      const reason = message(e);
      await status.publish({ lastError: reason });
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
      onPhase("syncing");
      return op().finally(() => onPhase("idle"));
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
      // CLEARED FIRST: `ensure` refuses to configure while the flag is set, so
      // the pull this queues would otherwise be a no-op.
      session.setPaused(false);
      void status.publish({ paused: false });
      void readConfig().then((config) => {
        if (config.enabled) void serialize(runPull);
        // A disable that happened while the vault was locked leaves the payload
        // residue behind, and this is what clears it.
        else void commands.disable().catch(() => {});
      });
    },
    onLocked() {
      session.setPaused(true);
      session.invalidate();
      void status.publish({ paused: true });
      // The dirty set lives in Rust and survives, so the next unlock pulls and
      // then pushes; nothing is owed by holding a timer across the lock.
      if (pending !== null) clearTimer(pending);
      pending = null;
    },
    pullNow: () => serialize(runPull),
    pushNow: () => serialize(runPush),
    invalidateSession() {
      session.invalidate();
    },
    dispose() {
      if (pending !== null) clearTimer(pending);
      pending = null;
    },
  };
}
