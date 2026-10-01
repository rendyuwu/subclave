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
  PairingCodeEvent,
  PopupRequest,
  SwResponse,
  SwState,
} from "./lib/messages";
import { PAIRING_CODE_EVENT } from "./lib/messages";
import { PROTOCOL_VERSION } from "./lib/protocol";
import type {
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

type Credentials = { clientId: string; secret: string };
type CallResult<T> = { ok: true; result: T } | { ok: false; code: NmErrorCode; message: string };

/** Static lookup for the message types the service worker answers. */
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
    tab: chrome.tabs.Tab,
    entryId: string,
    via: "popup" | "command",
  ): Promise<SwResponse> => {
    if (tab.id === undefined || !tab.url) {
      return { type: "fill", ok: false, code: "bad-request", message: "No active page" };
    }
    const credential = await call<CredentialResult>("get-credential", {
      id: entryId,
      url: tab.url,
      via,
    });
    if (!credential.ok) {
      return { type: "fill", ok: false, code: credential.code, message: credential.message };
    }
    await ensureContentScript(tab.id);
    const result = await sendToTab<ContentFillResult>(tab.id, {
      type: "subclave:fill",
      username: credential.result.username,
      password: credential.result.password,
    });
    return { type: "fill", ok: true, filled: Number(result.username) + Number(result.password) };
  };

  const generateAndFill = async (
    tab: chrome.tabs.Tab,
    entryId: string | null,
    via: "popup" | "command",
  ): Promise<SwResponse> => {
    if (tab.id === undefined || !tab.url) {
      return { type: "generate", ok: false, code: "bad-request", message: "No active page" };
    }
    const generated = await call<GenerateResult>("generate-password");
    if (!generated.ok) {
      return { type: "generate", ok: false, code: generated.code, message: generated.message };
    }
    await ensureContentScript(tab.id);
    const filled = await sendToTab<ContentGenerateResult>(tab.id, {
      type: "subclave:generate-fill",
      password: generated.result.password,
    });
    if (filled.filled === 0) return { type: "generate", ok: true, saved: false, filled: 0 };

    const save = await call<SaveLoginResult>("save-login", {
      url: tab.url,
      username: filled.username,
      password: generated.result.password,
      ...(entryId ? { entryId } : {}),
      via,
    });
    if (!save.ok) return { type: "generate", ok: false, code: save.code, message: save.message };
    return { type: "generate", ok: true, saved: true, filled: filled.filled };
  };

  const getState = async (): Promise<SwState> => {
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

    const tab = await activeTab();
    if (!tab?.url) return { state: "error", code: "bad-request", message: "No active page" };
    const logins = await call<GetLoginsResult>("get-logins", { url: tab.url, scope: "all" });
    if (!logins.ok) {
      if (logins.code === "not-associated" || logins.code === "auth-failed")
        return { state: "unpaired" };
      if (logins.code === "vault-locked") return { state: "locked" };
      return { state: "error", code: logins.code, message: logins.message };
    }
    let host = tab.url;
    try {
      host = new URL(tab.url).hostname;
    } catch {
      // Keep the raw URL as the label.
    }
    return {
      state: "ready",
      host,
      entries: logins.result.entries,
      showInLoginFields,
    };
  };

  const pair = async (): Promise<SwResponse> => {
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
    // The popup shows the code while the app's dialog is still open, so it is
    // broadcast before the (awaited) associate response.
    const event: PairingCodeEvent = { type: PAIRING_CODE_EVENT, code };
    void chrome.runtime.sendMessage(event).catch(() => undefined);

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
    // `chrome.action.openPopup` is Chrome 127+; Firefox and older Chrome have
    // no such member, so it is read through a named interface.
    const action: { openPopup?: () => Promise<void> } = chrome.action;
    if (entries.length > 1 && action.openPopup) {
      try {
        await action.openPopup.call(chrome.action);
        return { type: "fill", ok: true, filled: 0 };
      } catch {
        // Firefox and older Chrome reject openPopup; fall through to the
        // newest-match fallback below.
      }
    }
    let pick = entries[0];
    if (entries.length > 1) {
      for (const entry of entries) {
        if ((entry.lastUsedAt ?? 0) > (pick.lastUsedAt ?? 0)) pick = entry;
      }
    }
    return fillEntry(tab, pick.id, "command");
  };

  const handleMessage = async (message: PopupRequest): Promise<SwResponse> => {
    switch (message.type) {
      case "get-state":
        return { type: "state", state: await getState() };
      case "pair":
        return pair();
      case "fill-entry": {
        const tab = await activeTab();
        if (tab?.id === undefined) {
          return { type: "fill", ok: false, code: "bad-request", message: "No active page" };
        }
        return fillEntry(tab, message.entryId, message.via);
      }
      case "generate-and-fill": {
        const tab = await activeTab();
        if (tab?.id === undefined) {
          return { type: "generate", ok: false, code: "bad-request", message: "No active page" };
        }
        return generateAndFill(tab, message.entryId, message.via);
      }
      case "settings-get":
        await loadState();
        return { type: "settings", showInLoginFields };
      case "settings-set":
        await loadState();
        showInLoginFields = message.showInLoginFields;
        await chrome.storage.local.set({ [SHOW_IN_LOGIN_FIELDS_KEY]: showInLoginFields });
        return { type: "settings", showInLoginFields };
      case "focus-app": {
        const response = await transport.send(request("focus-app"));
        return { type: "focus", ok: response.ok };
      }
      case "fill-command":
        return runFillCommand();
    }
  };

  transport.onDisconnect(() => {
    authenticated = false;
  });

  chrome.commands.onCommand.addListener((command) => {
    if (command === "fill-login") void runFillCommand();
  });

  chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    const type = runtimeMessageType(message);
    if (type === null || POPUP_REQUEST_TYPES[type] !== true) return false;
    void handleMessage(message as PopupRequest)
      .then(sendResponse)
      .catch((error: unknown) => {
        sendResponse({ type: "error", code: "bad-request", message: String(error) });
      });
    return true;
  });
}

/** Only object messages with a string `type` reach the router; everything else
 * (a broadcast, a content-script message, an unknown sender) is ignored. */
function runtimeMessageType(message: unknown): string | null {
  if (message === null || typeof message !== "object" || !("type" in message)) return null;
  const type: unknown = message.type;
  return typeof type === "string" ? type : null;
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
