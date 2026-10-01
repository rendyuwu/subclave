// Canonical TypeScript mirrors of the Rust IPC payload enums. Keep in lockstep
// with the #[derive(Serialize)] types in src-tauri/src/modules/fs/file.rs.
// Defined ONCE and imported everywhere so the shapes cannot silently diverge -
// a hand-copied variant that dropped `image` previously caused real images to
// be mishandled as "non-text" and skipped.

/** Mirrors Rust `fs::file::ReadResult` (command: fs_read_file). */
export type FsReadResult =
  | { kind: "text"; content: string; size: number }
  | { kind: "image"; dataUrl: string; mime: string; size: number }
  | { kind: "binary"; size: number }
  | { kind: "toolarge"; size: number; limit: number };

/**
 * Names of Tauri events emitted by the RUST process and listened to on the TS
 * side. Magic strings on both sides drift silently (a typo just never fires),
 * so every TS listener references these constants. Mirror = the `emit(...)`
 * calls on the Rust side (`src-tauri/src/commands.rs`,
 * `src-tauri/src/windows.rs`, the vault modules).
 */
export const IPC_EVENTS = {
  /** Rust -> Settings webview: focus a settings tab (payload: tab id string). */
  SETTINGS_TAB: "subclave:settings-tab",
  /**
   * Vault locked (payload: `{ reason: "manual" | "idle" | "minimize" | "tray" }`).
   */
  VAULT_LOCKED: "subclave:vault-locked",
  /** Vault content changed (payload: `{ ids: string[], origin: "local" }`). */
  VAULT_CHANGED: "subclave:vault-changed",
  /**
   * Rust -> main webview: a browser extension asked to pair. Payload:
   * `{ requestId: string, browser: string, profileName: string, code: string }`.
   */
  PAIRING_REQUEST: "subclave:pairing-request",
  /** Vault write failed, or `null` when a retry succeeded. */
  VAULT_SAVE_FAILED: "subclave:vault-save-failed",
  /**
   * Rust -> main webview: a quit was requested while a save is still failing.
   * The webview confirms, then calls `quit_subclave`. Payload: `null`.
   */
  QUIT_REQUESTED: "subclave:quit-requested",
  /**
   * Rust -> main webview: the main window regained focus. The sync module
   * rate-limits its own pulls, so this fires on every focus. Payload: `null`.
   */
  SYNC_FOCUSED: "subclave:sync-focused",
} as const;
