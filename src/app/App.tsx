/**
 * App.tsx - the main-window shell.
 *
 * An empty shell for now: a header, three empty resizable panes and a status
 * bar, plus the command palette and the modal registry it registers with. The
 * panes stay empty until the vault UI lands.
 */
import { useEffect, useMemo, useState, useCallback } from "react";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { Toaster } from "@/components/ui/toast";
import { TooltipProvider } from "@/components/ui/tooltip";
import { CommandPalette } from "@/modules/commandPalette";
import { Header } from "@/modules/header";
import { openSettingsWindow } from "@/modules/settings/openSettingsWindow";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { useGlobalShortcuts } from "@/modules/shortcuts";
import { StatusBar } from "@/modules/statusbar";
import { ThemeProvider } from "@/modules/theme";
import { buildShortcutHandlers } from "./lib/shortcutHandlers";
import { useStoreRecoveryNotices } from "./hooks/useStoreRecoveryNotices";

/** One empty pane. */
const PANE =
  "border-border/60 bg-background subclave-glass-panel flex h-full min-h-0 flex-col overflow-hidden rounded-md border";

export default function App() {
  const init = usePreferencesStore((s) => s.init);
  useStoreRecoveryNotices();
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);

  useEffect(() => {
    void init();
  }, [init]);

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
          <Header onOpenSettings={openSettings} />
          <main className="bg-sidebar flex min-h-0 flex-1 gap-1.5 p-1.5">
            <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1 gap-1.5">
              <ResizablePanel id="groups" defaultSize="225px" minSize="8%" maxSize="450px">
                <div className={PANE} />
              </ResizablePanel>
              <ResizableHandle withHandle />
              <ResizablePanel id="entries" defaultSize="58%" minSize="25%">
                <div className={PANE} />
              </ResizablePanel>
              <ResizableHandle withHandle />
              <ResizablePanel id="detail" defaultSize="22%" minSize="18%" maxSize="50%">
                <div className={PANE} />
              </ResizablePanel>
            </ResizablePanelGroup>
          </main>
          <StatusBar />
          <Toaster />
          <CommandPalette open={commandPaletteOpen} onOpenChange={setCommandPaletteOpen} />
        </div>
      </TooltipProvider>
    </ThemeProvider>
  );
}
