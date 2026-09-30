import { Plus, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { EntryUrl, MatchMode } from "@/modules/vault/types";
import { Field, ToggleButton } from "./FormControls";

// The editor's URL rows. The match mode decides where a URL is offered (the
// browser and the extension read the same rule), so each mode carries its own
// one-line explanation rather than a bare "domain / host / exact".

const MATCHES: { id: MatchMode; label: string; explain: string }[] = [
  { id: "domain", label: "Domain", explain: "Any subdomain of this host." },
  { id: "host", label: "Host", explain: "This host exactly, no subdomains." },
  { id: "exact", label: "Exact", explain: "This URL and paths below it." },
];

export function UrlListField({
  urls,
  onChange,
}: {
  urls: EntryUrl[];
  onChange: (urls: EntryUrl[]) => void;
}) {
  function update(index: number, patch: Partial<EntryUrl>): void {
    onChange(urls.map((url, i) => (i === index ? { ...url, ...patch } : url)));
  }

  return (
    <Field label="URLs">
      <div className="flex flex-col gap-2">
        {urls.map((url, index) => (
          <div key={index} className="border-border/60 flex flex-col gap-1.5 rounded-xl border p-2">
            <div className="flex items-center gap-1.5">
              <Input
                aria-label={index === 0 ? "Primary URL" : `URL ${index + 1}`}
                value={url.url}
                spellCheck={false}
                autoComplete="off"
                placeholder="https://example.com"
                onChange={(e) => update(index, { url: e.target.value })}
                className="h-8 font-mono text-[12px]"
              />
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={`Remove URL ${index + 1}`}
                onClick={() => onChange(urls.filter((_, i) => i !== index))}
              >
                <X />
              </Button>
            </div>
            <div className="flex flex-wrap items-center gap-1">
              {MATCHES.map(({ id, label }) => (
                <ToggleButton
                  key={id}
                  active={url.match === id}
                  onClick={() => update(index, { match: id })}
                >
                  {label}
                </ToggleButton>
              ))}
              {index === 0 ? (
                <span className="text-muted-foreground text-[10px] tracking-tight">primary</span>
              ) : null}
            </div>
            <p className="text-muted-foreground text-[10px]">
              {MATCHES.find((m) => m.id === url.match)?.explain ?? ""}
            </p>
          </div>
        ))}
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="self-start"
          onClick={() => onChange([...urls, { url: "", match: "domain" }])}
        >
          <Plus />
          Add URL
        </Button>
      </div>
    </Field>
  );
}
