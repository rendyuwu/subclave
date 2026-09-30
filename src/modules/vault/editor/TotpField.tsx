import { useEffect, useState } from "react";
import { X } from "lucide-react";

import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
} from "@/components/ui/input-group";
import { totpPreview } from "@/modules/vault/ipc";
import { describeVaultError } from "@/modules/vault/errors";
import type { TotpCode } from "@/modules/vault/types";
import { Field } from "./FormControls";
import { totpUriFromInput } from "./draft";

// The editor's TOTP row. It accepts either an `otpauth://` URI or a bare base32
// secret (wrapped by `totpUriFromInput`) and previews the code before anything
// is saved, so a bad URI is reported here rather than on Save.

function groupDigits(code: string): string {
  return code.replace(/(.{3})(?=.)/g, "$1 ");
}

export function TotpField({
  value,
  onChange,
  ariaLabel = "TOTP secret",
}: {
  value: string;
  onChange: (value: string) => void;
  ariaLabel?: string;
}) {
  const [preview, setPreview] = useState<TotpCode | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [left, setLeft] = useState(0);

  async function load(raw: string): Promise<void> {
    try {
      const next = await totpPreview(totpUriFromInput(raw));
      setPreview(next);
      setError(null);
    } catch (e) {
      setPreview(null);
      setError(describeVaultError(String(e)));
    }
  }

  useEffect(() => {
    if (value.trim() === "") {
      setPreview(null);
      setError(null);
      return;
    }
    const timer = setTimeout(() => void load(value), 200);
    return () => clearTimeout(timer);
    // `load` reads nothing but its argument and the setters.
  }, [value]);

  // A countdown that re-reads the code when its period rolls over.
  useEffect(() => {
    if (!preview) return;
    setLeft(preview.remaining);
    const ticker = setInterval(() => setLeft((s) => Math.max(0, s - 1)), 1000);
    return () => clearInterval(ticker);
  }, [preview]);

  useEffect(() => {
    if (!preview || left > 0) return;
    const timer = setTimeout(() => void load(value), 200);
    return () => clearTimeout(timer);
  }, [left, preview, value]);

  return (
    <Field label="TOTP (optional)">
      <InputGroup className="h-8">
        <InputGroupInput
          aria-label={ariaLabel}
          value={value}
          spellCheck={false}
          autoComplete="off"
          placeholder="otpauth:// URI or base32 secret"
          onChange={(e) => onChange(e.target.value)}
          className="font-mono text-[12px]"
        />
        {value !== "" ? (
          <InputGroupAddon align="inline-end">
            <InputGroupButton
              size="icon-xs"
              aria-label="Clear TOTP"
              onClick={() => onChange("")}
              className="text-muted-foreground hover:text-foreground"
            >
              <X />
            </InputGroupButton>
          </InputGroupAddon>
        ) : null}
      </InputGroup>
      {value.trim() !== "" && error ? (
        <p className="text-destructive text-[11px]">{error}</p>
      ) : value.trim() !== "" && preview ? (
        <p className="text-muted-foreground font-mono text-[11px]">
          {groupDigits(preview.code)} <span className="tabular-nums">{left}s</span>
        </p>
      ) : null}
    </Field>
  );
}
