import { useState, type KeyboardEventHandler, type Ref } from "react";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Eye, EyeOff } from "lucide-react";

type Props = {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  autoComplete?: string;
  autoFocus?: boolean;
  inputRef?: Ref<HTMLInputElement>;
  onKeyDown?: KeyboardEventHandler<HTMLInputElement>;
  onKeyUp?: KeyboardEventHandler<HTMLInputElement>;
};

/**
 * A labelled password input with a reveal toggle. Shared by the create and
 * unlock screens. The reveal state never leaves this component and the value
 * never reaches the vault store, which holds no password.
 */
export function PasswordField({
  id,
  label,
  value,
  onChange,
  autoComplete,
  autoFocus,
  inputRef,
  onKeyDown,
  onKeyUp,
}: Props) {
  const [revealed, setRevealed] = useState(false);

  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="text-xs font-medium">
        {label}
      </label>
      <InputGroup>
        <InputGroupInput
          id={id}
          ref={inputRef}
          type={revealed ? "text" : "password"}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          autoComplete={autoComplete}
          autoFocus={autoFocus}
          spellCheck={false}
          onKeyDown={onKeyDown}
          onKeyUp={onKeyUp}
        />
        {/* Overrides the addon's `pr-3 has-[>button]:-mr-1`: the button sits in the
            same place, but the addon no longer hangs 3px past the group, which a
            scrolling parent turns into a horizontal scrollbar. */}
        <InputGroupAddon align="inline-end" className="pr-2 has-[>button]:mr-0">
          <button
            type="button"
            onClick={() => setRevealed((r) => !r)}
            aria-label={revealed ? "Hide password" : "Show password"}
            aria-pressed={revealed}
            className="text-muted-foreground hover:text-foreground flex size-5 shrink-0 cursor-pointer items-center justify-center rounded-md transition-colors"
          >
            {revealed ? <EyeOff size={14} strokeWidth={2} /> : <Eye size={14} strokeWidth={2} />}
          </button>
        </InputGroupAddon>
      </InputGroup>
    </div>
  );
}
