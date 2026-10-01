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
// dirty set) is in `src-tauri/src/modules/sync/engine/mod.rs`. This module
// decides only WHEN and holds the status the status bar renders. It imports no
// Tauri surface at module scope: the commands and the store arrive as injected
// ports, so a plain node check can drive it.
//
// A LOCKED VAULT SENDS NOTHING. The Rust commands answer an error while locked,
// so a trigger that arrives then is dropped on `paused` rather than attempted.
//
// The internals live in `scheduler/`: `status.ts` holds the rendered status,
// `session.ts` the one session per configuration, and `policy.ts` the triggers
// and the queue they share.

import type { SyncSettingsStore } from "./store";
import { type SyncCommands, type SyncPhase, type SyncStatus } from "./types";
import { createPolicy } from "./scheduler/policy";
import { createSessionStore } from "./scheduler/session";
import { createStatusStore } from "./scheduler/status";

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

export function createScheduler(io: SchedulerIo): Scheduler {
  if (io.label !== "main") return INERT;

  const now = io.now ?? Date.now;
  const setTimer = io.setTimer ?? ((fn, ms) => setTimeout(fn, ms));
  const clearTimer = io.clearTimer ?? ((handle) => clearTimeout(handle as never));

  const status = createStatusStore(io.settings, (next) => io.onStatus(next));
  const session = createSessionStore(io.commands, status.publish);
  return createPolicy({
    commands: io.commands,
    settings: io.settings,
    onPhase: (phase) => io.onPhase(phase),
    now,
    setTimer,
    clearTimer,
    status,
    session,
  });
}
