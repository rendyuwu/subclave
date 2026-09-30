import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { setAutostart, setRestoreWindowState, type ThemePref } from "@/modules/settings/store";
import { useTheme } from "@/modules/theme";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { useEffect } from "react";
import { Label } from "../components/Label";
import { SectionHeader } from "../components/SectionHeader";
import { SettingRow } from "../components/SettingRow";
import { Monitor, Moon, Sun, type LucideIcon } from "lucide-react";

const APPEARANCE: {
  id: ThemePref;
  label: string;
  icon: LucideIcon;
}[] = [
  { id: "system", label: "System", icon: Monitor },
  { id: "light", label: "Light", icon: Sun },
  { id: "dark", label: "Dark", icon: Moon },
];

export function GeneralSection() {
  const { theme, setTheme } = useTheme();
  const autostart = usePreferencesStore((s) => s.autostart);
  const restoreWindowState = usePreferencesStore((s) => s.restoreWindowState);

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
      <SectionHeader title="General" description="Appearance and startup." />

      <div className="flex flex-col gap-2">
        <Label>Appearance</Label>
        <div className="grid grid-cols-3 gap-2">
          {APPEARANCE.map((o) => {
            const Icon = o.icon;
            return (
              <button
                key={o.id}
                type="button"
                onClick={() => setTheme(o.id)}
                className={cn(
                  "group bg-card flex h-20 cursor-pointer flex-col items-center justify-center gap-1.5 rounded-lg border transition-all",
                  theme === o.id
                    ? "border-foreground/60 ring-foreground/20 ring-1"
                    : "border-border/60 hover:border-border",
                )}
              >
                <Icon size={18} strokeWidth={1.5} />
                <span className="text-[11.5px]">{o.label}</span>
              </button>
            );
          })}
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
          <SettingRow
            title="Restore window position & size"
            description="Reopen the main window where you left it. Applies on next launch."
          >
            <Switch
              checked={restoreWindowState}
              onCheckedChange={(v) => void setRestoreWindowState(v)}
            />
          </SettingRow>
        </div>
      </div>
    </div>
  );
}
