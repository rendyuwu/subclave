/**
 * Shared boot preamble for both webview entry points.
 *
 * Importing this module runs the side effects in the order the entries need:
 * the browser shim first (everything below may touch the Tauri API), then the
 * global stylesheet. `bootWindow` then does the rest of the first-paint work and
 * returns the root element to mount React into.
 */
import "@/lib/tauri-browser-shim";
import "@/styles/globals.css";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { USE_CUSTOM_WINDOW_CONTROLS } from "@/lib/platform";
import { applyBrandColorFastPath } from "@/modules/settings/brandColor";
import { applyCustomThemeFastPath } from "@/modules/settings/theme/apply";

export function bootWindow({ rootId }: { rootId: string }): HTMLElement {
  if (USE_CUSTOM_WINDOW_CONTROLS) {
    document.documentElement.dataset.chrome = "borderless";
  }

  applyBrandColorFastPath();
  // Custom theme overrides brand color when active. Run after the brand fast
  // path so its CSS variables win on first paint.
  applyCustomThemeFastPath();

  // The window is hidden at launch (per tauri.conf.json) so users never see a
  // transparent shadow-only frame before React paints. Two rAFs guarantee at
  // least one commit reached the compositor; the wall-clock fallback covers a
  // window whose frames are throttled while hidden, and whichever fires first
  // wins (the guard stops a second show).
  let shown = false;
  const showWindow = () => {
    if (shown) return;
    shown = true;
    getCurrentWindow()
      .show()
      .catch((e) => console.error("window.show failed:", e));
  };
  requestAnimationFrame(() => requestAnimationFrame(showWindow));
  setTimeout(showWindow, 50);

  return document.getElementById(rootId) as HTMLElement;
}
