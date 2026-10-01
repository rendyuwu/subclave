import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { DESTRUCTIVE_ACTION } from "@/lib/toolbarButton";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  setCustomTheme,
  setCustomThemeEnabled,
  setUserThemePresets,
} from "@/modules/settings/mutations";
import type { CustomTheme } from "@/modules/settings/theme/model";
import { DEFAULT_CUSTOM_THEME, THEME_PRESETS } from "@/modules/settings/themePresets";
import { BookmarkPlus, X } from "lucide-react";
import { useState } from "react";
import { SettingsAccordion } from "../../components/SettingsAccordion";

/**
 * Small palette swatch chip: a colored background plus a strip of accent dots,
 * one per preset card. Callers pass the representative background and the dot
 * colors (ANSI hues).
 */
function PalettePreview({ background, dots }: { background: string; dots: string[] }) {
  return (
    <div
      aria-hidden
      className="border-border/40 flex h-7 w-16 shrink-0 items-center gap-[3px] overflow-hidden rounded-[3px] border px-1.5"
      style={{ background }}
    >
      {dots.map((c, i) => (
        <span key={i} className="h-2.5 w-2.5 rounded-full" style={{ background: c }} />
      ))}
    </div>
  );
}

/**
 * The Presets accordion: the built-in preset grid plus the user's saved
 * presets, with save-as, delete and reset.
 */
