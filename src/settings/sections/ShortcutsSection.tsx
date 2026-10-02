import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { usePreferencesStore } from "@/modules/settings/preferences";
import { setShortcuts } from "@/modules/settings/mutations";
import {
  SHORTCUTS,
  SHORTCUT_GROUPS,
  type KeyBinding,
  type ShortcutId,
} from "@/modules/shortcuts/shortcuts";
import { useMemo, useState } from "react";
import { SectionHeader } from "../components/SectionHeader";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { CornerUpLeft, Search } from "lucide-react";
import { ShortcutRow } from "./shortcuts/ShortcutRow";

export function ShortcutsSection() {
  const userShortcuts = usePreferencesStore((s) => s.shortcuts);
  const [search, setSearch] = useState("");
  const [recordingId, setRecordingId] = useState<ShortcutId | null>(null);
  const [resetDialogOpen, setResetDialogOpen] = useState(false);

  const filteredShortcuts = useMemo(() => {
    const base = SHORTCUTS;
    if (!search) return base;
    const lower = search.toLowerCase();
    return base.filter(
      (s) => s.label.toLowerCase().includes(lower) || s.group.toLowerCase().includes(lower),
    );
  }, [search]);

  const onRecord = (id: ShortcutId, binding: KeyBinding) => {
    const next = { ...userShortcuts, [id]: [binding] };
    void setShortcuts(next);
    setRecordingId(null);
  };

  const onClear = (id: ShortcutId) => {
    const next = { ...userShortcuts, [id]: [] };
    void setShortcuts(next);
  };

  const onResetShortcut = (id: ShortcutId) => {
    const next = { ...userShortcuts };
    delete next[id];
    void setShortcuts(next);
  };

  const onResetAll = () => {
    void setShortcuts({});
    setResetDialogOpen(false);
  };

  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-center justify-between">
        <SectionHeader title="Shortcuts" description="View and customize keyboard shortcuts." />
        <Button
          variant="outline"
          size="sm"
          className="h-8 gap-1.5 px-2.5 text-[11px]"
          onClick={() => setResetDialogOpen(true)}
        >
          <CornerUpLeft size={12} strokeWidth={2} />
          Reset All
        </Button>
      </div>

      <div className="relative">
        <Search
          size={14}
          strokeWidth={2}
          className="text-muted-foreground absolute top-1/2 left-3 -translate-y-1/2"
        />
        <Input
          placeholder="Search shortcuts..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="h-9 pl-9 text-[12.5px]"
        />
      </div>

      <div className="flex flex-col gap-8">
        {SHORTCUT_GROUPS.map((group) => {
          const items = filteredShortcuts.filter((s) => s.group === group);
          if (items.length === 0) return null;

          return (
            <div key={group} className="flex flex-col gap-3">
              <h3 className="text-muted-foreground text-[11px] font-semibold tracking-wider uppercase">
                {group}
              </h3>
              <div className="divide-border/40 border-border/60 bg-card/40 flex flex-col divide-y overflow-hidden rounded-lg border">
                {items.map((s) => (
                  <ShortcutRow
                    key={s.id}
                    shortcut={s}
                    isRecording={recordingId === s.id}
                    onStartRecording={() => setRecordingId(s.id)}
                    onStopRecording={() => setRecordingId(null)}
                    onRecord={(b) => onRecord(s.id, b)}
                    onClear={() => onClear(s.id)}
                    onReset={() => onResetShortcut(s.id)}
                    userBindings={userShortcuts[s.id]}
                  />
                ))}
              </div>
            </div>
          );
        })}
      </div>

      <AlertDialog open={resetDialogOpen} onOpenChange={setResetDialogOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Reset all shortcuts?</AlertDialogTitle>
            <AlertDialogDescription>
              This will revert all your custom keyboard shortcuts to their factory defaults. This
              action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={onResetAll}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Reset All
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
