import { Button } from "@/components/ui/button";
import type { SyncRequest, SyncStatus } from "@/modules/sync/types";
import { SettingRow } from "../../components/SettingRow";

/**
 * The Status block: the two requests worth making by hand, and everything
 * `main` last wrote back about the remote. It holds no state of its own, so
 * what it shows is the file the rest of the app writes.
 */
export function SyncStatusPanel({
  status,
  enabled,
  onRequest,
  onRefresh,
}: {
  status: SyncStatus;
  /** Off means no network, so neither button has anything to ask for. */
  enabled: boolean;
  onRequest: (what: SyncRequest) => void;
  onRefresh: () => void;
}) {
  return (
    <>
      <div className="flex items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          className="h-8 px-2 text-[11px]"
          disabled={!enabled}
          onClick={() => void onRequest("pull")}
        >
          Pull now
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-8 px-2 text-[11px]"
          disabled={!enabled}
          onClick={() => void onRequest("push")}
        >
          Push now
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-8 px-2 text-[11px]"
          onClick={() => void onRefresh()}
        >
          Refresh
        </Button>
      </div>
      <SettingRow title="Last pull">
        <span className="text-muted-foreground text-[11px]">
          {status.lastPullAt === null ? "Never" : new Date(status.lastPullAt).toLocaleString()}
        </span>
      </SettingRow>
      <SettingRow title="Last push">
        <span className="text-muted-foreground text-[11px]">
          {status.lastPushAt === null ? "Never" : new Date(status.lastPushAt).toLocaleString()}
        </span>
      </SettingRow>
      <SettingRow
        title="Waiting to be pushed"
        description="Records this device has changed that the remote does not hold yet: at least this many."
      >
        <span className="text-muted-foreground text-[11px] tabular-nums">{status.pending}</span>
      </SettingRow>

      {status.quarantine.length > 0 ? (
        <div className="border-border/60 bg-card flex flex-col gap-1.5 rounded-lg border px-3 py-2.5">
          <span className="text-[12.5px] font-medium">Unreadable remote objects</span>
          <span className="text-muted-foreground text-[10.5px] leading-relaxed">
            These could not be decrypted or parsed, so they were left alone. A wrong passphrase on
            one device is the usual cause. Nothing here is deleted.
          </span>
          <ul className="flex flex-col gap-1">
            {status.quarantine.map((q) => (
              <li key={q.name} className="font-mono text-[10.5px] break-all">
                {q.name} - <span className="text-muted-foreground">{q.reason}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {status.stale.length > 0 ? (
        <div className="border-border/60 bg-card flex flex-col gap-1.5 rounded-lg border px-3 py-2.5">
          <span className="text-[12.5px] font-medium">Local records the remote has dropped</span>
          <span className="text-muted-foreground text-[10.5px] leading-relaxed">
            This device still holds these and the remote no longer has an object for them. They are
            reported and never deleted. Each is older than the 90-day window a deletion travels in,
            so the likeliest reading is that it was deleted on another device long ago. A pull does
            not re-publish these: edit one to send it to the remote again, or delete it here to
            accept the removal.
          </span>
          <ul className="flex flex-col gap-1">
            {status.stale.map((s) => (
              <li key={`${s.kind}:${s.id}`} className="font-mono text-[10.5px] break-all">
                {s.kind} - <span className="text-muted-foreground">{s.id}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      {status.lastError ? (
        <div
          role="alert"
          className="border-destructive/40 bg-destructive/5 flex flex-col gap-1 rounded-lg border px-3 py-2.5"
        >
          <span className="text-destructive text-[11.5px] font-semibold">
            Error on the last run
          </span>
          {/* In full. A truncated remote error is the one shape that reliably
              hides the clause naming the bucket or the missing permission. */}
          <span className="font-mono text-[10.5px] leading-relaxed break-words whitespace-pre-wrap">
            {status.lastError}
          </span>
        </div>
      ) : null}
    </>
  );
}
