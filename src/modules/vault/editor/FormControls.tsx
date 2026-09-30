import type { ReactNode } from "react";

// The two presentational primitives the entry and group editors both need: the
// label wrapper every row uses, and the segmented button behind the URL match
// toggle and the generator's character sets.

/** A label above one control. */
export function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-muted-foreground text-[11px] font-medium tracking-tight">{label}</span>
      {children}
    </div>
  );
}

/**
 * One segment of a small exclusive group. `disabled` (optional, default false)
 * is for a choice the current selection makes unavailable rather than one that
 * never applies.
 */
export function ToggleButton({
  active,
  onClick,
  disabled,
  title,
  children,
}: {
  active: boolean;
  onClick: () => void;
  disabled?: boolean;
  /** Hover name for a segment whose only visible content is an icon. */
  title?: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      disabled={disabled}
      title={title}
      className={
        "flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-[11.5px] transition-colors " +
        (disabled
          ? "border-border/40 text-muted-foreground cursor-not-allowed bg-transparent opacity-60"
          : "cursor-pointer " +
            (active
              ? "border-accent bg-accent/60"
              : "border-border/60 hover:bg-accent/30 bg-transparent"))
      }
    >
      {children}
    </button>
  );
}
