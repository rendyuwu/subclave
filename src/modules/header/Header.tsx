import { Button } from "@/components/ui/button";
import { IconTooltip } from "@/components/ui/icon-tooltip";
import { WindowControls } from "@/components/WindowControls";
import { cn } from "@/lib/utils";
import { TOOLBAR_HOVER } from "@/lib/toolbarButton";
import { IS_MAC, USE_CUSTOM_WINDOW_CONTROLS } from "@/lib/platform";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { memo, type ReactNode } from "react";
import { Lock, Settings } from "lucide-react";

type Props = {
  onOpenSettings: () => void;
  /** Renders the lock button. A locked vault passes nothing, so the button
   *  disappears rather than sitting there inert. */
  onLock?: () => void;
  /** Trailing buttons before the lock button; the unlocked shell passes the
   *  import and export menu. */
  actions?: ReactNode;
  /** Slot right before the trailing actions. The unlocked shell puts the
   *  search field here. */
  children?: ReactNode;
};

/**
 * Manual window-drag fallback. Tauri's auto `data-tauri-drag-region` is flaky
 * on WebView2 when the region wraps Radix DOM. Calling `startDragging()`
 * from a React mousedown handler works reliably. Interactive elements and
 * `data-tauri-drag-region="false"` children are excluded.
 */
const INTERACTIVE_SELECTOR =
  'button, a, input, textarea, select, [role="button"], [role="tab"], [role="menuitem"], [data-tauri-drag-region="false"]';

function onHeaderMouseDown(e: React.MouseEvent<HTMLElement>) {
  if (e.button !== 0) return;
  const target = e.target as HTMLElement;
  if (!target?.closest) return;
  // Portaled content (dropdown/context menus, tooltips) renders into
  // `document.body`, but React still bubbles its events up the *React* tree -
  // straight into this handler. Without this guard, selecting text in one of
  // those dialogs drags the window instead, and double-clicking a word
  // maximizes it. Only real DOM descendants of the header row drag.
  if (!e.currentTarget.contains(target)) return;
  if (target.closest(INTERACTIVE_SELECTOR)) return;
  const w = getCurrentWindow();
  if (e.detail === 2) {
    void w.toggleMaximize();
  } else {
    void w.startDragging();
  }
}

function HeaderImpl({ onOpenSettings, onLock, actions, children }: Props) {
  const settingsButton = (
    <IconTooltip label="Settings">
      <Button
        variant="ghost"
        size="icon"
        className={cn("text-muted-foreground", TOOLBAR_HOVER, "size-7 shrink-0 rounded-md")}
        onClick={onOpenSettings}
        aria-label="Settings"
      >
        <Settings size={15} strokeWidth={1.75} />
      </Button>
    </IconTooltip>
  );

  const lockButton = (
    <IconTooltip label="Lock">
      <Button
        variant="ghost"
        size="icon"
        className={cn("text-muted-foreground", TOOLBAR_HOVER, "size-7 shrink-0 rounded-md")}
        onClick={onLock}
        aria-label="Lock"
      >
        <Lock size={15} strokeWidth={1.75} />
      </Button>
    </IconTooltip>
  );

  return (
    <div
      data-subclave-header
      className="border-border/60 bg-card flex shrink-0 flex-col border-b select-none"
    >
      {/* One row: the app mark, the app name, the drag spacer, the slot, the
          trailing actions, the lock and settings buttons and the window
          controls. */}
      <div
        data-tauri-drag-region
        onMouseDown={onHeaderMouseDown}
        className={`flex h-9 shrink-0 items-center gap-2 ${IS_MAC ? "pr-2 pl-20" : "pr-0 pl-2"}`}
      >
        <img
          src="/icon.png"
          alt=""
          aria-hidden="true"
          draggable={false}
          className="size-4 shrink-0"
        />
        <span className="text-[13px] font-semibold">Subclave</span>

        {/* Drag spacer between the app name and the slot, so the slot sits
            beside the trailing icon cluster. */}
        <div data-tauri-drag-region className="h-full min-w-2 flex-1" />

        {children ? (
          <div className="flex max-w-md min-w-0 flex-1 items-center">{children}</div>
        ) : null}

        {actions}

        {onLock ? lockButton : null}

        {settingsButton}

        {USE_CUSTOM_WINDOW_CONTROLS && (
          <>
            <span className="bg-border ml-1 h-5 w-px shrink-0" />
            <WindowControls />
          </>
        )}
      </div>
    </div>
  );
}

/**
 * Memoised so unrelated App.tsx re-renders don't ripple through the header.
 * Callers MUST pass stable callback references (use `useCallback`) or memo
 * is a no-op.
 */
export const Header = memo(HeaderImpl);
