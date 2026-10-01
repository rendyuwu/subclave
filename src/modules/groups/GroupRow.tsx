// One row of the group tree: the expander, the group's glyph, its label and
// count, and whatever trailing actions the pane hands in. The pane owns
// selection, expansion, focus and drag state.

import { EntryGlyph } from "./AppearancePicker";
import type { TreeRow } from "./groupTreeModel";
import { cn } from "@/lib/utils";
import { ChevronRight } from "lucide-react";
import type { DragEvent, ReactNode } from "react";

export function GroupRow({
  row,
  selected,
  roving,
  dragOver,
  actions,
  onSelect,
  onToggleExpand,
  onDragOver,
  onDragLeave,
  onDrop,
}: {
  row: TreeRow;
  selected: boolean;
  roving: boolean;
  dragOver: boolean;
  actions: ReactNode;
  onSelect: () => void;
  onToggleExpand: () => void;
  onDragOver?: (event: DragEvent<HTMLDivElement>) => void;
  onDragLeave?: (event: DragEvent<HTMLDivElement>) => void;
  onDrop?: (event: DragEvent<HTMLDivElement>) => void;
}): ReactNode {
  return (
    <div
      data-tree-row={row.id}
      role="treeitem"
      aria-selected={selected}
      aria-level={row.level + 1}
      aria-expanded={row.hasChildren ? row.expanded : undefined}
      tabIndex={roving ? 0 : -1}
      onClick={onSelect}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      style={{ paddingLeft: row.level * 14 + 6 }}
      className={cn(
        "group/row focus-visible:ring-ring/40 flex cursor-pointer items-center gap-1.5 rounded-md py-1 pr-1 text-[12.5px] outline-none select-none focus-visible:ring-1",
        selected
          ? "bg-accent text-accent-foreground"
          : "text-muted-foreground hover:bg-muted/50 hover:text-foreground",
        dragOver && "bg-accent/60",
      )}
    >
      {row.hasChildren ? (
        <button
          type="button"
          tabIndex={-1}
          aria-label={row.expanded ? `Collapse ${row.label}` : `Expand ${row.label}`}
          onClick={(event) => {
            event.stopPropagation();
            onToggleExpand();
          }}
          className="text-muted-foreground flex size-4 shrink-0 items-center justify-center"
        >
          <ChevronRight
            size={12}
            strokeWidth={2.25}
            className={cn("transition-transform", row.expanded && "rotate-90")}
          />
        </button>
      ) : (
        <span className="size-4 shrink-0" />
      )}

      {row.group ? <EntryGlyph icon={row.group.icon} color={row.group.color} /> : null}

      <span className="min-w-0 flex-1 truncate">{row.label}</span>
      <span className="text-[11px] tabular-nums opacity-70">{row.count}</span>

      {actions}
    </div>
  );
}
