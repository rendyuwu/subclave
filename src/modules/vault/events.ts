import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { IPC_EVENTS } from "@/lib/ipc";

export type VaultLockReason = "manual" | "idle" | "minimize" | "tray";

export type VaultEventHandlers = {
  onLocked?: (reason: VaultLockReason) => void;
  onChanged?: () => void;
  /** The write failure reason, or `null` once a retry succeeded. */
  onSaveFailed?: (reason: string | null) => void;
  onQuitRequested?: () => void;
};

/**
 * Subscribe to the vault events the Rust side emits. Resolves to the unlisten
 * functions in subscription order; the app window holds them for its lifetime.
 */
export async function subscribeVaultEvents(handlers: VaultEventHandlers): Promise<UnlistenFn[]> {
  return Promise.all([
    listen<{ reason: VaultLockReason }>(IPC_EVENTS.VAULT_LOCKED, (event) =>
      handlers.onLocked?.(event.payload.reason),
    ),
    listen(IPC_EVENTS.VAULT_CHANGED, () => handlers.onChanged?.()),
    listen<string | null>(IPC_EVENTS.VAULT_SAVE_FAILED, (event) =>
      handlers.onSaveFailed?.(event.payload ?? null),
    ),
    listen(IPC_EVENTS.QUIT_REQUESTED, () => handlers.onQuitRequested?.()),
  ]);
}
