// A pill toggle, shared by the tag strip and any other count-carrying filter.

import { cn } from "@/lib/utils";

/** A pill toggle: label, an optional tabular-nums count, pressed state. */
export function Chip({
  label,
  count,
  selected,
  onClick,
}: {
  label: string;
  count?: number;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium transition-colors",
        selected
          ? "bg-accent text-accent-foreground border-transparent"
          : "border-border text-muted-foreground hover:bg-muted/50",
      )}
    >
      {label}
      {count !== undefined ? (
        <span className={cn("tabular-nums", selected ? "opacity-80" : "text-muted-foreground/70")}>
          {count}
        </span>
      ) : null}
    </button>
  );
}
