import type { ReactNode } from "react";
import { Copy, ExternalLink, Pencil, RotateCcw, Star } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import { EntryGlyph } from "@/modules/groups/AppearancePicker";
import { draftFromDetail, toDraftPayload } from "@/modules/vault/editor/draft";
import { SecretField } from "@/modules/vault/editor/SecretField";

import { copyEntryField, openEntryUrl, runVaultMutation } from "./commands";
import { HistoryList } from "./HistoryList";
import * as vault from "./ipc";
import { isExpired, TRASH_SCOPE } from "./list/derive";
import { useVaultStore } from "./store";
import { TotpCode } from "./TotpCode";
import type { MatchMode } from "./types";

const MATCH_LABELS: Record<MatchMode, string> = {
  domain: "Domain",
  exact: "Exact",
  host: "Host",
};

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    toast("URL copied.", { variant: "success" });
  } catch {
    toast("Could not copy.", { variant: "error" });
  }
}

function CopyButton({ label, onClick }: { label: string; onClick: () => void }): ReactNode {
  return (
    <Button variant="ghost" size="icon-xs" aria-label={label} onClick={onClick}>
      <Copy strokeWidth={1.75} />
    </Button>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }): ReactNode {
  return (
    <div className="border-border/40 flex flex-col gap-1.5 border-b px-3 py-2.5">
      <span className="text-muted-foreground text-[11px] font-medium tracking-wide uppercase">
        {label}
      </span>
      {children}
    </div>
  );
}

export function EntryDetail(): ReactNode {
  const detail = useVaultStore((s) => s.detail);

  if (!detail) {
    return (
      <div className="text-muted-foreground flex h-full items-center justify-center p-4 text-center text-xs">
        Select an entry to see its details.
      </div>
    );
  }

  const inTrash = detail.groupId === TRASH_SCOPE;
  const expired = isExpired(detail, Date.now());

  const toggleFavorite = () => {
    const draft = draftFromDetail(detail, detail.groupId);
    draft.favorite = !draft.favorite;
    runVaultMutation(() => vault.vaultEntryUpsert(toDraftPayload(draft)));
  };
  const restore = () => runVaultMutation(() => vault.vaultEntryRestore([detail.id]));

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="border-border/60 flex h-9 shrink-0 items-center gap-2 border-b px-3">
        <EntryGlyph icon={detail.icon} color={detail.color} />
        <span className="min-w-0 flex-1 truncate text-sm font-semibold">{detail.title}</span>
        {expired ? (
          <Badge variant="destructive" className="h-4 shrink-0 px-1.5 text-[10px]">
            Expired
          </Badge>
        ) : null}
        {inTrash ? (
          <Button variant="outline" size="xs" onClick={restore}>
            <RotateCcw strokeWidth={1.75} />
            Restore
          </Button>
        ) : (
          <>
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label="Edit entry"
              onClick={() => useVaultStore.getState().openEditor(detail.id, detail.groupId)}
            >
              <Pencil strokeWidth={1.75} />
            </Button>
            <Button
              variant="ghost"
              size="icon-xs"
              aria-label={detail.favorite ? "Remove from favourites" : "Add to favourites"}
              aria-pressed={detail.favorite}
              onClick={toggleFavorite}
            >
              <Star strokeWidth={1.75} className={detail.favorite ? "fill-current" : undefined} />
            </Button>
          </>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <Row label="Username">
          <div className="flex items-center gap-1.5">
            <span className="min-w-0 flex-1 truncate text-sm">
              {detail.username || <span className="text-muted-foreground">None</span>}
            </span>
            <CopyButton
              label="Copy username"
              onClick={() => void copyEntryField(detail.id, "username", "username")}
            />
          </div>
        </Row>

        <Row label="Password">
          <SecretField
            readOnly
            value=""
            ariaLabel="Password"
            resetKey={detail.id}
            onReveal={() => vault.vaultEntryReveal(detail.id, "password")}
            onCopy={
              detail.hasPassword
                ? () => void copyEntryField(detail.id, "password", "password")
                : undefined
            }
          />
        </Row>

        {detail.hasTotp ? (
          <Row label="One-time code">
            <div className="flex items-center gap-1.5">
              <div className="min-w-0 flex-1">
                <TotpCode id={detail.id} />
              </div>
              <CopyButton
                label="Copy one-time code"
                onClick={() => void copyEntryField(detail.id, "totp", "totp")}
              />
            </div>
          </Row>
        ) : null}

        {detail.urls.length > 0 ? (
          <Row label="URLs">
            <div className="flex flex-col gap-1.5">
              {detail.urls.map((url) => (
                <div key={`${url.match}:${url.url}`} className="flex items-center gap-1.5">
                  <span className="min-w-0 flex-1 truncate text-sm">{url.url}</span>
                  <Badge variant="outline" className="h-4 shrink-0 px-1.5 text-[10px] font-normal">
                    {MATCH_LABELS[url.match]}
                  </Badge>
                  <CopyButton label="Copy URL" onClick={() => void copyText(url.url)} />
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    aria-label="Open in browser"
                    onClick={() => void openEntryUrl(url.url)}
                  >
                    <ExternalLink strokeWidth={1.75} />
                  </Button>
                </div>
              ))}
            </div>
          </Row>
        ) : null}

        <Row label="Notes">
          {detail.notes ? (
            <p className="text-sm whitespace-pre-wrap">{detail.notes}</p>
          ) : (
            <span className="text-muted-foreground text-sm">No notes.</span>
          )}
        </Row>

        {detail.customFields.length > 0 ? (
          <Row label="Custom fields">
            <div className="flex flex-col gap-2">
              {detail.customFields.map((field) => (
                <div key={field.name} className="flex flex-col gap-1">
                  <span className="text-muted-foreground text-xs">{field.name}</span>
                  {field.hidden ? (
                    <SecretField
                      readOnly
                      value=""
                      ariaLabel={`Reveal ${field.name}`}
                      resetKey={`${detail.id}:${field.name}`}
                      onReveal={() => vault.vaultEntryReveal(detail.id, `custom:${field.name}`)}
                      onCopy={() =>
                        void copyEntryField(detail.id, `custom:${field.name}`, field.name)
                      }
                    />
                  ) : (
                    <div className="flex items-center gap-1.5">
                      <span className="min-w-0 flex-1 truncate font-mono text-sm">
                        {field.value ?? ""}
                      </span>
                      <CopyButton
                        label={`Copy ${field.name}`}
                        onClick={() =>
                          void copyEntryField(detail.id, `custom:${field.name}`, field.name)
                        }
                      />
                    </div>
                  )}
                </div>
              ))}
            </div>
          </Row>
        ) : null}

        {detail.history.length > 0 ? <HistoryList id={detail.id} history={detail.history} /> : null}
      </div>
    </div>
  );
}
