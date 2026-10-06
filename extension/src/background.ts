import {
  base64ToBytes,
  bytesToBase64,
  extProof,
  pairingCode,
  randomNonce,
  verifyAppProof,
} from "./lib/auth";
import type {
  ContentFillResult,
  ContentGenerateResult,
  ContentRequest,
  InlineRequest,
  PairingCodeEvent,
  PopupRequest,
  SavePrompt,
  SwResponse,
  SwState,
} from "./lib/messages";
import { PAIRING_CODE_EVENT } from "./lib/messages";
import { PROTOCOL_VERSION } from "./lib/protocol";
import type {
  CheckLoginResult,
  CredentialResult,
  GenerateResult,
  GetLoginsResult,
  HelloResult,
  NmAction,
  NmErrorCode,
  NmRequest,
  PairResult,
  SaveLoginResult,
  StatusResult,
} from "./lib/protocol";
import { createNativeTransport } from "./lib/transport";
import type { Transport } from "./lib/transport";

const CREDENTIALS_KEY = "subclave.credentials";
const SHOW_IN_LOGIN_FIELDS_KEY = "subclave.showInLoginFields";
/** One pending sign-in per tab, in `chrome.storage.session` so it survives a
 * service worker restart between the submit and the next page load. */
const PENDING_SAVE_PREFIX = "subclave.pendingSave.";
/** How long a submitted login waits for its prompt to be answered. */
const PENDING_SAVE_TTL_MS = 5 * 60_000;
/** How many page loads in its tab may show the prompt. */
const PENDING_SAVE_PAGE_LOADS = 3;

type PendingSave = {
  id: string;
  url: string;
  username: string;
  password: string;
  at: number;
  loads: number;
};

type Credentials = { clientId: string; secret: string };
type CallResult<T> = { ok: true; result: T } | { ok: false; code: NmErrorCode; message: string };

/** Static lookup for the message types the service worker answers, one table
 * per sender kind (`senderKind`). */
const POPUP_REQUEST_TYPES: Record<string, true> = {
  "get-state": true,
  pair: true,
  "fill-entry": true,
  "generate-and-fill": true,
  "settings-get": true,
  "settings-set": true,
  "focus-app": true,
  "fill-command": true,
};

const INLINE_REQUEST_TYPES: Record<string, true> = {
  "inline-settings": true,
  "inline-logins": true,
  "inline-fill": true,
  "inline-generate": true,
  "inline-pair": true,
  "inline-focus-app": true,
  "inline-submitted": true,
  "inline-pending-save": true,
  "inline-save": true,
  "inline-save-cancel": true,
  "inline-open-popup": true,
};

export type BackgroundOptions = {
  /** The Playwright harness installs a fake; production uses the native port. */
  transport?: Transport;
};

/**
 * The extension's only privileged code. Holds the connection lifecycle, the
 * handshake, the injection fallback and the fill command; the popup and the
 * content script reach all of it through `chrome.runtime.onMessage`.
 */
