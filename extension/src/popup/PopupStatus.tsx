import { useEffect, useState } from "react";
import type { SwState } from "../lib/messages";
import { OUTLINE_BUTTON } from "./ui";

type Props = {
  state: SwState;
  onUnlock: () => void;
};

export function PopupStatus({ state, onUnlock }: Props) {
  if (state.state === "locked") {
    return (
      <div className="flex flex-col gap-2 p-2">
        <p>Subclave is locked</p>
        <button type="button" className={OUTLINE_BUTTON} onClick={onUnlock}>
          Unlock
        </button>
      </div>
    );
  }
  if (state.state === "not-running") {
    return (
      <p className="p-2">Subclave is not running. Start it from your applications menu.</p>
    );
  }
  if (state.state === "error") return <p className="p-2">{state.message}</p>;
  return null;
}

const GRANT_FALLBACK = "Restore site access in the browser's extensions menu (Site access).";

/**
 * A convenience row, never a gate: the popup and the command fill through
 * `activeTab` either way. Chrome cannot re-grant a required host permission on
 * every version, so a failed request falls back to instructions, and the newer
 * `addHostAccessRequest` is tried when the browser has it.
 */
export function SiteAccessNotice() {
  const [granted, setGranted] = useState(true);
  const [hint, setHint] = useState(false);

  useEffect(() => {
    let cancelled = false;
    chrome.permissions
      .contains({ origins: ["<all_urls>"] })
      .then((value) => {
        if (!cancelled) setGranted(value);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  const request = async () => {
    let grantedNow = false;
    try {
      grantedNow = await chrome.permissions.request({ origins: ["<all_urls>"] });
    } catch {
      grantedNow = false;
    }
    if (!grantedNow) {
      // Chrome 133+ moved this behind a per-tab request; guarded because older
      // browsers have no such member.
      const api: { addHostAccessRequest?: (options: { tabId: number }) => Promise<void> } =
        chrome.permissions;
      try {
        const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
        if (api.addHostAccessRequest && tab?.id !== undefined) {
          await api.addHostAccessRequest({ tabId: tab.id });
          grantedNow = true;
        }
      } catch {
        // Fall through to the instructions.
      }
    }
    setGranted(grantedNow);
    setHint(!grantedNow);
  };

  if (granted) return null;
  return (
    <div className="flex flex-col gap-1 border-t border-border p-2">
      <p className="text-muted-foreground">Subclave needs access to this page to fill it.</p>
      <button type="button" className={OUTLINE_BUTTON} onClick={() => void request()}>
        Allow on this site
      </button>
      {hint && <p className="text-muted-foreground">{GRANT_FALLBACK}</p>}
    </div>
  );
}
