import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { DESTRUCTIVE_ACTION } from "@/lib/toolbarButton";
import { cn } from "@/lib/utils";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { previewAppOpacity } from "@/modules/settings/appOpacity";
import { setAppOpacity } from "@/modules/settings/mutations";
import { APP_OPACITY_MAX, APP_OPACITY_MIN, APP_OPACITY_STEP } from "@/modules/settings/schema";
import type { CustomTheme } from "@/modules/settings/theme/model";
import { previewWallpaper } from "@/modules/settings/theme/preview";
import { Image, Trash2 } from "lucide-react";
import { useState } from "react";
import { SettingsAccordion } from "../../components/SettingsAccordion";
import { UploadButton } from "../../components/UploadButton";
import { CompactSliderRow } from "./Sliders";

type Props = {
  background: CustomTheme["background"];
  updateBackground: (patch: Partial<CustomTheme["background"]>) => void;
  ensureWallpaperVisible: () => void;
  onPickBackground: () => Promise<void>;
  bgError: string | null;
};

/**
 * The Background & wallpaper accordion: one app-wide opacity slider, the
 * wallpaper source picker, and the wallpaper adjustment sliders.
 */
export function WallpaperPanel({
  background,
  updateBackground,
  ensureWallpaperVisible,
  onPickBackground,
  bgError,
}: Props) {
  const appOpacity = usePreferencesStore((s) => s.appOpacity);
  // Live % while dragging the transparency slider (committed value persists on
  // release; the live fade comes from the previewAppOpacity CSS channel).
  const [opacityPreview, setOpacityPreview] = useState<number | null>(null);

  const onClearBackground = () => {
    updateBackground({ enabled: false, path: "", dataUrl: "" });
  };

  return (
    <SettingsAccordion title="Background &amp; wallpaper">
      <div className="flex flex-col gap-2">
        {/* One opacity control for the whole app. 0% = fully see-through
         *  (reveals the image below, or the desktop when none is set),
         *  100% = solid. Each step writes + broadcasts the value, so the main
         *  window fades live as you drag and the value sticks. */}
        <div className="border-border/60 bg-card flex flex-col gap-1.5 rounded-lg border px-3 py-2.5">
          <CompactSliderRow
            label="Opacity"
            valueLabel={`${Math.round((opacityPreview ?? appOpacity) * 100)}%`}
            value={appOpacity}
            min={APP_OPACITY_MIN}
            max={APP_OPACITY_MAX}
            step={APP_OPACITY_STEP}
            onPreview={(n) => {
              setOpacityPreview(n); // live % label
              previewAppOpacity(n); // live main-window fade (CSS only, no store write)
            }}
            onCommit={(n) => {
              setOpacityPreview(null);
              void setAppOpacity(n); // persist once on release (Radix fires this reliably)
            }}
          />
          <span className="text-muted-foreground text-[10.5px]">
            0% = fully transparent (shows the image below, or your desktop). Applies to everything:
            the header, the panes, the status bar, panels and menus.
          </span>
        </div>
        {/* Wallpaper source: a local image, picked through the file dialog and
         *  read as a `data:` URL by `fs_read_file`. The Switch flips the layer
         *  on/off without losing the underlying source. */}
        <div className="border-border/60 bg-card flex flex-col gap-2 rounded-lg border px-3 py-2.5">
          <div className="flex items-center gap-2">
            <Input
              readOnly
              value={background.path}
              placeholder="Browse for a local image"
              spellCheck={false}
              className="h-8 flex-1 font-mono text-[11px]"
              aria-label="Wallpaper source"
            />
            <Tooltip>
              <TooltipTrigger asChild>
                <UploadButton icon={Image} onClick={() => void onPickBackground()}>
                  Browse
                </UploadButton>
              </TooltipTrigger>
              <TooltipContent side="top">
                Pick a local image (PNG / JPG / GIF / WebP / AVIF, max 10 MB). Animated GIFs play as
                a live wallpaper. Inlined as a data URI in your prefs.
              </TooltipContent>
            </Tooltip>
            {background.dataUrl ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="sm"
                    className={cn(DESTRUCTIVE_ACTION, "h-8 px-2 text-[11px]")}
                    onClick={onClearBackground}
                    aria-label="Clear background"
                  >
                    <Trash2 size={12} strokeWidth={1.75} />
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="top">Clear wallpaper</TooltipContent>
              </Tooltip>
            ) : null}
            <Switch
              checked={background.enabled && !!background.dataUrl}
              disabled={!background.dataUrl}
              onCheckedChange={(v) => {
                updateBackground({ enabled: v });
                if (v) ensureWallpaperVisible();
              }}
              aria-label="Toggle background image"
            />
          </div>
          {/* Current source line - a faint indicator so the user can tell
           *  which image is the wallpaper. */}
          {background.dataUrl ? (
            <div className="text-muted-foreground truncate text-[10.5px]">
              {background.path.startsWith("data:")
                ? "Local image (inlined)"
                : `Source: ${background.path}`}
            </div>
          ) : null}
        </div>
        {bgError ? <span className="text-destructive text-[10.5px]">{bgError}</span> : null}
        {/* Wallpaper adjustments - grouped into a single card with inline
         *  rows so three sliders + their labels don't take up 3× the
         *  vertical space of separate `SettingRow`s. */}
        {background.dataUrl ? (
          <div className="border-border/60 bg-card flex flex-col gap-2 rounded-lg border px-3 py-2.5 text-[11.5px]">
            <CompactSliderRow
              label="Blur"
              valueLabel={`${background.blur}px`}
              value={background.blur}
              min={0}
              max={40}
              step={1}
              onPreview={(n) =>
                // Live-preview on the main window's real wallpaper layer
                // (kind-aware via applyBackground). The old code poked
                // `#subclave-bg-layer` directly, but that element doesn't exist
                // in the Settings webview, so the preview was a no-op. We send
                // only the numbers; the main window merges them onto the
                // wallpaper it already holds (no image blob over IPC).
                previewWallpaper({
                  blur: n,
                  darken: background.darken ?? 0,
                  opacity: background.opacity ?? 1,
                })
              }
              onCommit={(n) => updateBackground({ blur: n })}
            />
            <CompactSliderRow
              label="Darken"
              valueLabel={`${Math.round((background.darken ?? 0) * 100)}%`}
              value={background.darken ?? 0}
              min={0}
              max={1}
              step={0.05}
              onPreview={(n) =>
                previewWallpaper({
                  blur: background.blur,
                  darken: n,
                  opacity: background.opacity ?? 1,
                })
              }
              onCommit={(n) => updateBackground({ darken: n })}
            />
            <CompactSliderRow
              label="Image opacity"
              valueLabel={`${Math.round((background.opacity ?? 1) * 100)}%`}
              value={background.opacity ?? 1}
              min={0}
              max={1}
              step={0.05}
              onPreview={(n) =>
                previewWallpaper({
                  blur: background.blur,
                  darken: background.darken ?? 0,
                  opacity: n,
                })
              }
              onCommit={(n) => updateBackground({ opacity: n })}
            />
            <span className="text-muted-foreground text-[10.5px]">
              Image opacity lets your desktop show through the wallpaper itself (100% = the image
              fully hides the desktop). Combine with App opacity above to layer both.
            </span>
          </div>
        ) : null}
      </div>
    </SettingsAccordion>
  );
}
