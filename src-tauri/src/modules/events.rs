//! Names of Tauri events the Rust process EMITS to the webview.
//!
//! Mirror of the `IPC_EVENTS` constants in `src/lib/ipc.ts`. A typo on either
//! side just makes the listener never fire (silent), so both sides reference a
//! single named constant instead of a bare string literal.

/// Rust -> Settings webview: focus a settings tab (payload: tab id string).
pub const SETTINGS_TAB: &str = "subclave:settings-tab";

/// Vault locked (payload: `{ reason: "manual" | "idle" }`).
pub const VAULT_LOCKED: &str = "subclave:vault-locked";
/// Vault content changed (payload: `{ ids: string[], origin: "local" }`).
pub const VAULT_CHANGED: &str = "subclave:vault-changed";
/// Vault write failed, or `null` when a retry succeeded.
pub const VAULT_SAVE_FAILED: &str = "subclave:vault-save-failed";

/// Rust -> main webview: the app was asked to quit while a write is still
/// pending. Payload: `null`. The webview owns the confirmation dialog and
/// calls `quit_subclave` once the user decides.
pub const QUIT_REQUESTED: &str = "subclave:quit-requested";
