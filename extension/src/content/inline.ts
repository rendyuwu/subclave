// The inline icon and picker: an icon in each detected login field, and a
// top-layer picker of the page's exact-host logins. A fill, generate or update
// row runs only after every guard in `guard.ts` passes. Matching, credential
// release and every write stay in the service worker and Rust.
//
// Everything is built with `createElement`/`textContent` inside one closed
// shadow root. Placement goes through `element.style` (CSSOM, which a page CSP
// does not block), so icons and picker stay put against the viewport even when
// the shadow `<style>` with the colours and fonts is refused.

import { sendToBackground } from "../lib/messages";
import type { InlineRequest, SwResponse, SwState } from "../lib/messages";
import type { NmErrorCode } from "../lib/protocol";
import { scan, topRect } from "./detect";
import { createGuard, otherTopLayer } from "./guard";
import type { GuardId } from "./guard";

const ICON_SIZE = 18;
const PICKER_MIN_WIDTH = 280;
/** Trailing debounce between a page mutation and the next detection pass,
 * capped so a page that never stops mutating still gets a pass. */
const DETECT_DEBOUNCE_MS = 250;
const DETECT_MAX_WAIT_MS = 1000;
/** How long after `load` a page with no login field keeps watching. Single-page
 * apps render their form after `load`: Discord, X and Stripe took 1.5 to 2.2 s
 * when this was measured. */
const SETTLE_AFTER_LOAD_MS = 5000;
const DETECT_ATTRIBUTES = ["type", "style", "class", "hidden", "open", "autocomplete"];
const SVG_NS = "http://www.w3.org/2000/svg";
const KEY_PATH =
  "M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z";

// No box-shadow, opacity, filter or transform on the picker: IntersectionObserver
// v2 (guard 6) reports any of them as not visible.
const CSS = `
:host { all: initial !important; position: fixed !important; top: 0 !important; left: 0 !important; width: 0 !important; height: 0 !important; z-index: 2147483647 !important; }
.icon { display: flex; align-items: center; justify-content: center; box-sizing: border-box; padding: 0; border: 0; border-radius: 0; background: #0057fe; color: #fff; cursor: pointer; }
.picker { box-sizing: border-box; padding: 4px 0; border: 1px solid #c8c8c8; border-radius: 0; background: #fff; color: #1a1a1a; font: 13px/1.4 system-ui, sans-serif; }
[role="listbox"] { outline: none; }
[role="listbox"]:focus-visible { outline: 2px solid #0057fe; outline-offset: -2px; }
[role="option"] { display: block; padding: 6px 10px; cursor: pointer; }
[role="option"][aria-selected="true"] { background: #e5edff; }
.title, .detail { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.detail, .notes p { color: #555; }
.notes:empty { display: none; }
.notes p { margin: 0; padding: 6px 10px; }
.notes .code { color: inherit; font: 600 20px/1.4 ui-monospace, monospace; letter-spacing: 0.1em; }
@media (prefers-color-scheme: dark) {
  .picker { background: #1a1a1a; color: #f2f2f2; border-color: #3a3a3a; }
  [role="option"][aria-selected="true"] { background: #20325c; }
  .detail, .notes p { color: #b0b0b0; }
}
`;

type Row = { label: string; detail?: string; guarded: boolean; run: () => Promise<void> };

type Inline = {
  stop(): void;
  pending(): HTMLInputElement | null;
  showPairingCode(code: string): void;
};

let current: Inline | null = null;

/** Draws the icons and starts detection. Browsers without the Popover API
 * (Firefox before 125, Chrome before 114) get no inline UI; the popup and the
 * fill command still work there. */
export function startInline(): void {
  if (current || !("showPopover" in HTMLElement.prototype)) return;
  current = createInline();
}

/** Tears the UI down. Field and document listeners stay and do nothing. */
export function stopInline(): void {
  const inline = current;
  current = null;
  inline?.stop();
}

/** The field an in-flight inline fill or generate belongs to, else `null`. */
export function pickAnchor(): HTMLInputElement | null {
  return current?.pending() ?? null;
}

export function showPairingCode(code: string): void {
  current?.showPairingCode(code);
}

