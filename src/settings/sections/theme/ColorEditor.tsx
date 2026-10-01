import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { setCustomTheme } from "@/modules/settings/mutations";
import type { CustomTheme, ThemeColors } from "@/modules/settings/theme/model";
import { THEME_PRESETS } from "@/modules/settings/themePresets";
import { useTheme } from "@/modules/theme";
import { useEffect, useMemo, useState } from "react";
import { SettingsAccordion } from "../../components/SettingsAccordion";
import { ColorSwatch } from "./ColorPicker";
import { COLOR_FIELDS, GROUPS, type Group } from "./colorFields";

/**
 * The Colors accordion: a light/dark variant toggle, the group tabs, and the
 * swatch grid for whichever group is active.
 */
export function ColorEditor() {
  const theme = usePreferencesStore((s) => s.customTheme);
  const [activeGroup, setActiveGroup] = useState<Group>("Base");
  // Which color variant is currently being edited. Defaults to the resolved
  // theme so the inputs always show what's actually painted on screen.
  const { resolvedTheme } = useTheme();
  const [editMode, setEditMode] = useState<"light" | "dark">(resolvedTheme);
  useEffect(() => setEditMode(resolvedTheme), [resolvedTheme]);

  const activeColors: ThemeColors = editMode === "dark" ? theme.dark : theme.light;

  const updateColor = (key: keyof ThemeColors, value: string) => {
    // Mark known presets as "modified" once, so the user can tell the active
    // palette has diverged from the canonical preset. Subsequent edits don't
    // keep appending "(modified)".
    const isPreset = THEME_PRESETS.some((p) => p.name === theme.name);
    const variantKey = editMode === "dark" ? "dark" : "light";
    const next: CustomTheme = {
      ...theme,
      name: isPreset ? `${theme.name} (modified)` : theme.name,
      [variantKey]: { ...theme[variantKey], [key]: value },
    };
    void setCustomTheme(next);
  };

  const visibleFields = useMemo(
    () => COLOR_FIELDS.filter((f) => f.group === activeGroup),
    [activeGroup],
  );

  return (
    <SettingsAccordion title="Colors" summary={editMode === "dark" ? "Dark" : "Light"}>
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-end">
          <div className="bg-muted/40 inline-flex h-7 items-center p-0.5 text-[11px]">
            <button
              type="button"
              onClick={() => setEditMode("light")}
              className={cn(
                "h-6 px-2.5 transition-colors",
                editMode === "light"
                  ? "bg-background text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              Light
            </button>
            <button
              type="button"
              onClick={() => setEditMode("dark")}
              className={cn(
                "h-6 px-2.5 transition-colors",
                editMode === "dark"
                  ? "bg-background text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              Dark
            </button>
          </div>
        </div>
        <div className="flex flex-wrap gap-1">
          {GROUPS.map((g) => (
            <Button
              key={g}
              type="button"
              variant={g === activeGroup ? "default" : "outline"}
              size="sm"
              className="h-7 px-2.5 text-[11px]"
              onClick={() => setActiveGroup(g)}
            >
              {g}
            </Button>
          ))}
        </div>
        <div className="border-border/60 bg-card grid grid-cols-1 gap-1 border p-2 sm:grid-cols-2">
          {visibleFields.map((field) => (
            <div key={field.key} className="flex items-center justify-between gap-3 px-2 py-1.5">
              <span className="text-[11.5px]">{field.label}</span>
              <ColorSwatch
                value={activeColors[field.key]}
                onChange={(next) => updateColor(field.key, next)}
              />
            </div>
          ))}
        </div>
      </div>
    </SettingsAccordion>
  );
}
