import { appProof, base64ToBytes, bytesToBase64, verifyExtProof } from "../../src/lib/auth";
import type { Bytes } from "../../src/lib/auth";
import type { LoginSummary, NmErrorCode, NmRequest, NmResponse } from "../../src/lib/protocol";
import type { Transport } from "../../src/lib/transport";

// The fake app behind the E2E service worker. It speaks the same wire protocol
// as `src-tauri/src/modules/browser/` over an in-extension channel, and it keeps
// everything the specs assert on in `chrome.storage.local`, never in module
// state: an MV3 service worker is suspended after ~30s idle, so a restarted
// worker must still find its mode, its call log and its connection count.

export const MODE_KEY = "subclaveE2E.mode";
export const LOG_KEY = "subclaveE2E.log";
export const CONNECTS_KEY = "subclaveE2E.connects";
export const CREDENTIALS_KEY = "subclave.credentials";
/** How many authenticated actions to answer `not-associated` before honouring
 * them, simulating `close_all` dropping the socket while the port lives on. */
export const DROP_AUTH_KEY = "subclaveE2E.dropAuth";
/** Milliseconds `get-credential` waits before answering, so a spec can
 * navigate the tab between a pick and its fill. */
export const CREDENTIAL_DELAY_KEY = "subclaveE2E.credentialDelay";

export type FakeMode = "locked" | "unpaired" | "ok" | "absent" | "no-fields";

const MODES: Record<string, true> = {
  locked: true,
  unpaired: true,
  ok: true,
  absent: true,
  "no-fields": true,
};

export const FAKE_CLIENT_ID = "e2e-client";
export const FAKE_SECRET_BYTES: Bytes = new Uint8Array(32).fill(1);
export const FAKE_SECRET_B64 = bytesToBase64(FAKE_SECRET_BYTES);
export const FAKE_PASSWORD = "s3cret";
export const FAKE_GENERATED = "generated-1";

export const FAKE_DOMAIN = "github.com";

export const FAKE_ENTRIES: LoginSummary[] = [
  { id: "entry-1", title: "Example Account", username: "alice", group: "Root", lastUsedAt: 100 },
  { id: "entry-2", title: "Example Work", username: "bob", group: "Work", lastUsedAt: 200 },
];

export type LogEntry = {
  action: string;
  params: Record<string, unknown> | null;
  ok: boolean;
  code?: NmErrorCode;
  result?: Record<string, unknown>;
};

