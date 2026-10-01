// Typed wrappers for the browser-integration commands, and the payload types
// they return. THE ONLY FILE IN THIS MODULE THAT IMPORTS `invoke`: every command
// name is a literal on the same line as its call, because the command-registry
// scanner in `scripts/command-registry-verify.ts` reads it there.

import { invoke } from "@tauri-apps/api/core";

/** The two native-messaging families the app writes manifests for. */
export type BrowserFamily = "chromium" | "firefox";

/** One detected browser under a family row (mirrors `manifests::BrowserSlot`). */
export type BrowserSlotStatus = {
  browser: string;
  manifestPath: string;
  installed: boolean;
  sandboxed: boolean;
};

/**
 * One family row of the status array. `listenError` is an addition to that row:
 * it carries the socket server's refusal (a directory that is not private to
 * the user, or a pipe another instance holds) so the Settings tab can show it.
 */
export type BrowserFamilyStatus = {
  family: BrowserFamily;
  enabled: boolean;
  listenError: string | null;
  browsers: BrowserSlotStatus[];
};

export type BrowserIntegrationStatus = BrowserFamilyStatus[];

/** A paired browser, minus its secret (the app never hands that back). */
export type BrowserClientSummary = {
  id: string;
  name: string;
  family: string;
  pairedAt: number;
  lastSeenAt: number | null;
};

export function browserIntegrationStatus(): Promise<BrowserIntegrationStatus> {
  return invoke<BrowserIntegrationStatus>("browser_integration_status");
}

export function browserIntegrationSet(family: BrowserFamily, enabled: boolean): Promise<void> {
  return invoke("browser_integration_set", { family, enabled });
}

export function browserClientsList(): Promise<BrowserClientSummary[]> {
  return invoke<BrowserClientSummary[]>("browser_clients_list");
}

export function browserClientRename(id: string, name: string): Promise<void> {
  return invoke("browser_client_rename", { id, name });
}

export function browserClientRevoke(id: string): Promise<void> {
  return invoke("browser_client_revoke", { id });
}

export function browserPairingRespond(requestId: string, accept: boolean): Promise<void> {
  return invoke("browser_pairing_respond", { requestId, accept });
}