export function startBackground(options: BackgroundOptions = {}): void {
  const transport = options.transport ?? createNativeTransport();

  let credentials: Credentials | null = null;
  let authenticated = false;
  let showInLoginFields = true;
  let loaded = false;
  let loading: Promise<void> | null = null;
  let authenticating: Promise<CallResult<null>> | null = null;
  let sequence = 0;

  const request = (action: NmAction, params?: unknown): NmRequest => ({
    v: PROTOCOL_VERSION,
    id: `ext-${(sequence += 1)}`,
    action,
    params,
  });

  const fail = (code: NmErrorCode, message: string): CallResult<never> => ({
    ok: false,
    code,
    message,
  });

  const loadState = async (): Promise<void> => {
    if (loaded) return;
    if (!loading) {
      loading = (async () => {
        const stored = await chrome.storage.local.get([CREDENTIALS_KEY, SHOW_IN_LOGIN_FIELDS_KEY]);
        const creds = stored[CREDENTIALS_KEY] as Credentials | undefined;
        credentials =
          creds && typeof creds.clientId === "string" && typeof creds.secret === "string"
            ? creds
            : null;
        showInLoginFields = stored[SHOW_IN_LOGIN_FIELDS_KEY] !== false;
        loaded = true;
      })();
    }
    await loading;
  };

  const forgetCredentials = async (): Promise<void> => {
    credentials = null;
    authenticated = false;
    await chrome.storage.local.remove(CREDENTIALS_KEY);
  };

  const handleAuthFailure = async (
    code: NmErrorCode,
    message: string,
  ): Promise<CallResult<never>> => {
    if (code === "auth-failed" || code === "not-associated") await forgetCredentials();
    return fail(code, message);
  };

  const authenticate = async (): Promise<CallResult<null>> => {
    const current = credentials;
    if (!current) return fail("not-associated", "Pair with Subclave");
    const statusResponse = await transport.send(request("status"));
    if (!statusResponse.ok) return fail(statusResponse.error.code, statusResponse.error.message);
    const status = statusResponse.result as StatusResult;
    if (status.locked) return fail("vault-locked", "Subclave is locked");

    const extNonce = randomNonce();
    const helloResponse = await transport.send(
      request("hello", { clientId: current.clientId, extNonce: bytesToBase64(extNonce) }),
    );
    if (!helloResponse.ok)
      return handleAuthFailure(helloResponse.error.code, helloResponse.error.message);

    const hello = helloResponse.result as HelloResult;
    const appNonce = base64ToBytes(hello.appNonce);
    const secret = base64ToBytes(current.secret);
    const serverProofValid = await verifyAppProof(
      secret,
      extNonce,
      appNonce,
      base64ToBytes(hello.appProof),
    );
    if (!serverProofValid) {
      // A server that cannot prove itself gets dropped; the stored credentials
      // stay so the popup can retry against the real app.
      transport.close();
      return fail("auth-failed", "Subclave could not be verified");
    }

    const proof = await extProof(secret, appNonce, extNonce);
    const authResponse = await transport.send(request("auth", { extProof: bytesToBase64(proof) }));
    if (!authResponse.ok)
      return handleAuthFailure(authResponse.error.code, authResponse.error.message);

    authenticated = true;
    return { ok: true, result: null };
  };

  const ensureAuthenticated = async (): Promise<CallResult<null>> => {
    await loadState();
    if (authenticated) return { ok: true, result: null };
    if (!credentials) return fail("not-associated", "Pair with Subclave");
    // One handshake at a time: a second `hello` on the same connection is
    // `bad-request`, so concurrent callers share the first attempt.
    if (!authenticating) {
      authenticating = authenticate().finally(() => {
        authenticating = null;
      });
    }
    return authenticating;
  };

  // One recovery at a time, for the same reason as `authenticating`: two
  // concurrent calls that both read `not-associated` would otherwise each close
  // the transport, and the second close would tear down the connection the
  // first just re-authenticated.
  let recovering: Promise<CallResult<null>> | null = null;
  const recoverConnection = async (): Promise<CallResult<null>> => {
    if (!recovering) {
      authenticated = false;
      transport.close();
      recovering = ensureAuthenticated().finally(() => {
        recovering = null;
      });
    }
    return recovering;
  };

  const call = async <T>(action: NmAction, params?: unknown): Promise<CallResult<T>> => {
    const guard = await ensureAuthenticated();
    if (!guard.ok) return guard;
    const response = await transport.send(request(action, params));
    if (
      !response.ok &&
      (response.error.code === "auth-failed" || response.error.code === "not-associated")
    ) {
      // The app drops every socket when it locks (or restarts), but the native
      // port survives, so `authenticated` can describe a connection that no
      // longer exists: the next action lands on a fresh, un-greeted connection
      // and reads as `not-associated`. Reconnect and handshake once, shared
      // across concurrent callers. Only `hello` answering `not-associated`
      // means the client itself is gone, and that path forgets the credentials
      // inside `authenticate`, so a replaced connection never costs a re-pair.
      const retry = await recoverConnection();
      if (!retry.ok) return retry;
      const second = await transport.send(request(action, params));
      if (second.ok) return { ok: true, result: second.result as T };
      if (second.error.code === "auth-failed" || second.error.code === "not-associated") {
        return handleAuthFailure(second.error.code, second.error.message);
      }
      return fail(second.error.code, second.error.message);
    }
    if (!response.ok) return fail(response.error.code, response.error.message);
    return { ok: true, result: response.result as T };
  };

  const activeTab = async (): Promise<chrome.tabs.Tab | null> => {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    return tab ?? null;
  };

  const sendToTab = async <T>(tabId: number, message: ContentRequest): Promise<T> =>
    (await chrome.tabs.sendMessage(tabId, message)) as T;

  const ensureContentScript = async (tabId: number): Promise<void> => {
    try {
      await sendToTab(tabId, { type: "subclave:ping" });
    } catch {
      // Chromium "on click" site access, or a tab opened before install.
      await chrome.scripting.executeScript({ target: { tabId }, files: ["content.js"] });
      await sendToTab(tabId, { type: "subclave:ping" });
    }
  };

  const fillEntry = async (
    tabId: number,
    url: string,
    entryId: string,
    via: "popup" | "command" | "inline",
  ): Promise<SwResponse> => {
    const credential = await call<CredentialResult>("get-credential", { id: entryId, url, via });
    if (!credential.ok) {
      return { type: "fill", ok: false, code: credential.code, message: credential.message };
    }
    await ensureContentScript(tabId);
    const result = await sendToTab<ContentFillResult>(tabId, {
      type: "subclave:fill",
      username: credential.result.username,
      password: credential.result.password,
      url,
      anchored: via === "inline",
    });
    return { type: "fill", ok: true, filled: Number(result.username) + Number(result.password) };
  };

  const generateAndFill = async (
    tabId: number,
    url: string,
    entryId: string | null,
    via: "popup" | "command" | "inline",
  ): Promise<SwResponse> => {
    const generated = await call<GenerateResult>("generate-password");
    if (!generated.ok) {
      return { type: "generate", ok: false, code: generated.code, message: generated.message };
    }
    await ensureContentScript(tabId);
    const filled = await sendToTab<ContentGenerateResult>(tabId, {
      type: "subclave:generate-fill",
      password: generated.result.password,
      url,
      anchored: via === "inline",
    });
    if (filled.filled === 0) return { type: "generate", ok: true, saved: false, filled: 0 };

    const save = await call<SaveLoginResult>("save-login", {
      url,
      username: filled.username,
      password: generated.result.password,
      ...(entryId ? { entryId } : {}),
      via,
    });
    if (!save.ok) return { type: "generate", ok: false, code: save.code, message: save.message };
    return { type: "generate", ok: true, saved: true, filled: filled.filled };
  };

  const getState = async (url: string | undefined, scope: "host" | "all"): Promise<SwState> => {
    await loadState();
    const statusResponse = await transport.send(request("status"));
    if (!statusResponse.ok) {
      if (statusResponse.error.code === "app-not-running") return { state: "not-running" };
      return {
        state: "error",
        code: statusResponse.error.code,
        message: statusResponse.error.message,
      };
    }
    const status = statusResponse.result as StatusResult;
    if (status.locked) return { state: "locked" };
    if (!credentials) return { state: "unpaired" };

    if (!url) return { state: "error", code: "bad-request", message: "No active page" };
    const logins = await call<GetLoginsResult>("get-logins", { url, scope });
    if (!logins.ok) {
      if (logins.code === "not-associated" || logins.code === "auth-failed")
        return { state: "unpaired" };
      if (logins.code === "vault-locked") return { state: "locked" };
      return { state: "error", code: logins.code, message: logins.message };
    }
    let host = url;
    try {
      host = new URL(url).hostname;
    } catch {
      // Keep the raw URL as the label.
    }
    return {
      state: "ready",
      host,
      entries: logins.result.entries,
      otherMatches: logins.result.otherMatches,
      domain: logins.result.domain,
      showInLoginFields,
    };
  };

  // The popup shows the code while the app's dialog is still open, so it is
  // sent before the (awaited) associate response: a broadcast for the popup, a
  // message to its own tab for an inline pairing.
  const pair = async (
    send: (event: PairingCodeEvent) => Promise<unknown> = (event) =>
      chrome.runtime.sendMessage(event),
  ): Promise<SwResponse> => {
    await loadState();
    const statusResponse = await transport.send(request("status"));
    if (!statusResponse.ok) {
      return {
        type: "pair",
        ok: false,
        code: statusResponse.error.code,
        message: statusResponse.error.message,
      };
    }
    const status = statusResponse.result as StatusResult;
    if (status.locked) {
      return { type: "pair", ok: false, code: "vault-locked", message: "Subclave is locked" };
    }

    const pairNonce = randomNonce();
    const code = await pairingCode(pairNonce);
    void send({ type: PAIRING_CODE_EVENT, code }).catch(() => undefined);

    const response = await transport.send(
      request("associate", {
        browser: browserName(),
        profileName: "Default",
        pairNonce: bytesToBase64(pairNonce),
      }),
    );
    if (!response.ok) {
      return {
        type: "pair",
        ok: false,
        code: response.error.code,
        message: response.error.message,
      };
    }

    credentials = response.result as PairResult;
    authenticated = false;
    await chrome.storage.local.set({ [CREDENTIALS_KEY]: credentials });
    return { type: "pair", ok: true, code };
  };

  const runFillCommand = async (): Promise<SwResponse> => {
    await loadState();
    const tab = await activeTab();
    if (!tab || tab.id === undefined || !tab.url || !credentials) {
      return { type: "fill", ok: true, filled: 0 };
    }
    const logins = await call<GetLoginsResult>("get-logins", { url: tab.url, scope: "all" });
    if (!logins.ok || logins.result.entries.length === 0) {
      return { type: "fill", ok: true, filled: 0 };
    }
    const entries = logins.result.entries;
    if (entries.length > 1 && (await openPopup())) return { type: "fill", ok: true, filled: 0 };
    let pick = entries[0];
    if (entries.length > 1) {
      for (const entry of entries) {
        if ((entry.lastUsedAt ?? 0) > (pick.lastUsedAt ?? 0)) pick = entry;
      }
    }
    return fillEntry(tab.id, tab.url, pick.id, "command");
  };

  const handleMessage = async (message: PopupRequest): Promise<SwResponse> => {
    switch (message.type) {
      case "get-state":
        return { type: "state", state: await getState((await activeTab())?.url, "all") };
      case "pair":
        return pair();
      case "fill-entry": {
        const tab = await activeTab();
        if (tab?.id === undefined || !tab.url) {
          return { type: "fill", ok: false, code: "bad-request", message: "No active page" };
        }
        return fillEntry(tab.id, tab.url, message.entryId, message.via);
      }
      case "generate-and-fill": {
        const tab = await activeTab();
        if (tab?.id === undefined || !tab.url) {
          return { type: "generate", ok: false, code: "bad-request", message: "No active page" };
        }
        return generateAndFill(tab.id, tab.url, message.entryId, message.via);
      }
      case "settings-get":
        await loadState();
        return { type: "settings", showInLoginFields };
      case "settings-set":
        await loadState();
        showInLoginFields = message.showInLoginFields;
        await chrome.storage.local.set({ [SHOW_IN_LOGIN_FIELDS_KEY]: showInLoginFields });
        return { type: "settings", showInLoginFields };
      case "focus-app":
        return focusApp();
      case "fill-command":
        return runFillCommand();
    }
  };

  const focusApp = async (): Promise<SwResponse> => {
    const response = await transport.send(request("focus-app"));
    return { type: "focus", ok: response.ok };
  };

  // ponytail: one chain for all tabs; per-tab chains if it ever matters
  let pendingChain: Promise<unknown> = Promise.resolve();
  /** Runs `step` after every earlier one. Storage has no transactions, so each
   * read-modify-write of a pending pair is one step; native calls stay out. */
  const withPending = <T>(step: () => Promise<T>): Promise<T> => {
    const result = pendingChain.then(step);
    pendingChain = result.catch(() => undefined);
    return result;
  };

  /** Call inside a `withPending` step. */
  const readPending = async (tabId: number): Promise<PendingSave | null> => {
    const key = PENDING_SAVE_PREFIX + tabId;
    const stored = await chrome.storage.session.get(key);
    // `inline-submitted` is the only writer of this key.
    return (stored[key] as PendingSave | undefined) ?? null;
  };

  /** Drops the tab's pair only if it is still the one named `id`, so a slow
   * answer never deletes a newer sign-in. */
  const dropPending = (tabId: number, id: string): Promise<void> =>
    withPending(async () => {
      if ((await readPending(tabId))?.id === id) {
        await chrome.storage.session.remove(PENDING_SAVE_PREFIX + tabId);
      }
    });

  /** The prompt for the tab's pending sign-in on the page at `url`, counted as
   * one of its page loads. Nothing pending never connects to the app; a stored
   * pair is dropped, and a failed check (locked, not running, unpaired) or a
   * page on another site keeps it for later. Expiry is checked here only, so a
   * prompt already showing can still be answered. */
  const pendingPrompt = async (tabId: number, url: string): Promise<SwResponse> => {
    const none: SwResponse = { type: "save-prompt", prompt: null };
    const key = PENDING_SAVE_PREFIX + tabId;
    const pending = await withPending(async () => {
      const stored = await readPending(tabId);
      if (!stored) return null;
      if (stored.loads >= PENDING_SAVE_PAGE_LOADS || Date.now() - stored.at > PENDING_SAVE_TTL_MS) {
        await chrome.storage.session.remove(key);
        return null;
      }
      await chrome.storage.session.set({ [key]: { ...stored, loads: stored.loads + 1 } });
      return stored;
    });
    if (!pending) return none;
    const checked = await call<CheckLoginResult>("check-login", {
      url: pending.url,
      pageUrl: url,
      username: pending.username,
      password: pending.password,
    });
    if (!checked.ok || checked.result.state === "other-site") return none;
    if (checked.result.state === "unchanged") {
      await dropPending(tabId, pending.id);
      return none;
    }
    let host = pending.url;
    try {
      host = new URL(pending.url).hostname;
    } catch {
      // Keep the raw URL as the label.
    }
    const prompt: SavePrompt = {
      id: pending.id,
      host,
      username: pending.username,
      entries: checked.result.entries,
    };
    return { type: "save-prompt", prompt };
  };

  /** Adds the pending sign-in `id` (`entryId` null) or updates that entry with
   * it; a newer sign-in in the tab answers `no-match`. */
  const savePending = async (
    tabId: number,
    id: string,
    entryId: string | null,
  ): Promise<SwResponse> => {
    const pending = await withPending(() => readPending(tabId));
    if (pending?.id !== id) {
      return {
        type: "save",
        ok: false,
        code: "no-match",
        message: "This sign-in is no longer waiting to be saved.",
      };
    }
    const saved = await call<SaveLoginResult>("save-login", {
      url: pending.url,
      username: pending.username,
      password: pending.password,
      ...(entryId ? { entryId } : {}),
      via: "inline",
    });
    if (!saved.ok) return { type: "save", ok: false, code: saved.code, message: saved.message };
    await dropPending(tabId, id);
    return { type: "save", ok: true };
  };

  /** Every inline request is answered for the sender's own tab at
   * `scope: "host"` and `via: "inline"`. Fills and the picker use the sender's
   * URL, so they never reach another host's logins or the popup's wider match.
   * The save prompt saves to the browser-stamped URL of the tab's own submitted
   * sign-in, and shows only on a page of that sign-in's site (`check-login`
   * decides, by the vault's Domain rule). `inline-open-popup` uses neither the
   * tab nor the URL; it only opens the popup. */
  const handleInline = async (
    message: InlineRequest,
    tabId: number,
    url: string,
  ): Promise<SwResponse> => {
    switch (message.type) {
      case "inline-settings":
        await loadState();
        return { type: "settings", showInLoginFields };
      case "inline-logins":
        return { type: "state", state: await getState(url, "host") };
      case "inline-fill":
        return fillEntry(tabId, url, message.entryId, "inline");
      case "inline-generate":
        return generateAndFill(tabId, url, message.entryId, "inline");
      case "inline-pair":
        return pair((event) => sendToTab(tabId, event));
      case "inline-focus-app":
        return focusApp();
      case "inline-submitted": {
        // `url` is the browser-stamped `sender.url`; the content script never
        // names one. A new submit replaces the tab's old pair.
        const pending: PendingSave = {
          id: crypto.randomUUID(),
          url,
          username: message.username,
          password: message.password,
          at: Date.now(),
          loads: 0,
        };
        await withPending(() =>
          chrome.storage.session.set({ [PENDING_SAVE_PREFIX + tabId]: pending }),
        );
        return { type: "submitted" };
      }
      case "inline-pending-save":
        return pendingPrompt(tabId, url);
      case "inline-save":
        return savePending(tabId, message.id, message.entryId);
      case "inline-save-cancel":
        await dropPending(tabId, message.id);
        return { type: "save", ok: true };
      case "inline-open-popup": {
        if (await openPopup()) return { type: "popup", ok: true };
        const commands = await chrome.commands.getAll();
        const shortcut = commands.find((command) => command.name === "fill-login")?.shortcut;
        return { type: "popup", ok: false, shortcut: shortcut ?? "" };
      }
    }
  };

  transport.onDisconnect(() => {
    authenticated = false;
  });

  chrome.commands.onCommand.addListener((command) => {
    if (command === "fill-login") void runFillCommand();
  });

  chrome.tabs.onRemoved.addListener((tabId) => {
    void chrome.storage.session.remove(PENDING_SAVE_PREFIX + tabId);
  });

  chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    const type = runtimeMessageType(message);
    if (type === null) return false;
    const kind = senderKind(sender);
    let answer: Promise<SwResponse>;
    if (kind === "page" && POPUP_REQUEST_TYPES[type] === true) {
      answer = handleMessage(message as PopupRequest);
    } else if (
      kind === "content" &&
      INLINE_REQUEST_TYPES[type] === true &&
      sender.tab?.id !== undefined &&
      sender.url
    ) {
      answer = handleInline(message as InlineRequest, sender.tab.id, sender.url);
    } else {
      return false;
    }
    void answer.then(sendResponse).catch((error: unknown) => {
      sendResponse({ type: "error", code: "bad-request", message: String(error) });
    });
    return true;
  });
}