export function PresetEditor() {
  const enabled = usePreferencesStore((s) => s.customThemeEnabled);
  const theme = usePreferencesStore((s) => s.customTheme);
  const userPresets = usePreferencesStore((s) => s.userThemePresets);
  // Inline name input for "Save as preset". `null` = button mode, string =
  // typing the name. Submitting (Enter or save button) writes to
  // userThemePresets, then collapses back to button mode.
  const [savePresetName, setSavePresetName] = useState<string | null>(null);

  const onPickPreset = (preset: CustomTheme) => {
    // Preserve the user's chosen background image when switching colour
    // presets: colours change, wallpaper stays put.
    const next: CustomTheme = {
      ...preset,
      background: { ...theme.background, enabled: theme.background.enabled },
    };
    void setCustomTheme(next);
    if (!enabled) void setCustomThemeEnabled(true);
  };

  // Compute a non-conflicting preset name. If the input collides with a
  // built-in name or an existing user preset, append " (2)", " (3)", … until
  // unique. Keeps the user from accidentally shadowing "Dracula" et al.
  const uniquePresetName = (raw: string): string => {
    const trimmed = raw.trim();
    if (!trimmed) return "";
    const used = new Set<string>([
      ...THEME_PRESETS.map((p) => p.name),
      ...userPresets.map((p) => p.name),
    ]);
    if (!used.has(trimmed)) return trimmed;
    for (let i = 2; i < 100; i++) {
      const candidate = `${trimmed} (${i})`;
      if (!used.has(candidate)) return candidate;
    }
    return `${trimmed} ${Date.now()}`;
  };

  const onSaveAsPreset = (rawName: string) => {
    const finalName = uniquePresetName(rawName);
    if (!finalName) return;
    const newPreset: CustomTheme = {
      ...theme,
      name: finalName,
      // Drop wallpaper from the saved preset - the user's wallpaper is a
      // separate concern and follows them across preset switches.
      background: {
        ...theme.background,
        enabled: false,
        path: "",
        dataUrl: "",
      },
    };
    void setUserThemePresets([...userPresets, newPreset]);
    // Switch the live theme name to the new preset so the "modified" tag
    // disappears until the user edits something again.
    void setCustomTheme({ ...theme, name: finalName });
    setSavePresetName(null);
  };

  const onDeleteUserPreset = (name: string) => {
    void setUserThemePresets(userPresets.filter((p) => p.name !== name));
    // If the live theme was the deleted preset, mark it as "(deleted preset)"
    // so the user understands its provenance vanished. They can keep editing
    // or pick another preset.
    if (theme.name === name) {
      void setCustomTheme({ ...theme, name: `${name} (deleted)` });
    }
  };

  const onResetToDefault = () => {
    void setCustomTheme(DEFAULT_CUSTOM_THEME);
  };

  return (
    <SettingsAccordion title="Presets" summary={theme.name} defaultOpen>
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-end gap-2">
          <div className="flex items-center gap-1">
            {savePresetName === null ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-7 gap-1 px-2 text-[11px]"
                onClick={() => {
                  // Seed the input with the current name minus any
                  // "(modified)" suffix so a quick Enter saves over.
                  const seed = theme.name.replace(/\s*\(modified\)\s*$/i, "");
                  setSavePresetName(seed);
                }}
              >
                <BookmarkPlus size={12} strokeWidth={1.75} />
                Save as preset
              </Button>
            ) : (
              <div className="flex items-center gap-1">
                <Input
                  value={savePresetName}
                  onChange={(e) => setSavePresetName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      onSaveAsPreset(savePresetName);
                    }
                    if (e.key === "Escape") {
                      e.preventDefault();
                      setSavePresetName(null);
                    }
                  }}
                  placeholder="Preset name"
                  autoFocus
                  spellCheck={false}
                  className="h-7 w-40 text-[11.5px]"
                />
                <Button
                  type="button"
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  disabled={!savePresetName.trim()}
                  onClick={() => onSaveAsPreset(savePresetName)}
                >
                  Save
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-7 px-2 text-[11px]"
                  onClick={() => setSavePresetName(null)}
                >
                  Cancel
                </Button>
              </div>
            )}
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="h-7 px-2 text-[11px]"
              onClick={onResetToDefault}
            >
              Reset
            </Button>
          </div>
        </div>
        <div className="grid grid-cols-1 gap-1.5 sm:grid-cols-2">
          {[
            ...THEME_PRESETS.map((p) => ({ preset: p, deletable: false })),
            ...userPresets.map((p) => ({ preset: p, deletable: true })),
          ].map(({ preset: p, deletable }) => {
            const active = p.name === theme.name;
            return (
              <div key={p.name} className="group relative">
                <button
                  type="button"
                  onClick={() => onPickPreset(p)}
                  aria-pressed={active}
                  className={cn(
                    "bg-card focus-visible:ring-ring/40 flex w-full items-center gap-2 border px-2 py-1.5 text-left transition-colors focus-visible:ring-2 focus-visible:outline-none",
                    active
                      ? "border-primary ring-primary/40 ring-1"
                      : "border-border/60 hover:border-border",
                  )}
                >
                  <PalettePreview
                    background={p.dark.background}
                    dots={[
                      p.dark.button,
                      p.dark.accent,
                      p.dark.ansiRed,
                      p.dark.ansiGreen,
                      p.dark.ansiYellow,
                      p.dark.ansiBlue,
                      p.dark.ansiMagenta,
                    ]}
                  />
                  <div className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate text-[12px] font-medium">{p.name}</span>
                    <span className="text-muted-foreground text-[10px] tracking-wider uppercase">
                      {deletable ? "your preset" : "light / dark"}
                    </span>
                  </div>
                </button>
                {deletable ? (
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          onDeleteUserPreset(p.name);
                        }}
                        className={cn(
                          DESTRUCTIVE_ACTION,
                          "bg-background/90 border-border/60 absolute top-1 right-1 hidden size-5 cursor-pointer items-center justify-center border transition-colors group-hover:flex",
                        )}
                        aria-label={`Delete preset ${p.name}`}
                      >
                        <X size={10} strokeWidth={2} />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent side="top">Delete &ldquo;{p.name}&rdquo;</TooltipContent>
                  </Tooltip>
                ) : null}
              </div>
            );
          })}
        </div>
      </div>
    </SettingsAccordion>
  );
}
