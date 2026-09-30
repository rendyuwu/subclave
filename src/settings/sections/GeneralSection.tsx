import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { setRestoreWindowState, type ThemePref } from "@/modules/settings/store";
import { useTheme } from "@/modules/theme";
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
  const restoreWindowState = usePreferencesStore((s) => s.restoreWindowState);

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
