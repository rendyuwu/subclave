// The one sync session per webview, memoized on the configuration it was
// opened with.

import { configSubset, type SyncCommands, type SyncConfig, type SyncStatus } from "../types";

/** The message a `configure` answer carries when the remote has no keyfile. */
const FRESH_REMOTE_ERROR = "No Subclave data at this location";

/** The message of a thrown value, for a status line. */
export function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export type SessionStore = {
  /** Open a session for `config` unless one is already open for it. */
  ensure(config: SyncConfig): Promise<boolean>;
  /** Forget the memoized session, so the next trigger configures again. */
  invalidate(): void;
  /** True while the vault is locked: every trigger is dropped rather than
   *  attempted against a locked vault. */
  isPaused(): boolean;
  /** Set the lock flag that `ensure` refuses to configure against. */
  setPaused(value: boolean): void;
};

/** Build the session memo over the configure command and the status port. */
export function createSessionStore(
  commands: SyncCommands,
  publish: (next: Partial<SyncStatus>) => Promise<void>,
): SessionStore {
  /** True while the vault is locked: every trigger is dropped rather than
   *  attempted against a locked vault. */
  let paused = false;
  /** The configuration the open session was opened with, as a fingerprint.
   *  `null` means no session is known to be open. */
  let session: string | null = null;

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
  async function ensure(config: SyncConfig): Promise<boolean> {
    if (paused || !config.enabled) return false;
    const fingerprint = JSON.stringify(configSubset(config));
    if (fingerprint === session) return true;
    try {
      const args = { config: configSubset(config), create: false };
      const answer = await commands.configure(args);
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

  return {
    ensure,
    invalidate() {
      session = null;
    },
    isPaused: () => paused,
    setPaused(value) {
      paused = value;
    },
  };
}
