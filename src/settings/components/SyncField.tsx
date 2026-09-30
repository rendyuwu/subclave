import { Input } from "@/components/ui/input";
import type { ComponentProps } from "react";

/**
 * One labelled free-text field of a sync form.
 *
 * A real `label`/`id` pair rather than `aria-label`, because these fields carry
 * a description each and a screen reader that gets only the terse name loses
 * it. `SettingRow` puts its control in a shrink-0 right slot, which is right
 * for a switch and wrong for an input that wants the width, so this borrows
 * that row's chrome and stacks instead.
 *
 * SHARED BY BOTH FORMS that configure a provider, Settings > Sync and the
 * first-run join screen. The two ask the user for the same fields, and a second
 * copy would drift the moment one of them gained a field.
 */
export function SyncField({
  id,
  label,
  description,
  ...props
}: { id: string; label: string; description?: string } & ComponentProps<"input">) {
  return (
    <div className="border-border/60 bg-card flex flex-col gap-1.5 rounded-lg border px-3 py-2.5">
      <label htmlFor={id} className="text-[12.5px] font-medium">
        {label}
      </label>
      {description ? (
        <span className="text-muted-foreground text-[10.5px] leading-relaxed">{description}</span>
      ) : null}
      <Input id={id} spellCheck={false} className="h-8 rounded-lg text-[12px]" {...props} />
    </div>
  );
}
