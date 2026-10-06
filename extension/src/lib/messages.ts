// The message contract between the popup (or the command path) and the service
// worker, between the content script's inline picker and the service worker,
// and between the service worker and the content script.

import type { LoginSummary, NmErrorCode, SaveCandidate } from "./protocol";

/** Sent service worker -> popup (or the tab whose picker asked to pair) before
 * the pairing response arrives. */
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

/** Sent by the top-frame content script only; the service worker answers these
 * for the sender's own tab and its `sender.url`, never a URL the page names.
 * The save-prompt requests (`inline-pending-save`, `inline-save`,
 * `inline-save-cancel`) act on the sign-in that same tab submitted: it saves
 * to the `sender.url` stamped on its `inline-submitted`, and shows only on a
 * page of that URL's site (`check-login` decides). `id` names the pending
 * sign-in the prompt shows, so a newer one is never written in its place.
 * `inline-open-popup` only opens the toolbar popup, which lists the wider
 * matches for the active tab in browser-owned UI; it names no entry and no URL. */
export type InlineRequest =
  | { type: "inline-settings" }
  | { type: "inline-logins" }
  | { type: "inline-fill"; entryId: string }
  | { type: "inline-generate"; entryId: string | null }
  | { type: "inline-pair" }
  | { type: "inline-focus-app" }
  | { type: "inline-submitted"; username: string; password: string }
  | { type: "inline-pending-save" }
  | { type: "inline-save"; id: string; entryId: string | null }
  | { type: "inline-save-cancel"; id: string }
  | { type: "inline-open-popup" };

/** A sign-in waiting for the save prompt. `id` is the pending sign-in's own;
 * `host` is the submitted page's hostname. */
export type SavePrompt = { id: string; host: string; username: string; entries: SaveCandidate[] };

export type SwState =
  | { state: "locked" }
  | { state: "not-running" }
  | { state: "unpaired" }
  | {
      state: "ready";
      host: string;
      entries: LoginSummary[];
      otherMatches: number;
      domain: string;
      showInLoginFields: boolean;
    }
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
  | { type: "submitted" }
  | { type: "save-prompt"; prompt: SavePrompt | null }
  | { type: "save"; ok: true }
  | { type: "save"; ok: false; code: NmErrorCode; message: string }
  | { type: "popup"; ok: true }
  | { type: "popup"; ok: false; shortcut: string }
  | { type: "error"; code: NmErrorCode; message: string };

/** `url` is the page the credential was released for, and `anchored` says the
 * fill came from the inline picker; the content script refuses a fill that no
 * longer belongs to its document (see `content/index.ts`). */
export type ContentRequest =
  | { type: "subclave:ping" }
  | { type: "subclave:fill"; username: string; password: string; url: string; anchored: boolean }
  | { type: "subclave:generate-fill"; password: string; url: string; anchored: boolean }
  | PairingCodeEvent;

export type ContentFillResult = { username: boolean; password: boolean };
export type ContentGenerateResult = { username: string; filled: number };

/** The service worker's `chrome.runtime.onMessage` handler answers this. Async
 * so an orphaned content script (the extension was updated or reloaded) gets
 * the "Extension context invalidated" throw as a rejection it can handle. */
export async function sendToBackground<T>(request: PopupRequest | InlineRequest): Promise<T> {
  return (await chrome.runtime.sendMessage(request)) as T;
}
