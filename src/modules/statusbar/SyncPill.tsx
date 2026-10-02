import { IconTooltip } from "@/components/ui/icon-tooltip";
import { Spinner } from "@/components/ui/spinner";
import { formatRelativeTime } from "@/lib/format";
import { openSettingsWindow } from "@/modules/settings/openSettingsWindow";
import { useSyncStore } from "@/modules/sync/store";
import { useVaultStore } from "@/modules/vault/store";
import { CircleAlert, Pause, RefreshCw } from "lucide-react";

/** One pill in the UpdaterPill shape: icon + label, tooltip on top. The
 *  destructive style is reserved for the error state. */
function Pill({
  label,
  tooltip,
  error = false,
  children,
}: {
  label: string;
  tooltip: string;
  error?: boolean;
  children: React.ReactNode;
}) {
  const pillClass = error
    ? "bg-destructive text-destructive-foreground hover:bg-destructive/90 focus-visible:ring-destructive/35"
    : "bg-primary text-primary-foreground hover:bg-primary/90 focus-visible:ring-primary/35";

  return (
    <IconTooltip label={tooltip} side="top">
      <button
        type="button"
        onClick={() => void openSettingsWindow("sync")}
        aria-label={tooltip}
        className={`inline-flex h-6 shrink-0 cursor-pointer items-center gap-1.5 rounded-md px-2.5 text-[11px] font-medium shadow-sm transition-colors focus-visible:ring-2 focus-visible:outline-none ${pillClass}`}
      >
        {children}
        <span className="truncate">{label}</span>
      </button>
    </IconTooltip>
  );
}

/**
 * Sync state in the status bar, hidden until sync is enabled. The pill is the
 * only surface besides Settings; the triggers themselves live in the scheduler.
 */
export function SyncPill() {
  const enabled = useSyncStore((s) => s.config.enabled);
  const status = useSyncStore((s) => s.status);
  const phase = useSyncStore((s) => s.phase);
  const locked = useVaultStore((s) => s.status?.locked ?? false);

  if (!enabled) return null;

  if (locked || status.paused)
    return (
      <Pill label="Sync paused (locked)" tooltip="Sync paused (locked)">
        <Pause size={11} strokeWidth={1.75} className="shrink-0" />
      </Pill>
    );

  if (phase === "syncing")
    return (
      <Pill label="Syncing" tooltip="Syncing">
        <Spinner className="size-3 shrink-0" />
      </Pill>
    );

  if (status.lastError !== null)
    return (
      <Pill label="Sync error" tooltip={`Sync error: ${status.lastError}`} error>
        <CircleAlert size={11} strokeWidth={1.75} className="shrink-0" />
      </Pill>
    );

  if (status.lastPullAt === null)
    return (
      <Pill label="Not synced yet" tooltip="Not synced yet">
        <RefreshCw size={11} strokeWidth={1.75} className="shrink-0" />
      </Pill>
    );

  const ago = formatRelativeTime(status.lastPullAt);
  return (
    <Pill label={`Synced ${ago}`} tooltip={`Last sync ${ago}`}>
      <RefreshCw size={11} strokeWidth={1.75} className="shrink-0" />
    </Pill>
  );
}
