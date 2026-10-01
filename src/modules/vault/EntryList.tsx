import { useMemo, useRef, type KeyboardEvent, type ReactNode } from "react";
import { SquarePlus } from "lucide-react";

import { Button } from "@/components/ui/button";

import { defaultGroupForScope } from "./commands";
import { EntryRow } from "./EntryRow";
import { ALL_SCOPE, FAVORITES_SCOPE, TRASH_SCOPE, visibleEntries } from "./list/derive";
import { useVaultStore } from "./store";

/** Centered empty-state panel with an optional action button. */
function EmptyState({ message, action }: { message: string; action?: ReactNode }): ReactNode {
  return (
    <div className="text-muted-foreground flex flex-col items-center justify-center gap-3 px-4 py-12 text-center text-xs">
      <p>{message}</p>
      {action}
    </div>
  );
}

export function EntryList(): ReactNode {
  const entries = useVaultStore((s) => s.entries);
  const groups = useVaultStore((s) => s.groups);
  const scope = useVaultStore((s) => s.scope);
  const tagFilter = useVaultStore((s) => s.tagFilter);
  const searchIds = useVaultStore((s) => s.searchIds);
  const query = useVaultStore((s) => s.query);
  const selectedId = useVaultStore((s) => s.selectedId);
  const selectEntry = useVaultStore((s) => s.selectEntry);
  const openEditor = useVaultStore((s) => s.openEditor);
  const setQuery = useVaultStore((s) => s.setQuery);

  const rows = useMemo(
    () =>
      visibleEntries({
        entries,
        scope,
        tagFilter,
        searchIds,
        now: Date.now(),
      }),
    [entries, scope, tagFilter, searchIds],
  );

  const rowRefs = useRef(new Map<string, HTMLDivElement>());
  const tabStopId =
    selectedId !== null && rows.some((row) => row.id === selectedId)
      ? selectedId
      : (rows[0]?.id ?? null);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (rows.length === 0) return;
    const current = selectedId === null ? -1 : rows.findIndex((row) => row.id === selectedId);
    let next = -1;
    if (e.key === "ArrowDown") next = current < 0 ? 0 : Math.min(rows.length - 1, current + 1);
    else if (e.key === "ArrowUp") next = current < 0 ? 0 : Math.max(0, current - 1);
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = rows.length - 1;
    else return;
    e.preventDefault();
    const id = rows[next].id;
    selectEntry(id);
    rowRefs.current.get(id)?.focus();
  };

  const newAtRoot = () => openEditor(null, defaultGroupForScope(scope));

  let empty: ReactNode = null;
  if (rows.length === 0) {
    if (entries.length === 0) {
      empty = (
        <EmptyState
          message="Your vault is empty."
          action={
            <Button variant="outline" size="sm" onClick={newAtRoot}>
              <SquarePlus strokeWidth={1.75} />
              New entry
            </Button>
          }
        />
      );
    } else if (query.trim().length > 0) {
      empty = (
        <EmptyState
          message={`No entries match '${query}'.`}
          action={
            <Button variant="outline" size="sm" onClick={() => void setQuery("")}>
              Clear search
            </Button>
          }
        />
      );
    } else if (scope === TRASH_SCOPE) {
      empty = <EmptyState message="Trash is empty." />;
    } else {
      const name =
        scope === ALL_SCOPE
          ? "All"
          : scope === FAVORITES_SCOPE
            ? "Favorites"
            : (groups.find((group) => group.id === scope)?.name ?? scope);
      empty = (
        <EmptyState
          message={`Nothing in ${name}.`}
          action={
            <Button variant="outline" size="sm" onClick={newAtRoot}>
              New entry here
            </Button>
          }
        />
      );
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="border-border/60 flex h-9 shrink-0 items-center justify-between gap-2 border-b px-2">
        <span className="text-muted-foreground text-xs tabular-nums">
          {rows.length} {rows.length === 1 ? "entry" : "entries"}
        </span>
        {scope !== TRASH_SCOPE ? (
          <Button variant="ghost" size="xs" onClick={newAtRoot}>
            <SquarePlus strokeWidth={1.75} />
            New entry
          </Button>
        ) : null}
      </div>
      <div
        role="listbox"
        aria-label="Entries"
        data-vault-list
        onKeyDown={onKeyDown}
        className="min-h-0 flex-1 overflow-y-auto p-1"
      >
        {empty ??
          rows.map((entry) => (
            <EntryRow
              key={entry.id}
              entry={entry}
              groups={groups}
              selected={entry.id === selectedId}
              tabStop={entry.id === tabStopId}
              onSelect={() => selectEntry(entry.id)}
              rowRef={(el) => {
                if (el) rowRefs.current.set(entry.id, el);
                else rowRefs.current.delete(entry.id);
              }}
            />
          ))}
      </div>
    </div>
  );
}
