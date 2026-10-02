// The six clickjacking guards every inline pick passes before a fill. A failed
// guard refuses the pick and points at the toolbar button; it never retries.
// Nothing here touches the DOM at module scope, so Node can import the
// constants.

import { rootsOf } from "./fill";

export const GUARD_DELAY_MS = 500;

/**
 * 1 the event is untrusted, or came less than `GUARD_DELAY_MS` after the
 *   picker was shown or moved;
 * 2 another popover, modal dialog or fullscreen element is in the top layer;
 * 3 `<html>`, `<body>` or the host is faded, hidden, filtered, blended or
 *   clipped;
 * 4 the page changed the host's attributes or moved it;
 * 5 the pointer is not over the host;
 * 6 IntersectionObserver v2 does not report the picker visible (Chromium only).
 */
export type GuardId = 1 | 2 | 3 | 4 | 5 | 6;

type Guard = {
  arm(): void;
  markShown(): void;
  disarm(): void;
  check(event: Event, point: { x: number; y: number } | null): GuardId | null;
};

/** True when any reachable root has something else in the top layer. The
 * extension's own closed root is never one `rootsOf` returns. */
export function otherTopLayer(): boolean {
  return rootsOf(document).some(
    (root) => root.querySelector(":popover-open, :modal, :fullscreen") !== null,
  );
}

export function createGuard(host: HTMLElement, picker: HTMLElement, onTamper: () => void): Guard {
  // Read here, not at module scope: Node has no `IntersectionObserverEntry`.
  const supportsV2 = "isVisible" in IntersectionObserverEntry.prototype;
  let shownAt = 0;
  let tampered = false;
  let visible: boolean | null = null;
  let visibility: IntersectionObserver | null = null;

  // A counted record is an attribute change on the host, or a child-list change
  // on `<html>` that took the host out (a move is a removal plus an insertion).
  const absorb = (records: MutationRecord[]): void => {
    for (const record of records) {
      if (
        record.type === "attributes"
          ? record.target === host
          : Array.from(record.removedNodes).includes(host)
      ) {
        tampered = true;
      }
    }
  };
  const tamper = new MutationObserver((records) => {
    absorb(records);
    if (tampered) onTamper();
  });

  const disarm = (): void => {
    tamper.disconnect();
    visibility?.disconnect();
    visibility = null;
    visible = null;
  };

  return {
    arm() {
      disarm();
      shownAt = performance.now();
      tampered = false;
      tamper.observe(host, { attributes: true });
      tamper.observe(document.documentElement, { childList: true });
      if (supportsV2) {
        const init: IntersectionObserverInit & { trackVisibility: boolean; delay: number } = {
          trackVisibility: true,
          delay: 100,
        };
        visibility = new IntersectionObserver((entries) => {
          const last: (IntersectionObserverEntry & { isVisible?: boolean }) | undefined =
            entries.at(-1);
          if (last) visible = last.isVisible === true;
        }, init);
        visibility.observe(picker);
      }
    },
    markShown() {
      shownAt = performance.now();
    },
    disarm,
    check(event, point) {
      if (!event.isTrusted || event.timeStamp - shownAt < GUARD_DELAY_MS) return 1;
      if (otherTopLayer()) return 2;
      for (const element of [document.documentElement, document.body, host]) {
        // `document.body` is typed non-null but is null on a document with no
        // `<body>` yet.
        if (!element) continue;
        const style = getComputedStyle(element);
        if (parseFloat(style.opacity) !== 1 || style.visibility !== "visible") return 3;
      }
      const hostStyle = getComputedStyle(host);
      if (
        hostStyle.filter !== "none" ||
        hostStyle.mixBlendMode !== "normal" ||
        hostStyle.clipPath !== "none"
      ) {
        return 3;
      }
      // The mutation records are the whole guard: the picker sits in the top
      // layer at its own fixed coordinates, so moving the 0x0 host cannot move it.
      absorb(tamper.takeRecords());
      if (tampered) return 4;
      // Clearing `pointer-events: none` from other popovers first would never
      // change the outcome: guard 2 already refuses every popover this script
      // can find. Popovers inside closed roots cannot be found, and guard 6
      // covers them on Chromium.
      if (point && document.elementsFromPoint(point.x, point.y)[0] !== host) return 5;
      if (supportsV2 && visible !== true) return 6;
      return null;
    },
  };
}
