import { Button } from "@/components/ui/button";
import { IconTooltip } from "@/components/ui/icon-tooltip";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import { DESTRUCTIVE_ACTION } from "@/lib/toolbarButton";
import { cn } from "@/lib/utils";
import { getBindingTokens, type KeyBinding, type Shortcut } from "@/modules/shortcuts/shortcuts";
import { CornerUpLeft, Trash2 } from "lucide-react";
import { Recorder } from "./Recorder";

export function ShortcutRow({
  shortcut,
  isRecording,
  onStartRecording,
  onStopRecording,
  onRecord,
  onClear,
  onReset,
  userBindings,
}: {
  shortcut: Shortcut;
  isRecording: boolean;
  onStartRecording: () => void;
  onStopRecording: () => void;
  onRecord: (b: KeyBinding) => void;
  onClear: () => void;
  onReset: () => void;
  userBindings?: KeyBinding[];
}) {
  const bindings = userBindings !== undefined ? userBindings : shortcut.defaultBindings;
  const isModified = userBindings !== undefined;
  const hasBindings = bindings && bindings.length > 0;

  return (
    <div className="group hover:bg-muted/30 flex items-center justify-between px-3 py-2.5 transition-colors">
      <span className="text-[12.5px] font-medium">{shortcut.label}</span>

      <div className="flex items-center gap-2">
        {isRecording ? (
          <Recorder onRecord={onRecord} onCancel={onStopRecording} />
        ) : (
          <>
            <div
              role="button"
              tabIndex={0}
              onClick={onStartRecording}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onStartRecording();
                }
              }}
              className="flex min-w-[100px] cursor-pointer items-center justify-end gap-1"
            >
              {hasBindings ? (
                <KbdGroup>
                  {getBindingTokens(bindings[0]).map((t, i) => (
                    <Kbd
                      key={i}
                      className="group-hover:bg-accent group-hover:text-accent-foreground transition-colors"
                    >
                      {t}
                    </Kbd>
                  ))}
                </KbdGroup>
              ) : (
                <span className="text-muted-foreground text-[11px] italic">Unassigned</span>
              )}
            </div>

            <div className="flex items-center gap-1">
              {isModified && (
                <IconTooltip label="Reset to default" side="left">
                  <Button
                    variant="ghost"
                    size="icon"
                    className="text-muted-foreground hover:text-foreground size-7"
                    onClick={onReset}
                    aria-label="Reset to default"
                  >
                    <CornerUpLeft size={12} />
                  </Button>
                </IconTooltip>
              )}
              <IconTooltip label="Clear shortcut" side="left">
                <Button
                  variant="ghost"
                  size="icon"
                  className={cn(
                    DESTRUCTIVE_ACTION,
                    "size-7 opacity-0 transition-opacity group-hover:opacity-100",
                  )}
                  onClick={onClear}
                  aria-label="Clear shortcut"
                >
                  <Trash2 size={12} />
                </Button>
              </IconTooltip>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
