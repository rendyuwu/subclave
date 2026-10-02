import { useState } from "react";

import { Checkbox } from "@/components/ui/checkbox";

import { StrengthMeter } from "./editor/StrengthMeter";
import { PasswordField } from "./PasswordField";

/** The shortest master password a first-run screen accepts. */
export const MIN_PASSWORD_LENGTH = 8;

/**
 * The master-password half of a first-run screen: the password, its
 * confirmation, and the no-recovery acknowledgement, plus the one boolean both
 * screens gate Submit on.
 */
export type MasterPasswordDraft = {
  password: string;
  confirm: string;
  acknowledged: boolean;
  setPassword: (value: string) => void;
  setConfirm: (value: string) => void;
  setAcknowledged: (value: boolean) => void;
  /** Long enough, confirmed, and acknowledged. A screen still has to add `!busy`. */
  ready: boolean;
};

/**
 * The state behind {@link MasterPasswordFields}, shared so the create and join
 * screens cannot gate Submit on two different sets of rules.
 */
export function useMasterPassword(): MasterPasswordDraft {
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [acknowledged, setAcknowledged] = useState(false);

  // Counted by code point rather than UTF-16 unit, so four emoji are four
  // characters and not eight.
  const longEnough = [...password].length >= MIN_PASSWORD_LENGTH;

  return {
    password,
    confirm,
    acknowledged,
    setPassword,
    setConfirm,
    setAcknowledged,
    ready: longEnough && password === confirm && acknowledged,
  };
}

/**
 * The master-password block of the create and join screens, in the order both
 * render it: password, confirmation with the strength meter, acknowledgement,
 * then the error paragraph.
 */
export function MasterPasswordFields({
  draft,
  idPrefix,
  error,
  description,
  autoFocus,
}: {
  draft: MasterPasswordDraft;
  /** `create` or `join`. Prefixes every `id`/`htmlFor` pair so the two forms never share one. */
  idPrefix: string;
  error: string | null;
  /** The line under the password field, on the screen that has one. */
  description?: string;
  autoFocus?: boolean;
}) {
  return (
    <>
      <div className="flex flex-col gap-1">
        <PasswordField
          id={`${idPrefix}-password`}
          label="Master password"
          value={draft.password}
          onChange={draft.setPassword}
          autoComplete="new-password"
          autoFocus={autoFocus}
        />
        {description ? (
          <span className="text-muted-foreground text-[10.5px] leading-relaxed">{description}</span>
        ) : null}
      </div>

      <div className="flex flex-col gap-1.5">
        <PasswordField
          id={`${idPrefix}-confirm`}
          label="Confirm master password"
          value={draft.confirm}
          onChange={draft.setConfirm}
          autoComplete="new-password"
        />
        <StrengthMeter value={draft.password} />
      </div>

      <div className="flex items-start gap-2">
        <Checkbox
          id={`${idPrefix}-acknowledge`}
          checked={draft.acknowledged}
          onCheckedChange={(checked) => draft.setAcknowledged(checked === true)}
          className="mt-0.5"
        />
        <label
          htmlFor={`${idPrefix}-acknowledge`}
          className="text-muted-foreground cursor-pointer text-xs leading-relaxed"
        >
          I understand there is no recovery if I forget this password.
        </label>
      </div>

      {error ? (
        <p role="alert" className="text-destructive text-xs">
          {error}
        </p>
      ) : null}
    </>
  );
}
