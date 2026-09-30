import { UpdaterPill } from "@/modules/updater";
import { ALL_SCOPE, groupCounts } from "@/modules/vault/list/derive";
import { useVaultStore } from "@/modules/vault/store";
import { useEffect, useState } from "react";

/** One status-bar group. The hairline that separates it from the group before
 *  it is drawn by `.sb-group` in globals.css, which hides an empty group and
 *  only gives a group its divider when a non-empty one precedes it. */
function Group({ children }: { children: React.ReactNode }) {
  return <div className="sb-group flex shrink-0 items-center gap-1.5">{children}</div>;
}

/**
 * The last `locksInMs` a poll delivered, counted down locally once a second.
 * The store polls every 5 s, so without this the countdown would jump in
 * fives; the anchor is re-taken whenever a fresh value arrives.
 */
function useRemainingMs(locksInMs: number | null): number | null {
  const [anchor, setAnchor] = useState<{ at: number; ms: number } | null>(null);
  const [, setTick] = useState(0);

  useEffect(() => {
    setAnchor(locksInMs === null ? null : { at: Date.now(), ms: locksInMs });
  }, [locksInMs]);

  useEffect(() => {
    if (locksInMs === null) return;
    const id = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(id);
  }, [locksInMs]);

  if (anchor === null) return null;
  return Math.max(0, anchor.ms - (Date.now() - anchor.at));
}

function LockGroup() {
  const status = useVaultStore((s) => s.status);
  const lock = useVaultStore((s) => s.lock);
  const remaining = useRemainingMs(status && !status.locked ? status.locksInMs : null);

  if (!status) return null;
  if (status.locked) return <Group>Locked</Group>;

  if (remaining === null) {
    return (
      <Group>
        <button
          type="button"
          onClick={() => void lock()}
          className="hover:text-foreground cursor-pointer transition-colors"
        >
          Unlocked
        </button>
      </Group>
    );
  }

  const total = Math.ceil(remaining / 1000);
  const clock = `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
  return (
    <Group>
      <button
        type="button"
        onClick={() => void lock()}
        className="hover:text-foreground cursor-pointer tabular-nums transition-colors"
      >
        Unlocked, locks in {clock}
      </button>
    </Group>
  );
}

function EntryCountGroup() {
  const entries = useVaultStore((s) => s.entries);
  const groups = useVaultStore((s) => s.groups);
  const count = groupCounts(entries, groups).get(ALL_SCOPE) ?? 0;
  return <Group>{count === 1 ? "1 entry" : `${count} entries`}</Group>;
}

export function StatusBar() {
  return (
    <footer className="border-border/60 bg-card/60 flex h-8 shrink-0 items-center justify-between gap-2 border-t px-3 text-[11px]">
      <div className="flex min-w-0 flex-1 items-center gap-2">
        <LockGroup />
        <EntryCountGroup />
      </div>
      <Group>
        <UpdaterPill />
      </Group>
    </footer>
  );
}
