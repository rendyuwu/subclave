// The message contract between the popup (or the command path) and the service
// worker, and between the service worker and the content script.

import type { LoginSummary, NmErrorCode } from "./protocol";

/** Sent service worker -> popup before the pairing response arrives. */
export const PAIRING_CODE_EVENT = "subclave:pairing-code";

export type PairingCodeEvent = { type: typeof PAIRING_CODE_EVENT; code: string };

export type PopupRequest =
  | { type: "get-state" }
  | { type: "pair" }
  | { type: "fill-entry"; entryId: string; via: "popup" | "command" }
  | { type: "generate-and-fill"; entryId: string | null; via: "popup" | "command" }
  | { type: "settings-get" }
  | { type: "settings-set"; showInLoginFields: boolean }
  | { type: "focus-app" }
  | { type: "fill-command" };

export type SwState =
  | { state: "locked" }
  | { state: "not-running" }
  | { state: "unpaired" }
  | { state: "ready"; host: string; entries: LoginSummary[]; showInLoginFields: boolean }
  | { state: "error"; code: NmErrorCode; message: string };

export type SwResponse =
  | { type: "state"; state: SwState }
  | { type: "pair"; ok: true; code: string }
  | { type: "pair"; ok: false; code: NmErrorCode; message: string }
  | { type: "fill"; ok: true; filled: number }
  | { type: "fill"; ok: false; code: NmErrorCode; message: string }
  | { type: "generate"; ok: true; saved: boolean; filled: number }
  | { type: "generate"; ok: false; code: NmErrorCode; message: string }
  | { type: "settings"; showInLoginFields: boolean }
  | { type: "focus"; ok: boolean }
  | { type: "error"; code: NmErrorCode; message: string };

export type ContentRequest =
  | { type: "subclave:ping" }
  | { type: "subclave:fill"; username: string; password: string }
  | { type: "subclave:generate-fill"; password: string };

export type ContentFillResult = { username: boolean; password: boolean };
export type ContentGenerateResult = { username: string; filled: number };

/** The service worker's `chrome.runtime.onMessage` handler answers this. */
export function sendToBackground<T>(request: PopupRequest): Promise<T> {
  return chrome.runtime.sendMessage(request) as Promise<T>;
}
