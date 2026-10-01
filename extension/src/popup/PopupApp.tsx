import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { PAIRING_CODE_EVENT, sendToBackground } from "../lib/messages";
import type { SwResponse, SwState } from "../lib/messages";
import { PopupEntryList } from "./PopupEntryList";
import { PopupGenerate } from "./PopupGenerate";
import { PopupPairing } from "./PopupPairing";
import { PopupSettings } from "./PopupSettings";
import { PopupStatus, SiteAccessNotice } from "./PopupStatus";

export function PopupApp() {
  const [state, setState] = useState<SwState | null>(null);
  const [pairingCode, setPairingCode] = useState<string | null>(null);
  const [pairError, setPairError] = useState<string | null>(null);
  const [pairing, setPairing] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);

  const refresh = useCallback(async () => {
    const response = await sendToBackground<SwResponse>({ type: "get-state" });
    if (response.type === "state") setState(response.state);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (state?.state === "ready") searchRef.current?.focus();
  }, [state?.state]);

  // The service worker broadcasts the pairing code while the app's dialog is
  // open, so the popup can show it before `pair` resolves.
  useEffect(() => {
    const listener = (message: unknown) => {
      if (message === null || typeof message !== "object" || !("type" in message)) return;
      if (message.type !== PAIRING_CODE_EVENT) return;
      if ("code" in message && typeof message.code === "string") setPairingCode(message.code);
    };
    chrome.runtime.onMessage.addListener(listener);
    return () => chrome.runtime.onMessage.removeListener(listener);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") window.close();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, []);

  const entries = state?.state === "ready" ? state.entries : [];
  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return entries;
    return entries.filter((entry) =>
      `${entry.title} ${entry.username} ${entry.group}`.toLowerCase().includes(needle),
    );
  }, [entries, query]);

  useEffect(() => {
    setActiveIndex((index) => Math.min(index, Math.max(filtered.length - 1, 0)));
  }, [filtered.length]);

  const fill = useCallback(
    async (entryId: string) => {
      const response = await sendToBackground<SwResponse>({
        type: "fill-entry",
        entryId,
        via: "popup",
      });
      if (response.type !== "fill") return;
      setNotice(
        response.ok
          ? response.filled > 0
            ? "Filled"
            : "No login fields found on this page."
          : response.message,
      );
      if (!response.ok) await refresh();
    },
    [refresh],
  );

  const generate = useCallback(
    async (entryId: string | null) => {
      const response = await sendToBackground<SwResponse>({
        type: "generate-and-fill",
        entryId,
        via: "popup",
      });
      if (response.type !== "generate") return;
      if (!response.ok) setNotice(response.message);
      else if (response.filled === 0) setNotice("No login fields found on this page.");
      else if (response.saved) setNotice("Password saved to Subclave");
      await refresh();
    },
    [refresh],
  );

  const pair = useCallback(async () => {
    setPairing(true);
    setPairError(null);
    const response = await sendToBackground<SwResponse>({ type: "pair" });
    setPairing(false);
    if (response.type !== "pair") return;
    if (response.ok) {
      setPairingCode(response.code);
      await refresh();
    } else {
      setPairError(response.message);
    }
  }, [refresh]);

  const unlock = useCallback(async () => {
    await sendToBackground<SwResponse>({ type: "focus-app" });
  }, []);

  const setShowInLoginFields = useCallback(async (value: boolean) => {
    const response = await sendToBackground<SwResponse>({
      type: "settings-set",
      showInLoginFields: value,
    });
    if (response.type !== "settings") return;
    setState((current) => {
      if (!current || current.state !== "ready") return current;
      return { ...current, showInLoginFields: response.showInLoginFields };
    });
  }, []);

  const onSearchKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (filtered.length === 0) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setActiveIndex((index) => (index + 1) % filtered.length);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveIndex((index) => (index - 1 + filtered.length) % filtered.length);
    } else if (event.key === "Enter") {
      event.preventDefault();
      const entry = filtered[activeIndex];
      if (entry) void fill(entry.id);
    }
  };

  if (state === null) return <p className="p-3 text-muted-foreground">Loading...</p>;

  if (state.state === "unpaired") {
    return (
      <div className="p-1">
        <PopupPairing
          busy={pairing}
          code={pairingCode}
          error={pairError}
          onPair={() => void pair()}
        />
      </div>
    );
  }

  if (state.state === "locked" || state.state === "not-running" || state.state === "error") {
    return <PopupStatus state={state} onUnlock={() => void unlock()} />;
  }

  return (
    <div className="flex flex-col gap-1 p-2">
      {entries.length > 0 ? (
        <>
          <input
            ref={searchRef}
            type="text"
            role="combobox"
            aria-expanded="true"
            aria-controls="subclave-logins"
            aria-activedescendant={filtered[activeIndex] ? `subclave-entry-${filtered[activeIndex].id}` : undefined}
            placeholder="Search logins"
            className="w-full border border-border bg-card px-2 py-1 outline-none focus:border-primary"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={onSearchKeyDown}
          />
          {filtered.length > 0 ? (
            <PopupEntryList
              activeIndex={activeIndex}
              entries={filtered}
              onActive={setActiveIndex}
              onSelect={(entry) => void fill(entry.id)}
            />
          ) : (
            <p className="px-2 text-muted-foreground">No matching logins.</p>
          )}
        </>
      ) : (
        <p className="px-2 text-muted-foreground">No logins for {state.host}.</p>
      )}
      <PopupGenerate entries={entries} onGenerate={(entryId) => void generate(entryId)} />
      <PopupSettings checked={state.showInLoginFields} onChange={(value) => void setShowInLoginFields(value)} />
      {notice && <p className="px-2 text-muted-foreground">{notice}</p>}
      <SiteAccessNotice />
    </div>
  );
}
