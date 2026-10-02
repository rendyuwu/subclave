import { Switch } from "@/components/ui/switch";
import { previewAppOpacity } from "@/modules/settings/appOpacity";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { setAppOpacity, setCustomTheme, setCustomThemeEnabled } from "@/modules/settings/mutations";
import { APP_OPACITY_MAX } from "@/modules/settings/schema";
import type { CustomTheme } from "@/modules/settings/theme/model";
import { SectionHeader } from "../components/SectionHeader";
import { SettingRow } from "../components/SettingRow";
import { ColorEditor } from "./theme/ColorEditor";
import { ImportExportCard } from "./theme/ImportExportCard";
import { PresetEditor } from "./theme/PresetEditor";
import { WallpaperPanel } from "./theme/WallpaperPanel";
import { useThemeFileIo } from "./theme/useThemeFileIo";

// Opacity to drop to when a wallpaper is first activated while the app is
// fully solid, so the image is clearly visible without manual fiddling.
const WALLPAPER_REVEAL_OPACITY = 0.5;

export function ThemeSection() {
  const enabled = usePreferencesStore((s) => s.customThemeEnabled);
  const theme = usePreferencesStore((s) => s.customTheme);
  const appOpacity = usePreferencesStore((s) => s.appOpacity);

  const updateBackground = (patch: Partial<CustomTheme["background"]>) => {
    const next: CustomTheme = {
      ...theme,
      background: { ...theme.background, ...patch },
    };
    void setCustomTheme(next);
  };

  // A wallpaper only shows through translucent surfaces, and surfaces are only
  // translucent when the custom theme is on AND opacity < 100%. So whenever a
  // wallpaper becomes active, enable the custom theme and (if still fully
  // solid) drop opacity so the image is visible right away instead of looking
  // like nothing happened.
  const ensureWallpaperVisible = () => {
    if (!enabled) void setCustomThemeEnabled(true);
    if (appOpacity >= APP_OPACITY_MAX) {
      // Reveal instantly via the transient preview channel (CSS only, reaches
      // the main window immediately regardless of store-event routing), then
      // persist the same value so it survives reload. Both are 0.5, so there is
      // no drift between the live preview and the stored value.
      previewAppOpacity(WALLPAPER_REVEAL_OPACITY);
      void setAppOpacity(WALLPAPER_REVEAL_OPACITY);
    }
  };

  const { onPickBackground, onImportFromDialog, onExport, bgError, importError, importStatus } =
    useThemeFileIo({ theme, enabled, updateBackground, ensureWallpaperVisible });

  return (
    <div className="flex flex-col gap-4">
      <SectionHeader
        title="Theme"
        description="Customise every color and add a background image."
      />

      <SettingRow
        title="Enable custom theme"
        description="When off, Subclave uses the default palette tinted by your main color."
      >
        <Switch checked={enabled} onCheckedChange={(v) => void setCustomThemeEnabled(v)} />
      </SettingRow>

      <PresetEditor />

      <ColorEditor />

      <WallpaperPanel
        background={theme.background}
        updateBackground={updateBackground}
        ensureWallpaperVisible={ensureWallpaperVisible}
        onPickBackground={onPickBackground}
        bgError={bgError}
      />

      <ImportExportCard
        onImportFromDialog={onImportFromDialog}
        onExport={onExport}
        importError={importError}
        importStatus={importStatus}
      />
    </div>
  );
}