function keyGlyph(): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, "svg");
  const attributes: Record<string, string> = {
    width: "14",
    height: "14",
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    "stroke-width": "2",
    "stroke-linecap": "round",
    "stroke-linejoin": "round",
    "aria-hidden": "true",
  };
  for (const [name, value] of Object.entries(attributes)) svg.setAttribute(name, value);
  const path = document.createElementNS(SVG_NS, "path");
  path.setAttribute("d", KEY_PATH);
  const dot = document.createElementNS(SVG_NS, "circle");
  dot.setAttribute("cx", "16.5");
  dot.setAttribute("cy", "7.5");
  dot.setAttribute("r", ".5");
  dot.setAttribute("fill", "currentColor");
  svg.append(path, dot);
  return svg;
}

function createInline(): Inline {
  let running = true;

  // The host never gets an attribute after this point: guard 4 watches it.
  const host = document.createElement("subclave-inline");
  const shadow = host.attachShadow({ mode: "closed" });
  const style = document.createElement("style");
  style.textContent = CSS;
  const picker = document.createElement("div");
  picker.className = "picker";
  picker.setAttribute("popover", "manual");
  Object.assign(picker.style, {
    position: "fixed",
    inset: "auto",
    margin: "0",
    height: "auto",
    maxHeight: "320px",
    overflow: "auto",
  });
  const list = document.createElement("div");
  list.setAttribute("role", "listbox");
  list.tabIndex = 0;
  list.setAttribute("aria-label", "Subclave logins");
  const notes = document.createElement("div");
  notes.className = "notes";
  notes.setAttribute("role", "status");
  notes.setAttribute("aria-live", "polite");
  picker.append(list, notes);
  // The page field keeps focus while the pointer works the picker.
  picker.addEventListener("mousedown", (event) => event.preventDefault());
  shadow.append(style, picker);
  document.documentElement.append(host);
  const glyph = keyGlyph();

  const icons = new Map<HTMLInputElement, HTMLButtonElement>();
  /** Detected field -> is a new-password field. */
  let fields = new Map<HTMLInputElement, boolean>();
  const wired = new WeakSet<HTMLInputElement>();
  const watched = new WeakSet<Document>();
  let observed = new WeakSet<Document | ShadowRoot>();
  let generation = 0;
  /** When the current mutation burst began, for the detection max wait. */
  let burstStart: number | null = null;
  let found = false;
  let idle = false;
  let reflowQueued = false;

  let anchor: HTMLInputElement | null = null;
  let anchorRect: DOMRect | null = null;
  let rows: Row[] = [];
  let active = 0;
  let seq = 0;
  let pending: HTMLInputElement | null = null;

  const isOpen = (): boolean => picker.matches(":popover-open");

  const close = (): void => {
    if (isOpen()) picker.hidePopover();
    guard.disarm();
    anchor = null;
    anchorRect = null;
    rows = [];
    seq += 1;
  };

  // A host attribute change or move closes the picker outright.
  const guard = createGuard(host, picker, close);

  /** The anchor is 1 px or more away from where the picker was placed. */
  const moved = (): boolean => {
    if (!anchor || !anchorRect) return false;
    const now = topRect(anchor);
    return (
      Math.abs(now.left - anchorRect.left) >= 1 ||
      Math.abs(now.top - anchorRect.top) >= 1 ||
      Math.abs(now.width - anchorRect.width) >= 1 ||
      Math.abs(now.height - anchorRect.height) >= 1
    );
  };

  const place = (): void => {
    if (!anchor) return;
    const rect = topRect(anchor);
    anchorRect = rect;
    const width = Math.min(Math.max(rect.width, PICKER_MIN_WIDTH), innerWidth - 8);
    picker.style.width = `${width}px`;
    picker.style.left = `${Math.min(Math.max(rect.left, 4), innerWidth - width - 4)}px`;
    const height = picker.getBoundingClientRect().height;
    const below = rect.bottom + 2;
    const above = rect.top - height - 2;
    picker.style.top = `${below + height > innerHeight && above >= 0 ? above : below}px`;
  };

  const note = (text: string, className = ""): HTMLParagraphElement => {
    const line = document.createElement("p");
    line.textContent = text;
    if (className) line.className = className;
    return line;
  };

  /** Every render while open re-places the picker and restarts guard 1's
   * timer, so the delay runs from when pickable rows appear. */
  const render = (next: Row[], lines: HTMLElement[]): void => {
    rows = next;
    active = 0;
    list.replaceChildren(
      ...next.map((row, index) => {
        const option = document.createElement("div");
        option.id = `subclave-opt-${index}`;
        option.setAttribute("role", "option");
        option.dataset.index = String(index);
        option.setAttribute("aria-selected", String(index === 0));
        const title = document.createElement("span");
        title.className = "title";
        title.textContent = row.label;
        option.append(title);
        if (row.detail) {
          const detail = document.createElement("span");
          detail.className = "detail";
          detail.textContent = row.detail;
          option.append(detail);
        }
        return option;
      }),
    );
    if (next.length > 0) list.setAttribute("aria-activedescendant", "subclave-opt-0");
    else list.removeAttribute("aria-activedescendant");
    notes.replaceChildren(...lines);
    if (isOpen()) {
      place();
      guard.markShown();
    }
  };

  /** Moves the highlight in place. Never re-renders and never restarts guard
   * 1's timer: the pointer move before every click, or the arrow keys before
   * Enter, would otherwise get every pick refused. */
  const setActive = (index: number): void => {
    const options = list.children;
    options[active]?.setAttribute("aria-selected", "false");
    active = index;
    const option = options[index];
    if (!(option instanceof HTMLElement)) return;
    option.setAttribute("aria-selected", "true");
    list.setAttribute("aria-activedescendant", option.id);
    // Scrolled by hand: `scrollIntoView` could scroll the page too, and so
    // move the anchor out from under the picker.
    const bottom = option.offsetTop + option.offsetHeight;
    if (option.offsetTop < picker.scrollTop) picker.scrollTop = option.offsetTop;
    else if (bottom > picker.scrollTop + picker.clientHeight) {
      picker.scrollTop = bottom - picker.clientHeight;
    }
  };

  const refuse = (id: GuardId): void => {
    const message = note("Use the Subclave toolbar button to fill on this page.", "message");
    message.dataset.guard = String(id);
    render([], [message]);
  };

  /** `null` once the extension was updated or reloaded under this page: this
   * copy of the script can never reach the service worker again, so its UI
   * goes away. */
  const ask = async (request: InlineRequest): Promise<SwResponse | null> => {
    try {
      return await sendToBackground<SwResponse>(request);
    } catch {
      stopInline();
      return null;
    }
  };

  const errorView = (code: NmErrorCode, message: string): void => {
    if (code === "vault-locked") showState({ state: "locked" });
    else if (code === "app-not-running") showState({ state: "not-running" });
    else if (code === "not-associated" || code === "auth-failed") showState({ state: "unpaired" });
    else render([], [note(message)]);
  };

  const loadLogins = async (mine: number): Promise<void> => {
    const response = await ask({ type: "inline-logins" });
    if (!response || mine !== seq) return;
    if (response.type === "state") showState(response.state);
    else if (response.type === "error") errorView(response.code, response.message);
  };

  const fill = async (entryId: string): Promise<void> => {
    const mine = seq;
    pending = anchor;
    const response = await ask({ type: "inline-fill", entryId });
    pending = null;
    if (!response || mine !== seq) return;
    if (response.type === "fill") {
      if (!response.ok) errorView(response.code, response.message);
      else if (response.filled === 0) render([], [note("No login fields found on this page.")]);
      else close();
    } else if (response.type === "error") {
      errorView(response.code, response.message);
    }
  };

  const generate = async (entryId: string | null): Promise<void> => {
    const mine = seq;
    pending = anchor;
    const response = await ask({ type: "inline-generate", entryId });
    pending = null;
    if (!response || mine !== seq) return;
    if (response.type === "generate") {
      if (!response.ok) errorView(response.code, response.message);
      else
        render(
          [],
          [
            note(
              response.filled === 0
                ? "No login fields found on this page."
                : "Password saved to Subclave",
            ),
          ],
        );
    } else if (response.type === "error") {
      errorView(response.code, response.message);
    }
  };

  const unlock = async (): Promise<void> => {
    const mine = seq;
    await ask({ type: "inline-focus-app" });
    if (mine === seq) close();
  };

  const pairNow = async (): Promise<void> => {
    const mine = seq;
    render([], [note("Waiting for Subclave")]);
    const response = await ask({ type: "inline-pair" });
    if (!response || mine !== seq) return;
    if (response.type === "pair") {
      if (response.ok) await loadLogins(mine);
      else render([], [note(response.message)]);
    } else if (response.type === "error") {
      render([], [note(response.message)]);
    }
  };

  const showReady = (state: Extract<SwState, { state: "ready" }>): void => {
    const next: Row[] = [];
    if (anchor && fields.get(anchor) === true) {
      if (state.entries.length === 0) {
        next.push({ label: "Generate for this site", guarded: true, run: () => generate(null) });
      } else {
        for (const entry of state.entries) {
          next.push({
            label: `Update ${entry.title}`,
            guarded: true,
            run: () => generate(entry.id),
          });
        }
        next.push({ label: "New entry", guarded: true, run: () => generate(null) });
      }
    }
    for (const entry of state.entries) {
      next.push({
        label: entry.title,
        detail: `${entry.username} ${entry.group}`,
        guarded: true,
        run: () => fill(entry.id),
      });
    }
    const lines: HTMLElement[] = [];
    if (state.entries.length === 0) lines.push(note(`No logins for ${state.host}.`));
    const more = state.otherMatches;
    if (more > 0) {
      lines.push(
        note(
          `${more} more login${more === 1 ? "" : "s"} on ${state.domain}: use the Subclave toolbar button`,
        ),
      );
    }
    render(next, lines);
  };

  const showState = (state: SwState): void => {
    switch (state.state) {
      case "locked":
        render([{ label: "Unlock", guarded: false, run: unlock }], [note("Subclave is locked")]);
        return;
      case "not-running":
        render([], [note("Subclave is not running. Start it from your applications menu.")]);
        return;
      case "unpaired":
        render([{ label: "Pair with Subclave", guarded: false, run: pairNow }], []);
        return;
      case "error":
        render([], [note(state.message)]);
        return;
      case "ready":
        // A modal dialog makes the picker inert, so this refusal is the only
        // feedback the user can get.
        if (otherTopLayer()) refuse(2);
        else showReady(state);
    }
  };

  const open = async (field: HTMLInputElement, focusList: boolean): Promise<void> => {
    if (anchor === field && isOpen()) {
      if (focusList) list.focus({ preventScroll: true });
      return;
    }
    close();
    // Before `arm`, so the tamper observer never sees this move.
    if (document.documentElement.lastElementChild !== host) document.documentElement.append(host);
    anchor = field;
    const mine = (seq += 1);
    render([], [note("Loading...")]);
    picker.showPopover();
    place();
    guard.arm();
    if (focusList) list.focus({ preventScroll: true });
    await loadLogins(mine);
  };

  const pick = async (
    index: number,
    event: Event,
    point: { x: number; y: number } | null,
  ): Promise<void> => {
    const row = rows[index];
    // No row: the tamper observer's microtask closed the picker between a page
    // listener and this one. Pending: one fill or generate at a time, since a
    // second pick while the first is in flight would hand the first fill the
    // second picker's field.
    if (!row || pending) return;
    // The picker moved under the pointer: guard 1's delay starts over.
    if (moved()) {
      place();
      guard.markShown();
    }
    if (row.guarded) {
      const failed = guard.check(event, point);
      if (failed !== null) {
        refuse(failed);
        return;
      }
    } else if (!event.isTrusted) {
      return;
    }
    render([], [note("Loading...")]);
    await row.run();
  };

  const optionIndex = (target: EventTarget | null): number | null => {
    const option =
      target instanceof Element ? target.closest<HTMLElement>('[role="option"]') : null;
    return option ? Number(option.dataset.index) : null;
  };

  list.addEventListener("pointermove", (event) => {
    const index = optionIndex(event.target);
    if (index !== null && index !== active) setActive(index);
  });
  list.addEventListener("click", (event) => {
    const index = optionIndex(event.target);
    if (index !== null) void pick(index, event, { x: event.clientX, y: event.clientY });
  });
  list.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (rows.length > 0) {
        setActive((active + (event.key === "ArrowDown" ? 1 : rows.length - 1)) % rows.length);
      }
    } else if (event.key === "Enter") {
      event.preventDefault();
      void pick(active, event, null);
    } else if (event.key === "Escape") {
      event.preventDefault();
      const field = anchor;
      close();
      field?.focus({ preventScroll: true });
    }
  });

  const positionIcons = (): void => {
    for (const [input, icon] of icons) {
      const rect = topRect(input);
      icon.style.left = `${rect.right - ICON_SIZE - 4}px`;
      icon.style.top = `${rect.top + (rect.height - ICON_SIZE) / 2}px`;
    }
  };

  const queueReflow = (): void => {
    if (!running || reflowQueued) return;
    reflowQueued = true;
    requestAnimationFrame(() => {
      reflowQueued = false;
      if (!running) return;
      positionIcons();
      if (anchor && isOpen() && moved()) close();
    });
  };

  // Only the last mutation batch inside the window runs a pass; the
  // generation check replaces a stored timer handle.
  const schedule = (): void => {
    if (!running || idle) return;
    const now = performance.now();
    burstStart ??= now;
    const wait = Math.min(DETECT_DEBOUNCE_MS, burstStart + DETECT_MAX_WAIT_MS - now);
    if (wait <= 0) {
      pass();
      return;
    }
    const mine = (generation += 1);
    setTimeout(() => {
      if (mine === generation) pass();
    }, wait);
  };

  const detector = new MutationObserver(schedule);

  const makeIcon = (input: HTMLInputElement): HTMLButtonElement => {
    const icon = document.createElement("button");
    icon.type = "button";
    icon.className = "icon";
    icon.tabIndex = -1;
    icon.setAttribute("aria-label", "Subclave: fill login");
    Object.assign(icon.style, {
      position: "fixed",
      inset: "auto",
      margin: "0",
      width: `${ICON_SIZE}px`,
      height: `${ICON_SIZE}px`,
    });
    icon.append(glyph.cloneNode(true));
    icon.addEventListener("pointerdown", (event) => {
      if (running && event.isTrusted && event.button === 0) void open(input, false);
    });
    icon.addEventListener("mousedown", (event) => event.preventDefault());
    return icon;
  };

  // Programmatic focus never opens anything: there is no `focus` listener.
  const wire = (input: HTMLInputElement): void => {
    input.addEventListener("pointerdown", (event) => {
      if (running && fields.has(input) && event.isTrusted && event.button === 0) {
        void open(input, false);
      }
    });
    input.addEventListener("keydown", (event) => {
      if (!running || !fields.has(input) || !event.isTrusted) return;
      const plain = !event.altKey && !event.ctrlKey && !event.metaKey && !event.shiftKey;
      if (event.key === "ArrowDown" && plain) {
        event.preventDefault();
        void open(input, true);
      } else if (event.key === "Escape" && anchor === input && isOpen()) {
        close();
      }
    });
  };

  const watch = (doc: Document): void => {
    if (watched.has(doc)) return;
    watched.add(doc);
    doc.addEventListener(
      "pointerdown",
      (event) => {
        if (!running || !anchor || !isOpen()) return;
        const path = event.composedPath();
        if (!path.includes(host) && !path.includes(anchor)) close();
      },
      true,
    );
    doc.addEventListener("scroll", queueReflow, { capture: true, passive: true });
    // A same-origin iframe that loads after the first pass.
    doc.addEventListener("load", schedule, true);
  };

  const pass = (): void => {
    if (!running) return;
    // A pass covers every mutation so far: drop pending timers and end the burst.
    generation += 1;
    burstStart = null;
    const { fields: detected, roots } = scan();
    for (const root of roots) {
      if (observed.has(root)) continue;
      observed.add(root);
      detector.observe(root, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: DETECT_ATTRIBUTES,
      });
    }
    const next = new Map<HTMLInputElement, boolean>(
      detected.map((field) => [field.input, field.newPassword]),
    );
    for (const [input, icon] of icons) {
      if (next.has(input)) continue;
      icon.remove();
      icons.delete(input);
    }
    for (const { input } of detected) {
      if (!icons.has(input)) {
        const icon = makeIcon(input);
        shadow.append(icon);
        icons.set(input, icon);
      }
      if (!wired.has(input)) {
        wired.add(input);
        wire(input);
      }
      watch(input.ownerDocument);
    }
    fields = next;
    if (next.size > 0) found = true;
    positionIcons();
    if (anchor && isOpen()) {
      if (!fields.has(anchor)) close();
      else if (moved()) {
        place();
        guard.markShown();
      }
    }
  };

  // A page with no login field a few seconds after it has loaded stops
  // watching for good; one that ever had a field keeps watching (two-step
  // forms, show/hide toggles).
  const settle = (): void => {
    if (!running || found) return;
    detector.disconnect();
    idle = true;
    generation += 1;
  };
  const settleLater = (): void => {
    setTimeout(settle, SETTLE_AFTER_LOAD_MS);
  };

  watch(document);
  window.addEventListener("resize", queueReflow);
  pass();
  if (document.readyState === "complete") {
    settleLater();
  } else {
    // `load` waits for subframes, so late same-origin iframes are included.
    window.addEventListener(
      "load",
      () => {
        pass();
        settleLater();
      },
      { once: true },
    );
  }

  return {
    stop() {
      close();
      running = false;
      detector.disconnect();
      observed = new WeakSet();
      host.remove();
      icons.clear();
      fields = new Map();
    },
    pending: () => pending,
    showPairingCode(code) {
      if (!running || !isOpen()) return;
      render([], [note("Waiting for Subclave"), note(code, "code")]);
    },
  };
}