function plainRecord(value: unknown): Record<string, unknown> {
  // `result` only ever holds the fake app's own plain objects.
  return value !== null && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function stringParam(params: Record<string, unknown> | null, key: string): string | null {
  if (!params || !(key in params)) return null;
  const value: unknown = params[key];
  return typeof value === "string" ? value : null;
}

async function readMode(): Promise<FakeMode> {
  const stored = await chrome.storage.local.get(MODE_KEY);
  const raw: unknown = stored[MODE_KEY];
  // Validated against MODES before the cast, so a stale or hand-written key
  // cannot steer the fake app somewhere the union does not describe.
  return typeof raw === "string" && MODES[raw] === true ? (raw as FakeMode) : "ok";
}

async function readNumber(key: string): Promise<number> {
  const stored = await chrome.storage.local.get(key);
  const raw: unknown = stored[key];
  return typeof raw === "number" ? raw : 0;
}

/** The actions the app only serves on an authenticated connection. */
const AUTHED_ACTIONS: Record<string, true> = {
  "get-logins": true,
  "get-credential": true,
  "save-login": true,
  "generate-password": true,
};

/** Consume one queued "the app replaced the connection" answer. */
async function consumeDropAuth(): Promise<boolean> {
  const remaining = await readNumber(DROP_AUTH_KEY);
  if (remaining <= 0) return false;
  await chrome.storage.local.set({ [DROP_AUTH_KEY]: remaining - 1 });
  return true;
}

async function appendLog(entry: LogEntry): Promise<void> {
  const stored = await chrome.storage.local.get(LOG_KEY);
  const existing: unknown = stored[LOG_KEY];
  const log: LogEntry[] = Array.isArray(existing) ? (existing as LogEntry[]) : [];
  log.push(entry);
  await chrome.storage.local.set({ [LOG_KEY]: log });
}

export function createFakeTransport(): Transport {
  let connected = false;
  let appNonce: Bytes | null = null;
  let extNonce: Bytes | null = null;
  const disconnects = new Set<() => void>();
  // One request in flight at a time, so the read-modify-write of the log and the
  // connect counter cannot interleave.
  let queue: Promise<void> = Promise.resolve();

  const serial = <T>(task: () => Promise<T>): Promise<T> => {
    const result = queue.then(task);
    queue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  };

  const registerConnection = async (): Promise<void> => {
    if (connected) return;
    connected = true;
    await chrome.storage.local.set({ [CONNECTS_KEY]: (await readNumber(CONNECTS_KEY)) + 1 });
  };

  const answer = async (mode: FakeMode, request: NmRequest): Promise<NmResponse> => {
    const params: Record<string, unknown> | null = plainRecord(request.params);
    const ok = (result: unknown): NmResponse => ({ v: 1, id: request.id, ok: true, result });
    const error = (code: NmErrorCode, message: string): NmResponse => ({
      v: 1,
      id: request.id,
      ok: false,
      error: { code, message },
    });

    if (mode === "absent") return error("app-not-running", "Subclave is not running");
    if (mode === "locked" && request.action !== "status" && request.action !== "focus-app") {
      return error("vault-locked", "Subclave is locked");
    }
    // Simulates `close_all`: the app dropped the socket on lock (or restart)
    // but the native port stayed up, so the next authenticated action lands on
    // a fresh, un-greeted connection.
    if (AUTHED_ACTIONS[request.action] === true && (await consumeDropAuth())) {
      return error("not-associated", "Unknown client");
    }

    switch (request.action) {
      case "status":
        return ok({
          locked: mode === "locked",
          appVersion: "0.1.0-e2e",
          protocol: 1,
        });
      case "focus-app":
        return ok({});
      case "associate":
        return ok({ clientId: FAKE_CLIENT_ID, secret: FAKE_SECRET_B64 });
      case "hello": {
        if (mode === "unpaired") return error("not-associated", "Unknown client");
        const requested = stringParam(params, "extNonce");
        if (!requested) return error("bad-request", "Missing extNonce");
        extNonce = base64ToBytes(requested);
        appNonce = new Uint8Array(32).fill(3);
        const proof = await appProof(FAKE_SECRET_BYTES, extNonce, appNonce);
        return ok({ appNonce: bytesToBase64(appNonce), appProof: bytesToBase64(proof) });
      }
      case "auth": {
        if (!appNonce || !extNonce) return error("bad-request", "No hello first");
        const proof = stringParam(params, "extProof");
        if (!proof) return error("bad-request", "Missing extProof");
        const valid = await verifyExtProof(
          FAKE_SECRET_BYTES,
          appNonce,
          extNonce,
          base64ToBytes(proof),
        );
        return valid ? ok({}) : error("auth-failed", "Bad proof");
      }
      case "get-logins": {
        const hostScope = stringParam(params, "scope") === "host";
        if (mode === "no-fields") {
          return ok({ entries: [], otherMatches: hostScope ? 1 : 0, domain: FAKE_DOMAIN });
        }
        return ok({ entries: FAKE_ENTRIES, otherMatches: hostScope ? 2 : 0, domain: FAKE_DOMAIN });
      }
      case "get-credential": {
        const delay = await readNumber(CREDENTIAL_DELAY_KEY);
        if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
        const id = stringParam(params, "id");
        const entry = FAKE_ENTRIES.find((candidate) => candidate.id === id) ?? FAKE_ENTRIES[0];
        return ok({ username: entry.username, password: FAKE_PASSWORD });
      }
      case "save-login": {
        const entryId = stringParam(params, "entryId");
        return ok({ id: entryId ?? "entry-saved", created: entryId === null });
      }
      case "generate-password":
        return ok({ password: FAKE_GENERATED });
      default:
        return error("bad-request", `Unknown action ${request.action}`);
    }
  };

  return {
    send(request) {
      return serial(async () => {
        await registerConnection();
        const mode = await readMode();
        const response = await answer(mode, request);
        const entry: LogEntry = {
          action: request.action,
          params: plainRecord(request.params),
          ok: response.ok,
        };
        if (response.ok) entry.result = plainRecord(response.result);
        else entry.code = response.error.code;
        await appendLog(entry);
        return response;
      });
    },
    onDisconnect(callback) {
      disconnects.add(callback);
    },
    close() {
      connected = false;
      for (const callback of disconnects) callback();
    },
  };
}
