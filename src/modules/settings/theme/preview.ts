/**
 * Transient cross-window channel for live wallpaper blur / darken / opacity
 * dragging.
 *
 * The Theme settings UI runs in its OWN webview, which has no wallpaper layer
 * (`applyBackground` removes it there). So a settings-side slider can't paint
 * the real wallpaper directly - it broadcasts only the in-flight numeric values
 * and the main window re-applies them against the wallpaper it already holds,
 * mirroring the opacity slider's `previewAppOpacity` channel. Deliberately
 * carries NO `dataUrl`: the image blob can be multiple MB, and serialising it
 * over IPC on every drag tick (~60/s) would be very heavy - the main window
 * already has it cached. No store write happens while dragging; the committed
 * value persists on release via the normal `customTheme` path.
 */
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";

const WALLPAPER_PREVIEW_EVENT = "subclave://wallpaper-preview";

export type WallpaperPreview = {
  blur: number;
  darken: number;
  opacity: number;
};

/** Settings window: broadcast in-flight blur/darken/opacity for the main window. */
export function previewWallpaper(p: WallpaperPreview): void {
  void emit(WALLPAPER_PREVIEW_EVENT, p);
}

/** Main window: subscribe to live wallpaper blur/darken/opacity previews. */
export function onWallpaperPreview(cb: (p: WallpaperPreview) => void): Promise<UnlistenFn> {
  return listen<WallpaperPreview>(WALLPAPER_PREVIEW_EVENT, (e) => cb(e.payload));
}
