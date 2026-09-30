import { Input } from "@/components/ui/input";
import { X } from "lucide-react";
import { useId, useRef, useState } from "react";

// The entry editor's tag field: chips with a remove button, plus a text input
// that commits a new chip on Enter or comma. Not `Combobox`: that component is
// built for "pick exactly one of these strings" and every call site wants a
// single value, while this wants several, added and removed one at a time.
//
// A native `<datalist>` backs the suggestions rather than a second popover: it
// needs no dependency, and a suggestion here is a typing aid, not a
// constraint, so the normaliser still runs on every commit.

export type TagsInputProps = {
  tags: readonly string[];
  onChange: (tags: readonly string[]) => void;
  /** Every tag already used across saved entries, offered through the
   *  `<datalist>`. Exact stored spellings, not deduped across case. */
  suggestions: readonly string[];
};

/** Trim, drop blanks and case-insensitive duplicates, keeping the first spelling. */
function normalizeVaultTags(candidates: readonly string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of candidates) {
    const trimmed = raw.trim();
    if (!trimmed) continue;
    const key = trimmed.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(trimmed);
  }
  return out;
}

export function TagsInput({ tags, onChange, suggestions }: TagsInputProps) {
  const [draft, setDraft] = useState("");
  const listId = useId();
  const inputRef = useRef<HTMLInputElement>(null);

  function commit(candidate: string): void {
    const next = normalizeVaultTags([...tags, candidate]);
    if (next.length <= tags.length) {
      // The normaliser dropped the candidate (a case-duplicate, or blank).
      // Keep the draft rather than clearing it with no sign anything happened.
      return;
    }
    onChange(next);
    setDraft("");
  }

  function remove(tag: string): void {
    const key = tag.toLowerCase();
    onChange(tags.filter((t) => t.toLowerCase() !== key));
    // The removed chip's own button unmounts with it, so keyboard focus would
    // otherwise fall back to the dialog container. Send it to the input.
    inputRef.current?.focus();
  }

  return (
    <div className="flex flex-col gap-1.5">
      {tags.length > 0 ? (
        <div className="flex flex-wrap gap-1">
          {tags.map((tag) => (
            <span
              key={tag}
              className="border-border inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs"
            >
              {tag}
              <button
                type="button"
                aria-label={`Remove tag ${tag}`}
                onClick={() => remove(tag)}
                className="text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:ring-ring/50 flex size-5 items-center justify-center rounded outline-none focus-visible:ring-2"
              >
                <X size={11} strokeWidth={2} />
              </button>
            </span>
          ))}
        </div>
      ) : null}
      <Input
        ref={inputRef}
        list={listId}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => {
          // An Enter pressed to confirm an IME (CJK) conversion must not commit
          // the unconverted draft as a tag.
          if (e.nativeEvent.isComposing || e.keyCode === 229) return;
          if (e.key === "Enter" || e.key === ",") {
            e.preventDefault();
            if (draft.trim()) commit(draft);
          } else if (e.key === "Backspace" && draft === "" && tags.length > 0) {
            remove(tags[tags.length - 1]);
          }
        }}
        // A typed-but-uncommitted tag on blur (tabbing on, or closing the dialog
        // straight from here) is not silently dropped.
        onBlur={() => {
          if (draft.trim()) commit(draft);
        }}
        placeholder="Add a tag..."
        spellCheck={false}
        autoComplete="off"
        className="h-8 text-[12px]"
      />
      <datalist id={listId}>
        {suggestions.map((s) => (
          <option key={s} value={s} />
        ))}
      </datalist>
    </div>
  );
}
