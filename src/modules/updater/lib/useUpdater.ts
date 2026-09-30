import { IS_LINUX } from "@/lib/platform";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { useCallback, useEffect, useRef, useState } from "react";

export const GITHUB_REPO = "rendyuwu/subclave";

export interface ManualUpdateInfo {
  version: string;
  currentVersion: string;
  notes: string | null;
  releaseUrl: string;
}

export type UpdaterState =
  | { kind: "idle" }
  | { kind: "checking" }
  | {
      kind: "available";
      version: string;
      currentVersion: string;
      notes: string | null;
      date: string | null;
    }
  | {
      kind: "manual-available";
      version: string;
      currentVersion: string;
      notes: string | null;
      releaseUrl: string;
    }
  | {
      kind: "downloading";
      version: string;
      received: number;
      total: number | null;
    }
  | { kind: "ready"; version: string }
  | { kind: "error"; message: string };

const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

/** Linux manual update flow. The bundler can't apply deb/rpm in-place, so we
 *  surface the latest release the updater plugin reports, and the user installs
 *  it through their package manager. Returns null when on the latest.
 *
 *  The check goes through the updater plugin (Rust), never a webview `fetch`:
 *  the CSP allows only IPC, so the check runs in the Rust updater plugin. */
export async function fetchLinuxRelease(): Promise<ManualUpdateInfo | null> {
  const update = await check();
  if (!update) return null;
  const info: ManualUpdateInfo = {
    version: update.version,
    currentVersion: update.currentVersion,
    notes: update.body ?? null,
    releaseUrl: `https://github.com/${GITHUB_REPO}/releases/tag/v${update.version}`,
  };
  await update.close();
  return info;
}

export function useUpdater() {
  const [state, setState] = useState<UpdaterState>({ kind: "idle" });
  const updateRef = useRef<Update | null>(null);

  const reset = useCallback(() => {
    updateRef.current = null;
    setState({ kind: "idle" });
  }, []);

  // `silent` checks are the unattended background sweeps (first-run + 6h
  // interval). When GitHub is unreachable (offline at launch, proxy, DNS) they
  // must NOT light up the red "Update check failed" pill - a failed reachability
  // probe is not news the user asked for. Only explicit checks (the dialog's
  // Retry button) surface the error.
  //
  // Silent sweeps also stay invisible mid-flight: they skip the "checking"
  // panel and, on failure, leave the current state untouched. So an error the
  // user explicitly surfaced neither self-erases nor flips to a false "up to
  // date" on a still-failing retry; only a definitive success commits state.
  const checkForUpdate = useCallback(async (opts?: { silent?: boolean }): Promise<boolean> => {
    const silent = opts?.silent ?? false;
    if (!silent) setState({ kind: "checking" });
    try {
      if (IS_LINUX) {
        const info = await fetchLinuxRelease();
        if (!info) {
          setState({ kind: "idle" });
          return false;
        }
        updateRef.current = null;
        setState({
          kind: "manual-available",
          version: info.version,
          currentVersion: info.currentVersion,
          notes: info.notes,
          releaseUrl: info.releaseUrl,
        });
        return true;
      }
      const update = await check();
      if (!update) {
        setState({ kind: "idle" });
        return false;
      }
      updateRef.current = update;
      setState({
        kind: "available",
        version: update.version,
        currentVersion: update.currentVersion,
        notes: update.body ?? null,
        date: update.date ?? null,
      });
      return true;
    } catch (e) {
      // Silent sweep failed: leave whatever is showing as-is (idle stays idle,
      // an explicitly-surfaced error stays put) instead of clobbering it.
      if (silent) return false;
      setState({ kind: "error", message: stringifyError(e) });
      return false;
    }
  }, []);

  const downloadAndInstall = useCallback(async () => {
    const update = updateRef.current;
    if (!update) return;
    let received = 0;
    let total: number | null = null;
    setState({ kind: "downloading", version: update.version, received: 0, total: null });
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started") {
          total = event.data.contentLength ?? null;
          setState({
            kind: "downloading",
            version: update.version,
            received: 0,
            total,
          });
        } else if (event.event === "Progress") {
          received += event.data.chunkLength;
          setState({
            kind: "downloading",
            version: update.version,
            received,
            total,
          });
        } else if (event.event === "Finished") {
          setState({ kind: "ready", version: update.version });
        }
      });
    } catch (e) {
      setState({ kind: "error", message: stringifyError(e) });
    }
  }, []);

  const relaunchApp = useCallback(async () => {
    try {
      await relaunch();
    } catch (e) {
      setState({ kind: "error", message: stringifyError(e) });
    }
  }, []);

  const stateKindRef = useRef(state.kind);
  stateKindRef.current = state.kind;

  // First check 8s after mount so it doesn't compete with first paint. One-shot.
  useEffect(() => {
    const first = window.setTimeout(() => {
      if (stateKindRef.current === "idle") {
        void checkForUpdate({ silent: true });
      }
    }, 8_000);
    return () => window.clearTimeout(first);
  }, [checkForUpdate]);

  useEffect(() => {
    const interval = window.setInterval(() => {
      const k = stateKindRef.current;
      if (k === "idle" || k === "error") {
        void checkForUpdate({ silent: true });
      }
    }, CHECK_INTERVAL_MS);
    return () => window.clearInterval(interval);
  }, [checkForUpdate]);

  return {
    state,
    checkForUpdate,
    downloadAndInstall,
    relaunchApp,
    reset,
  };
}

function stringifyError(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
