import { canonicalKey, type KeyBinding } from "@/modules/shortcuts/shortcuts";
import { useEffect, useRef } from "react";

export function Recorder({
  onRecord,
  onCancel,
}: {
  onRecord: (b: KeyBinding) => void;
  onCancel: () => void;
}) {
  const onRecordRef = useRef(onRecord);
  const onCancelRef = useRef(onCancel);
  useEffect(() => {
    onRecordRef.current = onRecord;
    onCancelRef.current = onCancel;
  });

  useEffect(() => {
    const onDown = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();

      if (e.key === "Escape") {
        onCancelRef.current();
        return;
      }

      if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;

      // Require at least one primary modifier (Ctrl, Alt, Meta). Reject
      // Shift-only shortcuts that would insert a character.
      const hasPrimaryModifier = e.ctrlKey || e.altKey || e.metaKey;
      const isCharacterKey = e.key.length === 1; // anything that types a glyph
      // Blocks shortcuts like Shift+2 ("@") and Shift+, ("<") on many layouts.
      if (!hasPrimaryModifier && (!e.shiftKey || isCharacterKey)) {
        return;
      }
      // Record the canonical, layout-independent key. Option+Z on macOS or
      // Ctrl+T on a Cyrillic layout would otherwise store the glyph and never re-fire.
      onRecordRef.current({
        key: canonicalKey(e),
        ctrl: e.ctrlKey,
        shift: e.shiftKey,
        alt: e.altKey,
        meta: e.metaKey,
      });
    };

    window.addEventListener("keydown", onDown, { capture: true });
    return () => {
      window.removeEventListener("keydown", onDown, { capture: true });
    };
  }, []);

  return (
    <div className="bg-accent/50 ring-accent flex items-center gap-2 rounded px-2 py-1 text-[11px] ring-1">
      <span className="animate-pulse font-medium">Recording…</span>
      <span className="text-muted-foreground">(Esc to cancel)</span>
    </div>
  );
}
