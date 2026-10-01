import type { FsReadResult } from "@/lib/ipc";
import { slugify } from "@/lib/utils";
import { setCustomTheme, setCustomThemeEnabled } from "@/modules/settings/mutations";
import {
  parseThemeFile,
  serializeThemeFile,
  type CustomTheme,
} from "@/modules/settings/theme/model";
import { invoke } from "@tauri-apps/api/core";
import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { useState } from "react";

type ReadResult = FsReadResult;

type Options = {
  theme: CustomTheme;
  enabled: boolean;
  updateBackground: (patch: Partial<CustomTheme["background"]>) => void;
  ensureWallpaperVisible: () => void;
};

/**
 * The three file-dialog flows of the Theme section: pick a wallpaper, import a
 * `.subclave` theme, export the current theme. Each flow owns its own error /
 * status line, returned for the callers that render them.
 */
export function useThemeFileIo({
  theme,
  enabled,
  updateBackground,
  ensureWallpaperVisible,
}: Options) {
  const [bgError, setBgError] = useState<string | null>(null);
  const [importError, setImportError] = useState<string | null>(null);
  const [importStatus, setImportStatus] = useState<string | null>(null);

  const onPickBackground = async () => {
    setBgError(null);
    try {
      const selected = await openFileDialog({
        multiple: false,
        filters: [
          {
            name: "Image",
            extensions: ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "avif"],
          },
        ],
      });
      const path = typeof selected === "string" ? selected : null;
      if (!path) return;
      const result = await invoke<ReadResult>("fs_read_file", { path });
      if (result.kind === "image") {
        updateBackground({ enabled: true, path, dataUrl: result.dataUrl });
        ensureWallpaperVisible();
      } else if (result.kind === "toolarge") {
        setBgError(
          `Image is ${(result.size / (1024 * 1024)).toFixed(1)} MB, over the ${(
            result.limit /
            (1024 * 1024)
          ).toFixed(0)} MB limit.`,
        );
      } else {
        setBgError("Selected file isn't a recognised image.");
      }
    } catch (e) {
      setBgError(e instanceof Error ? e.message : String(e));
    }
  };

  const onImportFromDialog = async () => {
    setImportError(null);
    setImportStatus(null);
    try {
      const selected = await openFileDialog({
        multiple: false,
        filters: [{ name: "Subclave theme", extensions: ["subclave", "json"] }],
      });
      const path = typeof selected === "string" ? selected : null;
      if (!path) return;
      const result = await invoke<ReadResult>("fs_read_file", { path });
      if (result.kind !== "text") {
        setImportError("Theme file is not a UTF-8 text file.");
        return;
      }
      const parsed = parseThemeFile(JSON.parse(result.content), theme);
      void setCustomTheme(parsed);
      if (!enabled) void setCustomThemeEnabled(true);
      setImportStatus(`Imported "${parsed.name}".`);
    } catch (e) {
      setImportError(e instanceof Error ? e.message : String(e));
    }
  };

  const onExport = async () => {
    try {
      const target = await saveFileDialog({
        defaultPath: `${slugify(theme.name, "theme")}.subclave`,
        filters: [{ name: "Subclave theme", extensions: ["subclave"] }],
      });
      if (!target) return;
      // Drop inline `data:` blobs because they are typically multi-MB base64
      // and bloat the theme file with non-portable bytes. Also blank `path`
      // (it holds the exporter's absolute OS file path - a privacy leak) and
      // turn the layer off so the recipient doesn't get an enabled-but-empty
      // wallpaper.
      const slim: CustomTheme = {
        ...theme,
        background: { ...theme.background, enabled: false, path: "", dataUrl: "" },
      };
      const json = serializeThemeFile(slim);
      await invoke<void>("fs_write_file", { path: target, content: json });
      setImportStatus(`Exported to ${target}`);
    } catch (e) {
      setImportError(e instanceof Error ? e.message : String(e));
    }
  };

  return { onPickBackground, onImportFromDialog, onExport, bgError, importError, importStatus };
}
