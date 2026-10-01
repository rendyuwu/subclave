import { IDLE_TIMEOUT_MS, NATIVE_HOST, PROTOCOL_VERSION } from "./protocol";
import type { NmRequest, NmResponse } from "./protocol";

/**
 * Everything the service worker needs from the native host. The Playwright
 * harness installs its own implementation of this, so the service worker never
 * touches `chrome.runtime.connectNative` except through
 * `createNativeTransport`.
 */
export type Transport = {
  send(request: NmRequest): Promise<NmResponse>;
  onDisconnect(callback: () => void): void;
  close(): void;
};

function notRunning(id: string): NmResponse {
  return {
    v: PROTOCOL_VERSION,
    id,
    ok: false,
    error: { code: "app-not-running", message: "Subclave is not running" },
  };
}

/**
 * A port over `chrome.runtime.connectNative`. Chrome owns the wire framing: it
 * writes the 4-byte native-endian length prefix and the JSON the proxy and the
 * app's socket server read (`src-tauri/subclave-proxy/src/frame.rs`), and it
 * delivers the host's framed JSON back as a parsed object, so this side passes
 * objects, never bytes.
 */
export function createNativeTransport(): Transport {
  let port: chrome.runtime.Port | null = null;
  const pending = new Map<string, (response: NmResponse) => void>();
  const disconnects = new Set<() => void>();
  let idleGeneration = 0;

  const cancelIdle = () => {
    idleGeneration += 1;
  };

  // A generation counter instead of a stored timer handle: the handle's type is
  // `number` under the DOM lib and a `Timeout` object under @types/node, and
  // only the newest generation may close the port.
  const scheduleIdleClose = () => {
    const generation = (idleGeneration += 1);
    void new Promise<void>((resolve) => {
      setTimeout(resolve, IDLE_TIMEOUT_MS);
    }).then(() => {
      if (generation === idleGeneration) close();
    });
  };

  const failPending = () => {
    for (const [id, resolve] of pending) resolve(notRunning(id));
    pending.clear();
  };

  const handleMessage = (message: unknown) => {
    const response = message as NmResponse | null;
    if (!response || typeof response.id !== "string") return;
    const resolve = pending.get(response.id);
    if (!resolve) return;
    pending.delete(response.id);
    resolve(response);
  };

  const close = () => {
    cancelIdle();
    const current = port;
    port = null;
    if (current) {
      try {
        current.disconnect();
      } catch {
        // Already gone.
      }
    }
    failPending();
  };

  const connect = (): chrome.runtime.Port | null => {
    if (port) return port;
    let created: chrome.runtime.Port;
    try {
      created = chrome.runtime.connectNative(NATIVE_HOST);
    } catch {
      // No native messaging host installed for this browser.
      return null;
    }
    created.onMessage.addListener(handleMessage);
    created.onDisconnect.addListener(() => {
      if (port !== created) return;
      port = null;
      cancelIdle();
      failPending();
      for (const callback of disconnects) callback();
    });
    port = created;
    return created;
  };

  return {
    send(request) {
      return new Promise<NmResponse>((resolve) => {
        const active = connect();
        if (!active) {
          resolve(notRunning(request.id));
          return;
        }
        pending.set(request.id, resolve);
        try {
          active.postMessage(request);
        } catch {
          pending.delete(request.id);
          resolve(notRunning(request.id));
          return;
        }
        scheduleIdleClose();
      });
    },
    onDisconnect(callback) {
      disconnects.add(callback);
    },
    close,
  };
}
