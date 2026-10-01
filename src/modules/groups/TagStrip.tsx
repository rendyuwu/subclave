// The tag filter strip above the entry list: one chip per tag in use,
// multi-select, plus a clear action once any are selected. Zero prop and
// store-driven. It renders nothing only when there is no tag and no filter: a
// filter outlives its last tagged entry (trashing that entry drops its chip but
// keeps the list filtered), so the Clear action has to stay reachable.

import { Chip } from "./Chip";
import { tagCounts } from "@/modules/vault/list/derive";
import { useVaultStore } from "@/modules/vault/store";
import { X } from "lucide-react";
import type { ReactNode } from "react";

export function TagStrip(): ReactNode {
  const entries = useVaultStore((s) => s.entries);
  const tagFilter = useVaultStore((s) => s.tagFilter);
  const toggleTag = useVaultStore((s) => s.toggleTag);
  const clearTags = useVaultStore((s) => s.clearTags);

  const tags = tagCounts(entries);
  const selected = new Set(tagFilter);
  if (tags.length === 0 && selected.size === 0) return null;

  return (
    <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label="Tags">
      {tags.map((entry) => {
        const key = entry.tag.toLowerCase();
        return (
          <Chip
            key={key}
            label={entry.tag}
            count={entry.count}
            selected={selected.has(key)}
            onClick={() => toggleTag(key)}
          />
        );
      })}
      {selected.size > 0 ? (
        <button
          type="button"
          aria-label="Clear tag filter"
          onClick={clearTags}
          className="text-muted-foreground hover:text-foreground inline-flex items-center gap-1 text-xs transition-colors"
        >
          <X size={12} strokeWidth={2} />
          Clear
        </button>
      ) : null}
    </div>
  );
}
