import { useEffect, useState } from "react";

import { Progress } from "@/components/ui/progress";
import { genStrength } from "@/modules/vault/ipc";
import type { Strength } from "@/modules/vault/types";
import { cn } from "@/lib/utils";

// The strength bar the create screen, the security section's change-password
// dialog and the entry editor all share. It holds no vault state: `value` comes
// in, `gen_strength` is asked (debounced), the score and the warning go out.
// The text label always carries the verdict, so colour is never the only signal.

const LABELS = ["Very weak", "Weak", "Fair", "Strong", "Very strong"] as const;

export function StrengthMeter({ value, className }: { value: string; className?: string }) {
  const [strength, setStrength] = useState<Strength | null>(null);

  useEffect(() => {
    if (value === "") {
      setStrength(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(() => {
      genStrength(value)
        .then((next) => {
          if (!cancelled) setStrength(next);
        })
        .catch(() => {
          if (!cancelled) setStrength(null);
        });
    }, 200);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [value]);

  const score = strength ? Math.min(4, Math.max(0, strength.score)) : 0;
  const label = strength ? LABELS[score] : "";

  // Nothing to judge before the first character.
  if (value === "") return null;

  return (
    <div className={cn("flex flex-col gap-1", className)}>
      <Progress value={(score + 1) * 20} className="h-1.5" aria-label={`Strength: ${label}`} />
      <div className="flex flex-col gap-0.5 text-[11px]">
        <span className="text-muted-foreground">{label}</span>
        {strength?.warning ? (
          <span className="text-muted-foreground">{strength.warning}</span>
        ) : null}
      </div>
    </div>
  );
}
