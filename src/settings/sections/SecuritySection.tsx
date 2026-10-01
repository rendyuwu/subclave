import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  setAutoLockMinutes,
  setAutostart,
  setClipboardClearSeconds,
  setCloseToTray,
  setLockOnMinimize,
} from "@/modules/settings/mutations";
import { AUTO_LOCK_MINUTES_MAX, CLIPBOARD_CLEAR_SECONDS_MAX } from "@/modules/settings/schema";
import { vaultStatus, vaultTouch } from "@/modules/vault/ipc";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { useEffect, useState } from "react";
import { Label } from "../components/Label";
import { SectionHeader } from "../components/SectionHeader";
import { SettingRow } from "../components/SettingRow";
import { ChangeMasterDialog } from "./ChangeMasterDialog";

/** A whole-number preference row. The value commits on blur or Enter, clamped
 *  to `0..=max`, so a mid-typing value never lands in the store. */
function NumberRow({
  title,
  description,
  value,
  max,
  suffix,
  onCommit,
}: {
  title: string;
  description: string;
  value: number;
  max: number;
  suffix: string;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  useEffect(() => setDraft(String(value)), [value]);

  const commit = () => {
    const parsed = Number.parseInt(draft, 10);
    const next = Number.isFinite(parsed) ? Math.min(max, Math.max(0, parsed)) : value;
    setDraft(String(next));
    if (next !== value) onCommit(next);
  };

  return (
    <SettingRow title={title} description={description}>
      <div className="flex items-center gap-2">
        <Input
          type="number"
          min={0}
          max={max}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur();
          }}
          className="h-8 w-20 text-right tabular-nums"
        />
        <span className="text-muted-foreground text-[10.5px]">{suffix}</span>
      </div>
    </SettingRow>
  );
}

export function SecuritySection() {
  const autoLockMinutes = usePreferencesStore((s) => s.autoLockMinutes);
  const clipboardClearSeconds = usePreferencesStore((s) => s.clipboardClearSeconds);
  const lockOnMinimize = usePreferencesStore((s) => s.lockOnMinimize);
  const closeToTray = usePreferencesStore((s) => s.closeToTray);
  const autostart = usePreferencesStore((s) => s.autostart);
  const [changeMasterOpen, setChangeMasterOpen] = useState(false);
  const [vaultLocked, setVaultLocked] = useState(false);

  // Reconcile autostart pref with actual OS state on mount; the user may have
  // toggled it from System Settings.
  useEffect(() => {
    let alive = true;
    void isEnabled()
      .then((on) => {
        if (!alive) return;
        if (on !== usePreferencesStore.getState().autostart) {
          void setAutostart(on);
        }
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  // The master password can only change while the vault is unlocked. One probe
  // runs per mount, which is per tab visit; a lock that happens in the main
  // window while this tab stays open is not seen here, and the click answers
  // with Rust's own refusal instead.
  useEffect(() => {
    let alive = true;
    void vaultStatus()
      .then((s) => {
        if (alive) setVaultLocked(s.locked);
      })
      .catch(() => {
        if (alive) setVaultLocked(true);
      });
    return () => {
      alive = false;
    };
  }, []);

  const onToggleAutostart = async (next: boolean) => {
    try {
      if (next) await enable();
      else await disable();
      await setAutostart(next);
    } catch (e) {
      console.error("autostart toggle failed", e);
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader title="Security" description="Locking, clipboard and the master password." />

      <div className="flex flex-col gap-2">
        <Label>Locking</Label>
        <div className="flex flex-col gap-2">
          <NumberRow
            title="Auto-lock after"
            description="How long the vault stays unlocked while idle."
            value={autoLockMinutes}
            max={AUTO_LOCK_MINUTES_MAX}
            suffix="minutes, 0 = never"
            onCommit={async (v) => {
              // The touch must come AFTER the write lands: `vault_touch` makes
              // Rust re-read the settings file, and fire-and-forget ordering
              // let the touch read the OLD value back, so a shortened window
              // did not take effect until the next unlock.
              await setAutoLockMinutes(v);
              // Rust reads the window at unlock or touch time, so a nudge makes
              // a running vault pick the new value up within the second.
              await vaultTouch().catch(() => undefined);
            }}
          />
          <NumberRow
            title="Clear the clipboard after"
            description="How long a copied secret stays on the clipboard."
            value={clipboardClearSeconds}
            max={CLIPBOARD_CLEAR_SECONDS_MAX}
            suffix="seconds, 0 = never"
            onCommit={(v) => void setClipboardClearSeconds(v)}
          />
          <SettingRow
            title="Lock when minimized"
            description="Lock the vault when the main window is minimized."
          >
            <Switch checked={lockOnMinimize} onCheckedChange={(v) => void setLockOnMinimize(v)} />
          </SettingRow>
          <SettingRow
            title="Close to the tray"
            description="Closing the window hides it. Subclave keeps running so the browser extension stays connected."
          >
            <Switch checked={closeToTray} onCheckedChange={(v) => void setCloseToTray(v)} />
          </SettingRow>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <Label>Startup</Label>
        <div className="flex flex-col gap-2">
          <SettingRow
            title="Launch at login"
            description="Open Subclave automatically when you sign in."
          >
            <Switch checked={autostart} onCheckedChange={(v) => void onToggleAutostart(v)} />
          </SettingRow>
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <Label>Master password</Label>
        <div className="flex flex-col gap-2">
          <SettingRow
            title="Change master password"
            description="Re-encrypts the vault with a new password. Existing backups still open with the old one."
          >
            <Button
              variant="outline"
              size="sm"
              disabled={vaultLocked}
              onClick={() => setChangeMasterOpen(true)}
            >
              Change password
            </Button>
          </SettingRow>
        </div>
      </div>

      <ChangeMasterDialog open={changeMasterOpen} onOpenChange={setChangeMasterOpen} />
    </div>
  );
}
