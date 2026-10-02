import { useEffect } from "react";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { GroupTree } from "@/modules/groups/GroupTree";
import { TagStrip } from "@/modules/groups/TagStrip";
import { useGlobalShortcuts } from "@/modules/shortcuts";
import { useVaultCommands } from "./commands";
import { EntryDetail } from "./EntryDetail";
import { EntryList } from "./EntryList";
import { EntryEditorDialog } from "./editor/EntryEditorDialog";
import { useVaultStore } from "./store";

const PANE =
  "border-border/60 bg-background subclave-glass-panel flex h-full min-h-0 flex-col overflow-hidden rounded-md border";

/**
 * The unlocked three-pane workspace: group tree, entry list, entry detail, with
 * the entry editor on top. Owns the vault key commands and the idle-activity
 * touches, so both stop when the vault locks and this unmounts.
 */
export function VaultWorkspace() {
  const editor = useVaultStore((s) => s.editor);
  const closeEditor = useVaultStore((s) => s.closeEditor);
  const touchActivity = useVaultStore((s) => s.touchActivity);

  useGlobalShortcuts(useVaultCommands());

  // Capture phase, so a handler that stops propagation cannot hide activity.
  // `touchActivity` throttles itself to one `vault_touch` per 30 s.
  useEffect(() => {
    window.addEventListener("pointerdown", touchActivity, { capture: true, passive: true });
    window.addEventListener("keydown", touchActivity, { capture: true, passive: true });
    return () => {
      window.removeEventListener("pointerdown", touchActivity, { capture: true });
      window.removeEventListener("keydown", touchActivity, { capture: true });
    };
  }, [touchActivity]);

  return (
    <>
      <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1 gap-1.5">
        <ResizablePanel id="groups" defaultSize="225px" minSize="8%" maxSize="450px">
          <div className={PANE}>
            <GroupTree />
          </div>
        </ResizablePanel>
        <ResizableHandle withHandle />
        <ResizablePanel id="entries" defaultSize="58%" minSize="25%">
          <div className={PANE}>
            <TagStrip />
            <EntryList />
          </div>
        </ResizablePanel>
        <ResizableHandle withHandle />
        <ResizablePanel id="detail" defaultSize="22%" minSize="18%" maxSize="50%">
          <div className={PANE}>
            <EntryDetail />
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
      <EntryEditorDialog request={editor} onClose={closeEditor} />
    </>
  );
}
