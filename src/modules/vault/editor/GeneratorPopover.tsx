import { useEffect, useRef, useState } from "react";
import { RefreshCw, Sparkles } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Slider } from "@/components/ui/slider";
import { genPassword } from "@/modules/vault/ipc";
import { setGenerator } from "@/modules/settings/mutations";
import { usePreferencesStore } from "@/modules/settings/preferences";
import type { GeneratorOptions } from "@/modules/vault/types";
import { ToggleButton } from "./FormControls";

// The password generator behind the editor's password row. The options are the
// persisted `generator` preference, seeded here and written when a password is
// taken through Use, or when the popover closes after a Regenerate, so dragging
// the length slider does not write the settings file on every frame (and the
// regeneration is debounced for the same reason).

type SetKey = "lower" | "upper" | "digits" | "symbols";

const SETS: { key: SetKey; label: string }[] = [
  { key: "lower", label: "a-z" },
  { key: "upper", label: "A-Z" },
  { key: "digits", label: "0-9" },
  { key: "symbols", label: "!@#" },
];

function hasSet(options: GeneratorOptions): boolean {
  return options.lower || options.upper || options.digits || options.symbols;
}

export function GeneratorPopover({ onUse }: { onUse: (password: string) => void }) {
  const saved = usePreferencesStore((state) => state.generator);
  const [open, setOpen] = useState(false);
  const [options, setOptions] = useState<GeneratorOptions>(saved);
  const [output, setOutput] = useState("");
  // True once Regenerate has run against the current options, which is what
  // makes closing the popover write them back (Use writes them at once).
  const touchedRef = useRef(false);
  const regenTimer = useRef<number | null>(null);

  useEffect(() => {
    if (!open) setOptions(saved);
  }, [saved, open]);

  useEffect(() => {
    return () => {
      if (regenTimer.current !== null) window.clearTimeout(regenTimer.current);
    };
  }, []);

  const setsOff = !hasSet(options);

  async function regenerate(next: GeneratorOptions = options): Promise<void> {
    if (!hasSet(next)) {
      setOutput("");
      return;
    }
    try {
      setOutput(await genPassword(next));
    } catch {
      setOutput("");
    }
  }

  /** Repaint the output after an option change, without one IPC call per frame. */
  function scheduleRegenerate(next: GeneratorOptions): void {
    if (regenTimer.current !== null) window.clearTimeout(regenTimer.current);
    regenTimer.current = window.setTimeout(() => void regenerate(next), 150);
  }

  function change(patch: Partial<GeneratorOptions>): void {
    // Moving the slider does not mark the options touched: they are only worth
    // writing once a password was actually taken from them.
    const next = { ...options, ...patch };
    setOptions(next);
    scheduleRegenerate(next);
  }

  function toggleSet(key: SetKey): void {
    const patch: Partial<GeneratorOptions> = { [key]: !options[key] };
    change(patch);
  }

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (!next && touchedRef.current) {
          void setGenerator(options);
          touchedRef.current = false;
        }
        if (next) void regenerate();
        setOpen(next);
      }}
    >
      <PopoverTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label="Generate password"
          title="Generate password"
          className="text-muted-foreground hover:text-foreground"
        >
          <Sparkles />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" sideOffset={6} className="w-72 gap-3 p-3">
        <div className="flex flex-col gap-1.5">
          <div className="flex items-center justify-between text-[11px]">
            <span className="text-muted-foreground font-medium">Length</span>
            <span className="tabular-nums">{options.length}</span>
          </div>
          <Slider
            min={8}
            max={128}
            step={1}
            value={[options.length]}
            onValueChange={([length]) => change({ length })}
          />
        </div>
        <div className="flex flex-wrap gap-1" role="group" aria-label="Character sets">
          {SETS.map(({ key, label }) => (
            <ToggleButton key={key} active={options[key]} onClick={() => toggleSet(key)}>
              {label}
            </ToggleButton>
          ))}
        </div>
        <label className="flex items-center gap-2 text-[11.5px]">
          <Checkbox
            checked={options.excludeAmbiguous}
            onCheckedChange={(checked) => change({ excludeAmbiguous: checked === true })}
          />
          Exclude ambiguous characters
        </label>
        <code className="bg-muted/60 min-h-9 rounded-lg px-2 py-2 font-mono text-[12px] break-all">
          {output || " "}
        </code>
        <div className="flex items-center justify-between gap-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={setsOff}
            onClick={() => {
              touchedRef.current = true;
              void regenerate();
            }}
          >
            <RefreshCw />
            Regenerate
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={setsOff || output === ""}
            onClick={() => {
              // `setOpen` here bypasses Radix's `onOpenChange`, the popover's
              // other write path, so the options are persisted now. Clearing
              // the flag keeps a later close from writing them twice.
              void setGenerator(options);
              touchedRef.current = false;
              onUse(output);
              setOpen(false);
            }}
          >
            {setsOff ? "Pick at least one character set." : "Use"}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
