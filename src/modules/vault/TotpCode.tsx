import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { groupDigits } from "@/lib/format";
import { totpCode } from "./ipc";
import type { TotpCode as TotpCodeValue } from "./types";

const RING_RADIUS = 8;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

/**
 * The live TOTP code for one entry: the grouped digits, a ring that drains over
 * the current period and the seconds left as text, so the countdown never
 * depends on colour alone. Refetches when the code expires.
 */
export function TotpCode({ id }: { id: string }): ReactNode {
  const [value, setValue] = useState<TotpCodeValue | null>(null);
  const [expiresAt, setExpiresAt] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const loading = useRef(false);

  const load = useCallback(async () => {
    if (loading.current) return;
    loading.current = true;
    try {
      const next = await totpCode(id);
      setValue(next);
      setExpiresAt(Date.now() + next.remaining * 1000);
    } catch {
      setValue(null);
      setExpiresAt(0);
    } finally {
      loading.current = false;
    }
  }, [id]);

  useEffect(() => {
    setValue(null);
    setExpiresAt(0);
    void load();
  }, [id, load]);

  useEffect(() => {
    const tick = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(tick);
  }, []);

  useEffect(() => {
    if (expiresAt > 0 && now >= expiresAt) void load();
  }, [now, expiresAt, load]);

  if (!value) return null;

  const remaining = expiresAt > now ? Math.max(0, Math.ceil((expiresAt - now) / 1000)) : 0;
  const fraction = value.period > 0 ? Math.min(1, remaining / value.period) : 0;
  const offset = RING_CIRCUMFERENCE * (1 - fraction);

  return (
    <div className="flex items-center gap-2">
      <span className="font-mono text-sm tracking-wider tabular-nums">
        {groupDigits(value.code)}
      </span>
      <span className="relative inline-flex size-5 items-center justify-center">
        <svg viewBox="0 0 20 20" className="absolute size-5 -rotate-90" aria-hidden="true">
          <circle
            cx="10"
            cy="10"
            r={RING_RADIUS}
            fill="none"
            strokeWidth="2"
            className="stroke-border"
          />
          <circle
            cx="10"
            cy="10"
            r={RING_RADIUS}
            fill="none"
            strokeWidth="2"
            strokeDasharray={RING_CIRCUMFERENCE}
            strokeDashoffset={offset}
            className="stroke-primary transition-[stroke-dashoffset] duration-200 ease-linear"
          />
        </svg>
      </span>
      <span className="text-muted-foreground text-xs tabular-nums">{remaining}s</span>
    </div>
  );
}
