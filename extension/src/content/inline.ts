// The inline icon and picker: an icon in each detected login field, and a
// top-layer picker of the page's exact-host logins. After a trusted sign-in
// submit, the same picker shows the save prompt (Add, Update, Cancel) in the
// viewport's corner. A fill, generate, update, Add or Update row runs only
// after every guard in `guard.ts` passes. Matching, credential release and
// every write stay in the service worker and Rust.
//
// Everything is built with `createElement`/`textContent` inside one closed
// shadow root. Placement goes through `element.style` (CSSOM, which a page CSP
// does not block), so icons and picker stay put against the viewport even when
// the shadow `<style>` with the colours and fonts is refused.

import { sendToBackground } from "../lib/messages";
import type { InlineRequest, SavePrompt, SwResponse, SwState } from "../lib/messages";
import type { NmErrorCode, SaveCandidate } from "../lib/protocol";
import { scan, topRect } from "./detect";
import {
  autocompleteTokens,
  findAllInputs,
  hasAutocompleteToken,
  inputType,
  isRendered,
  rootsOf,
  scopeOf,
  usernamePartner,
} from "./fill";
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
/** How long a page that saw a sign-in waits before showing the prompt
 * itself; most sign-ins navigate first, and the next page's script shows it. */
const SUBMIT_PROMPT_DELAY_MS = 1000;
/** Longer page-supplied values are never captured: they would only fail the
 * native frame cap and fill `chrome.storage.session`. */
const MAX_SUBMITTED_LENGTH = 1024;
/** The controls whose click submits their form. */
const SUBMIT_BUTTON =
  'button:not([type]), button[type="submit"], input[type="submit"], input[type="image"]';
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
.heading { margin: 0; padding: 6px 10px 2px; font-weight: 600; }
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

/** The form's rendered inputs in document order, reached the way detection
 * reaches them (open shadow roots, same-origin iframes). */
function formInputs(form: HTMLFormElement): HTMLInputElement[] {
  return findAllInputs(rootsOf(document)).filter((i) => isRendered(i) && scopeOf(i) === form);
}

/** The rendered username and password `form` holds, or `null` without a
 * filled password or without a username (a password-only form is the second
 * step of a two-step login, and an update from it would blank the stored
 * username). A password is an input detection once saw as one (`seen`), so a
 * password shown as text at submit still counts; card codes and one-time
 * codes never do. A change-password form gives its new password: the first
 * `new-password` field with a value, else the last filled password
 * (current, new, repeat). */
function submittedLogin(
  form: HTMLFormElement,
  seen: WeakSet<HTMLInputElement>,
): { username: string; password: string } | null {
  const own = formInputs(form);
  const passwords = own.filter(
    (i) =>
      (inputType(i) === "password" || seen.has(i)) &&
      i.value !== "" &&
      !autocompleteTokens(i).some((token) => token.startsWith("cc-") || token === "one-time-code"),
  );
  if (passwords.length === 0) return null;
  const password = (
    passwords.find((i) => hasAutocompleteToken(i, "new-password")) ??
    passwords[passwords.length - 1]
  ).value;
  const username = usernamePartner(own, passwords[0])?.value.trim() ?? "";
  if (
    username === "" ||
    username.length > MAX_SUBMITTED_LENGTH ||
    password.length > MAX_SUBMITTED_LENGTH
  ) {
    return null;
  }
  return { username, password };
}

/** A real user gesture within the activation window. `navigator.userActivation`
 * is Chrome 72+ and Firefox 120+, both below the Popover API floor
 * `startInline` requires; it is read through a named interface. */
