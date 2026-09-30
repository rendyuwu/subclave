import { openSettingsWindow } from "@/modules/settings/openSettingsWindow";
import { type ShortcutHandlers } from "@/modules/shortcuts";

/**
 * Component-local identifiers from App that the keyboard-shortcut handler
 * map closes over. Module-level dependencies (openSettingsWindow) are imported
 * directly above and are NOT threaded through here.
 */
export interface ShortcutHandlerDeps {
  toggleCommandPalette: () => void;
}

export function buildShortcutHandlers(deps: ShortcutHandlerDeps): ShortcutHandlers {
  return {
    "settings.open": () => void openSettingsWindow(),
    "commandPalette.open": deps.toggleCommandPalette,
  };
}
