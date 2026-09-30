import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Copy, Eye, EyeOff } from "lucide-react";

import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";
import { cn } from "@/lib/utils";

// The shared masked-secret primitive. The detail pane renders it read-only
// (fetch the value on reveal, show it for a window, hide it again); the entry
// editor renders it editable (the value lives in the dialog's draft). It holds
// no vault state and never fetches on its own: `onReveal` is the caller's
// lookup, and whatever it returns is what the field shows.

const MASK = "••••••••";
const REVEAL_MS = 30_000;

export type SecretFieldProps = {
  /** The text the field owns: the draft value in the editor, "" in read-only
   *  mode where the value only arrives through `onReveal`. */
  value: string;
  /** Called when the eye is first clicked while masked; may be async and may
   *  return the plaintext, which the field then shows. */
  onReveal?: () => Promise<string | null> | string | null;
  /** When absent, no copy button is rendered. */
  onCopy?: () => void;
  /** Read-only mode: plain text plus buttons, no input at all. */
  readOnly?: boolean;
  ariaLabel: string;
  className?: string;
  /** Editable mode only; when absent the input is read-only. */
  onChange?: (value: string) => void;
  /** Notified whenever the reveal state changes. */
  onRevealChange?: (revealed: boolean) => void;
  /** Bump to close the reveal (the selected entry changed, the vault locked). */
  resetKey?: string | number;
  /** Bump to force the reveal open (the generator's Use). */
  revealKey?: number;
  /** Extra addons, such as the generator trigger. */
  children?: ReactNode;
};

export function SecretField({
  value,
  onReveal,
  onCopy,
  readOnly,
  ariaLabel,
  className,
  onChange,
  onRevealChange,
  resetKey,
  revealKey,
  children,
}: SecretFieldProps) {
  const [revealed, setRevealed] = useState(false);
  const [secret, setSecret] = useState<string | null>(null);
  const notifyRef = useRef(onRevealChange);
  useEffect(() => {
    notifyRef.current = onRevealChange;
  });

  const hide = useCallback(() => {
    setRevealed(false);
    setSecret(null);
    notifyRef.current?.(false);
  }, []);

  // The caller changed the subject under the field.
  useEffect(() => {
    hide();
  }, [resetKey, hide]);

  const lastRevealKey = useRef(revealKey);
  useEffect(() => {
    if (revealKey === lastRevealKey.current) return;
    lastRevealKey.current = revealKey;
    setSecret(null);
    setRevealed(true);
    notifyRef.current?.(true);
  }, [revealKey]);

  // The reveal window closes on its own.
  useEffect(() => {
    if (!revealed) return;
    const timer = setTimeout(hide, REVEAL_MS);
    return () => clearTimeout(timer);
  }, [revealed, hide]);

  async function toggleReveal(): Promise<void> {
    if (revealed) {
      hide();
      return;
    }
    setRevealed(true);
    if (onReveal) {
      try {
        const next = await onReveal();
        if (typeof next === "string") setSecret(next);
      } catch {
        // The caller owns the error; the field still opens.
      }
    }
    notifyRef.current?.(true);
  }

  const revealedText = revealed ? (secret ?? value) : "";
  /** Editable mode still carries the parent's value while masked (the input's
   *  own `password` type does the masking); read-only mode shows the mask. */
  const editableText = revealed ? (secret ?? value) : value;
  const actions = (
    <InputGroupAddon align="inline-end">
      {children}
      {onCopy ? (
        <InputGroupButton
          size="icon-xs"
          aria-label={`Copy ${ariaLabel}`}
          onClick={onCopy}
          className="text-muted-foreground hover:text-foreground"
        >
          <Copy />
        </InputGroupButton>
      ) : null}
      <InputGroupButton
        size="icon-xs"
        aria-label={revealed ? `Hide ${ariaLabel}` : `Reveal ${ariaLabel}`}
        aria-pressed={revealed}
        onClick={() => void toggleReveal()}
        className="text-muted-foreground hover:text-foreground"
      >
        {revealed ? <EyeOff /> : <Eye />}
      </InputGroupButton>
    </InputGroupAddon>
  );

  if (readOnly) {
    return (
      <InputGroup
        className={cn("h-8", className)}
        onBlur={(e) => {
          const next = e.relatedTarget;
          if (!(next instanceof Node) || !e.currentTarget.contains(next)) hide();
        }}
      >
        <span
          aria-label={ariaLabel}
          className={cn(
            "flex-1 truncate px-3 font-mono text-[12px]",
            !revealed && "text-muted-foreground tracking-widest",
          )}
        >
          {revealed ? revealedText : MASK}
        </span>
        {actions}
      </InputGroup>
    );
  }

  return (
    <InputGroup
      className={className}
      onBlur={(e) => {
        const next = e.relatedTarget;
        if (!(next instanceof Node) || !e.currentTarget.contains(next)) hide();
      }}
    >
      <InputGroupInput
        aria-label={ariaLabel}
        type={revealed ? "text" : "password"}
        value={editableText}
        readOnly={onChange === undefined}
        spellCheck={false}
        autoComplete="off"
        onChange={(e) => {
          // Editing a revealed stored value hands the field back to the caller.
          if (secret !== null) setSecret(null);
          onChange?.(e.target.value);
        }}
        className="font-mono text-[12px]"
      />
      {actions}
    </InputGroup>
  );
}
