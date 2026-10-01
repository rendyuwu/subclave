/**
 * App.tsx - the main-window shell.
 *
 * The shell owns the chrome (header, tray, status bar) and hands the tray's
 * whole content to `VaultScreen`, which picks the first-run, unlock or
 * workspace surface from the vault status. The three panes live inside
 * `VaultWorkspace`, not here, so the panel group is only mounted once there
 * is a vault to show.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { Toaster } from "@/components/ui/toast";
import { TooltipProvider } from "@/components/ui/tooltip";
import { CommandPalette } from "@/modules/commandPalette/CommandPalette";
import { Header } from "@/modules/header/Header";
import { openSettingsWindow } from "@/modules/settings/openSettingsWindow";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { useGlobalShortcuts } from "@/modules/shortcuts";
import { StatusBar } from "@/modules/statusbar/StatusBar";
import { startSync } from "@/modules/sync";
import { ThemeProvider } from "@/modules/theme";
import { QuitConfirmDialog } from "@/modules/vault/QuitConfirmDialog";
import { SaveFailedBanner } from "@/modules/vault/SaveFailedBanner";
import { VaultSearchInput } from "@/modules/vault/SearchField";
import { VaultScreen } from "@/modules/vault/VaultScreen";
import { useVaultStore } from "@/modules/vault/store";
import { buildShortcutHandlers } from "./lib/shortcutHandlers";
import { useStoreRecoveryNotices } from "./hooks/useStoreRecoveryNotices";

export default function App() {
  const initPrefs = usePreferencesStore((s) => s.init);
  const initVault = useVaultStore((s) => s.init);
  const lock = useVaultStore((s) => s.lock);
  const status = useVaultStore((s) => s.status);
  const locked = status?.locked ?? true;
  useStoreRecoveryNotices();
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);

  useEffect(() => {
    void initPrefs();
  }, [initPrefs]);

  useEffect(() => {
    void initVault();
  }, [initVault]);

  useEffect(() => startSync(), []);

  const shortcutHandlers = useMemo(
    () => buildShortcutHandlers({ toggleCommandPalette: () => setCommandPaletteOpen((o) => !o) }),
    [],
  );
  useGlobalShortcuts(shortcutHandlers);

  const openSettings = useCallback(() => void openSettingsWindow(), []);

  return (
    <ThemeProvider>
      <TooltipProvider>
        <div className="bg-background text-foreground relative flex h-screen flex-col overflow-hidden">
          <Header onOpenSettings={openSettings} onLock={locked ? undefined : lock}>
            {locked ? null : <VaultSearchInput />}
          </Header>
          <SaveFailedBanner />
          <main className="bg-sidebar flex min-h-0 flex-1 gap-1.5 p-1.5">
            <VaultScreen />
          </main>
          <StatusBar />
          <QuitConfirmDialog />
          <Toaster />
          <CommandPalette open={commandPaletteOpen} onOpenChange={setCommandPaletteOpen} />
        </div>
      </TooltipProvider>
    </ThemeProvider>
  );
}