/**
 * Popup requests come only from extension pages; inline requests come only from
 * the top-frame content script, whose `url` the browser stamps. This makes the
 * exact-host rule structural: a content script can never ask for `scope: "all"`
 * or `via: "popup"`, and a page cannot name another page's URL.
 */
function senderKind(sender: chrome.runtime.MessageSender): "page" | "content" | null {
  if (sender.id !== chrome.runtime.id || !sender.url) return null;
  if (sender.url.startsWith(chrome.runtime.getURL(""))) return "page";
  if (sender.tab?.id !== undefined && sender.frameId === 0) return "content";
  return null;
}

/** Only object messages with a string `type` reach the router; everything else
 * (a broadcast, an unknown type, a message from the wrong kind of sender) is
 * ignored. */
function runtimeMessageType(message: unknown): string | null {
  if (message === null || typeof message !== "object" || !("type" in message)) return null;
  const type: unknown = message.type;
  return typeof type === "string" ? type : null;
}

/** Opens the toolbar popup in the focused window. `chrome.action.openPopup`
 * is open to every extension from Chrome 127 and needs no user action from
 * Firefox 149; older versions lack it or reject the call, which reads as
 * `false`. Read through a named interface because the member can be missing. */
async function openPopup(): Promise<boolean> {
  const action: { openPopup?: () => Promise<void> } = chrome.action;
  if (!action.openPopup) return false;
  try {
    await action.openPopup.call(chrome.action);
    return true;
  } catch {
    return false;
  }
}

/** Chromium's UA always says "Chrome", so the other products are checked first. */
function browserName(): string {
  const ua = navigator.userAgent;
  if (ua.includes("Firefox/")) return "Firefox";
  if (ua.includes("Edg/")) return "Edge";
  if (ua.includes("Vivaldi/")) return "Vivaldi";
  // `navigator.brave` is not in the DOM lib; named so the read is auditable.
  const extended: Navigator & { brave?: unknown } = navigator;
  if (extended.brave) return "Brave";
  return "Chrome";
}