function userActive(): boolean {
  const nav: Navigator & { userActivation?: { isActive: boolean } } = navigator;
  return nav.userActivation?.isActive === true;
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
  const heading = document.createElement("p");
  heading.className = "heading";
  heading.setAttribute("role", "status");
  picker.append(heading, list, notes);
  // The page field keeps focus while the pointer works the picker.
  picker.addEventListener("mousedown", (event) => event.preventDefault());
  shadow.append(style, picker);
  document.documentElement.append(host);
  const glyph = keyGlyph();

  const icons = new Map<HTMLInputElement, HTMLButtonElement>();
  /** Detected field -> is a new-password field. */
  let fields = new Map<HTMLInputElement, boolean>();
  const wired = new WeakSet<HTMLInputElement>();
  const wiredForms = new WeakSet<HTMLFormElement>();
  /** Every input detection ever saw as a password, for `submittedLogin`. */
  const seenPasswords = new WeakSet<HTMLInputElement>();
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
  /** The save prompt on show; `listing` once the user asked to pick the entry
   * to update. The prompt is the picker open with no anchor. */
  let saving: { prompt: SavePrompt; listing: boolean } | null = null;
  let submitSeq = 0;
  /** The fill command's shortcut ("" when unbound) once the service worker
   * could not open the popup for this page; the more-logins footer is text
   * from then on. */
  // ponytail: remembered per page, so a browser that can never open the popup
  // shows the row until its first click on each page; have the service worker
  // learn it up front if that matters.
  let popupFallback: string | null = null;

  const isOpen = (): boolean => picker.matches(":popover-open");

  const close = (): void => {
    if (isOpen()) picker.hidePopover();
    guard.disarm();
    anchor = null;
    anchorRect = null;
    rows = [];
    saving = null;
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
    if (!anchor) {
      // The save prompt: the viewport's top-right corner, placed again by
      // `queueReflow` when the viewport resizes.
      const width = Math.min(PICKER_MIN_WIDTH, innerWidth - 8);
      picker.style.width = `${width}px`;
      picker.style.left = `${Math.max(4, document.documentElement.clientWidth - width - 8)}px`;
      picker.style.top = "8px";
      return;
    }
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
   * timer, so the delay runs from when pickable rows appear. Visibility of
   * the heading goes through `hidden`: a page CSP can refuse the `<style>`. */
  const render = (next: Row[], lines: HTMLElement[], head = ""): void => {
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
    heading.textContent = head;
    heading.hidden = head === "";
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
    const message = note(
      saving
        ? "Subclave did not accept that click."
        : "Use the Subclave toolbar button to fill on this page.",
      "message",
    );
    message.dataset.guard = String(id);
    // The prompt's rows come back (restarting guard 1's delay) so the user
    // can try again.
    if (saving) showPrompt([message]);
    else render([], [message]);
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

  /** Opens the toolbar popup, which lists the wider matches in browser-owned
   * UI. Fills and releases nothing, so its row needs a trusted event only. */
  const openPopup = async (state: Extract<SwState, { state: "ready" }>): Promise<void> => {
    const mine = seq;
    const response = await ask({ type: "inline-open-popup" });
    if (!response || mine !== seq) return;
    if (response.type === "popup" && response.ok) close();
    else if (response.type === "popup") {
      popupFallback = response.shortcut;
      showReady(state);
    } else if (response.type === "error") {
      errorView(response.code, response.message);
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
      const count = `${more} more login${more === 1 ? "" : "s"} on ${state.domain}`;
      if (popupFallback === null) {
        next.push({
          label: count,
          detail: "Open in Subclave popup",
          guarded: false,
          run: () => openPopup(state),
        });
      } else {
        const press = popupFallback ? `, or press ${popupFallback}` : "";
        lines.push(note(`${count}: open Subclave from the browser's Extensions menu${press}`));
      }
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

  /** Saves the pending sign-in `id`, the one the prompt shows. */
  const save = async (id: string, entryId: string | null): Promise<void> => {
    const mine = seq;
    const response = await ask({ type: "inline-save", id, entryId });
    if (!response || mine !== seq) return;
    if (response.type === "save" && response.ok) close();
    else if (response.type === "save" || response.type === "error") {
      showPrompt([note(response.message)]);
    }
  };

  const cancelSave = async (id: string): Promise<void> => {
    close();
    await ask({ type: "inline-save-cancel", id });
  };

  /** Add and Update write to the vault, so they pass every guard; "Update an
   * existing login" and Cancel write nothing and need a trusted event only. */
  const showPrompt = (lines: HTMLElement[] = []): void => {
    if (!saving) return;
    const { prompt, listing } = saving;
    const update = (entry: SaveCandidate): Row => ({
      label: `Update ${entry.title}`,
      detail: entry.username,
      guarded: true,
      run: () => save(prompt.id, entry.id),
    });
    const add: Row = {
      label: "Add",
      detail: prompt.username,
      guarded: true,
      run: () => save(prompt.id, null),
    };
    const cancel: Row = { label: "Cancel", guarded: false, run: () => cancelSave(prompt.id) };
    const choose: Row = {
      label: "Update an existing login",
      guarded: false,
      run: async () => {
        if (saving) {
          saving.listing = true;
          showPrompt();
        }
      },
    };
    const same = prompt.entries.filter((entry) => entry.username === prompt.username);
    let next: Row[];
    if (listing) next = [...prompt.entries.map(update), cancel];
    else if (same.length === 1) next = [update(same[0]), add, cancel];
    else if (prompt.entries.length > 0) next = [add, choose, cancel];
    else next = [add, cancel];
    render(next, lines, `Save login for ${prompt.host}?`);
  };

  /** Shows the tab's pending sign-in, if the service worker has one to offer.
   * Never takes focus. */
  const promptSave = async (): Promise<void> => {
    const response = await ask({ type: "inline-pending-save" });
    if (!running || response?.type !== "save-prompt" || !response.prompt) return;
    // The user is working the picker, or guard 2 would refuse every Add and
    // Update: the pair stays pending for a later page load.
    if ((anchor && isOpen()) || otherTopLayer()) return;
    close();
    // Before `arm`, so the tamper observer never sees this move.
    if (document.documentElement.lastElementChild !== host) document.documentElement.append(host);
    saving = { prompt: response.prompt, listing: false };
    showPrompt();
    picker.showPopover();
    place();
    guard.arm();
  };

  const submitted = async (
    form: HTMLFormElement,
    login: { username: string; password: string },
  ): Promise<void> => {
    // A page can plant values and submit them from its own `mousedown` on a
    // prompt row: closing in that same task leaves the click nothing to land
    // on, and the pair id the rows carry backs it up.
    if (saving) close();
    // The click-then-submit pair of one sign-in sends two identical messages;
    // only the last one keeps its timer.
    const mine = (submitSeq += 1);
    const response = await ask({ type: "inline-submitted", ...login });
    if (!response || !running) return;
    setTimeout(() => {
      // Still in the form: not signed in yet (a show-password toggle, a refused
      // password), so only a later page load shows the prompt.
      const stillTyped = formInputs(form).some((input) => input.value === login.password);
      if (mine === submitSeq && running && !stillTyped) void promptSave();
    }, SUBMIT_PROMPT_DELAY_MS);
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
      else if (!anchor && isOpen()) {
        // The save prompt follows the viewport's corner; moved under the
        // pointer, guard 1's delay starts over, as for the anchored picker.
        const { left, width } = picker.style;
        place();
        if (picker.style.left !== left || picker.style.width !== width) guard.markShown();
      }
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

  // `isTrusted` alone is not enough: `requestSubmit()` and `button.click()`
  // from page script fire a trusted `submit`, so a real user gesture must be
  // active too. The click path covers sites that sign in by script from the
  // button's click and never fire `submit`. Nothing here calls
  // `preventDefault`.
  const wireForm = (form: HTMLFormElement): void => {
    const capture = (event: Event): void => {
      if (!running || !event.isTrusted || !userActive()) return;
      const login = submittedLogin(form, seenPasswords);
      if (login) void submitted(form, login);
    };
    form.addEventListener("submit", capture, true);
    form.addEventListener(
      "click",
      (event) => {
        // A click target is always an element.
        const target = event.target as Element | null;
        if (target?.closest(SUBMIT_BUTTON)) capture(event);
      },
      true,
    );
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
      if (inputType(input) === "password") seenPasswords.add(input);
      if (!icons.has(input)) {
        const icon = makeIcon(input);
        shadow.append(icon);
        icons.set(input, icon);
      }
      if (!wired.has(input)) {
        wired.add(input);
        wire(input);
      }
      const form = input.form;
      if (form && !wiredForms.has(form)) {
        wiredForms.add(form);
        wireForm(form);
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
  // A sign-in this tab submitted on an earlier page.
  void promptSave();

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
