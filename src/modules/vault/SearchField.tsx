import { useRef } from "react";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { Kbd } from "@/components/ui/kbd";
import { KEY_SEP } from "@/lib/platform";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { useGlobalShortcuts } from "@/modules/shortcuts";
import { getBindingTokens, SHORTCUTS } from "@/modules/shortcuts/shortcuts";
import { Search, X } from "lucide-react";
import { useVaultStore } from "./store";

const SEARCH_SHORTCUT = SHORTCUTS.find((s) => s.id === "search.focus");

/**
 * The header's vault search field. It owns the input ref and the `search.focus`
 * chord, which is why that id is deliberately absent from the workspace's
 * command map: this component is the only thing that can focus the field it
 * renders.
 */
export function VaultSearchInput() {
  const query = useVaultStore((s) => s.query);
  const setQuery = useVaultStore((s) => s.setQuery);
  const userShortcuts = usePreferencesStore((s) => s.shortcuts);
  const inputRef = useRef<HTMLInputElement>(null);

  useGlobalShortcuts({
    "search.focus": () => {
      inputRef.current?.focus();
      inputRef.current?.select();
    },
  });

  const binding = userShortcuts["search.focus"]?.[0] ?? SEARCH_SHORTCUT?.defaultBindings[0];
  const tokens = getBindingTokens(binding);

  return (
    <InputGroup className="w-full">
      <InputGroupAddon align="inline-start">
        <Search strokeWidth={2} className="size-4 shrink-0 opacity-50" />
      </InputGroupAddon>
      <InputGroupInput
        ref={inputRef}
        value={query}
        onChange={(e) => void setQuery(e.target.value)}
        placeholder="Search entries"
        aria-label="Search entries"
        className="text-sm"
      />
      <InputGroupAddon align="inline-end">
        {query ? (
          <button
            type="button"
            onClick={() => void setQuery("")}
            aria-label="Clear search"
            className="text-muted-foreground hover:text-foreground flex size-5 shrink-0 cursor-pointer items-center justify-center rounded-md transition-colors"
          >
            <X size={14} strokeWidth={2} />
          </button>
        ) : null}
        {tokens.length > 0 ? <Kbd>{tokens.join(KEY_SEP)}</Kbd> : null}
      </InputGroupAddon>
    </InputGroup>
  );
}
