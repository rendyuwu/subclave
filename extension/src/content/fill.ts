// The fill path, shared by the popup, the fill command and the inline picker.
// No framework, no storage: the service worker owns everything else.
//
// Field discovery walks the document, open shadow roots and same-origin iframes
// (`contentDocument`); filling goes through the native value setter so a
// framework listening to `input`/`change` sees a real edit.

export type FillResult = { username: boolean; password: boolean };

export function rootsOf(doc: Document): Array<Document | ShadowRoot> {
  const roots: Array<Document | ShadowRoot> = [doc];
  const visit = (root: Document | ShadowRoot): void => {
    for (const element of Array.from(root.querySelectorAll("*"))) {
      const shadow = element.shadowRoot;
      if (shadow && !roots.includes(shadow)) {
        roots.push(shadow);
        visit(shadow);
      }
      if (element instanceof HTMLIFrameElement) {
        let inner: Document | null = null;
        try {
          inner = element.contentDocument;
        } catch {
          inner = null;
        }
        if (inner && !roots.includes(inner)) {
          roots.push(inner);
          visit(inner);
        }
      }
    }
  };
  visit(doc);
  return roots;
}

export function findAllInputs(roots: Array<Document | ShadowRoot>): HTMLInputElement[] {
  const found: HTMLInputElement[] = [];
  for (const root of roots) {
    for (const element of Array.from(root.querySelectorAll("input"))) {
      // `querySelectorAll("input")` already filtered by tag; the cast is the
      // standard DOM-node narrowing, and it stays local.
      found.push(element as HTMLInputElement);
    }
  }
  return found;
}

export function inputType(input: HTMLInputElement): string {
  return (input.getAttribute("type") ?? "text").toLowerCase();
}

/** Laid out at a non-zero size, not hidden by CSS, not parked at negative
 * page coordinates (an off-screen honeypot), and not a decoy taken out of both
 * the tab order and the accessibility tree: a field the user can scroll to and
 * reach. Detection uses this, so a login form below the fold still gets its
 * icon. */
export function isRendered(input: HTMLInputElement): boolean {
  if (input.tabIndex < 0 && input.closest('[aria-hidden="true"]')) return false;
  const rect = input.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return false;
  const style = getComputedStyle(input);
  if (style.visibility === "hidden" || style.display === "none") return false;
  if (input.checkVisibility?.({ checkOpacity: true, checkVisibilityCSS: true }) === false)
    return false;
  const view = input.ownerDocument.defaultView;
  return !view || (rect.right + view.scrollX > 0 && rect.bottom + view.scrollY > 0);
}

/** Rendered and inside the viewport right now; never filled otherwise, on any
 * path. */
export function isVisible(input: HTMLInputElement): boolean {
  if (!isRendered(input)) return false;
  const rect = input.getBoundingClientRect();
  const view = input.ownerDocument.defaultView;
  return (
    !view ||
    (rect.right > 0 &&
      rect.bottom > 0 &&
      rect.left < view.innerWidth &&
      rect.top < view.innerHeight)
  );
}

export function isUsernameType(input: HTMLInputElement): boolean {
  const type = inputType(input);
  return type === "text" || type === "email";
}

/** The input's form or, for a formless input, its document or shadow root, so
 * formless inputs in different roots or frames never count as one form. */
export function scopeOf(input: HTMLInputElement): Node {
  return input.form ?? input.closest("form") ?? input.getRootNode();
}

/** `autocomplete` is a token list (`"username webauthn"`), not one value. */
export function autocompleteTokens(input: HTMLInputElement): string[] {
  return (input.getAttribute("autocomplete") ?? "").toLowerCase().split(/\s+/);
}

export function hasAutocompleteToken(input: HTMLInputElement, token: string): boolean {
  return autocompleteTokens(input).includes(token);
}

/** The first text-like input before the password field among `inputs`, which
 * the caller has already filtered (for visibility, and on an anchored path for
 * the anchor's scope), preferring the password's own scope and an explicit
 * autocomplete hint. */
export function usernamePartner(
  inputs: HTMLInputElement[],
  passwordInput: HTMLInputElement,
): HTMLInputElement | null {
  const passwordAt = inputs.indexOf(passwordInput);
  const candidates = inputs
    .slice(0, passwordAt < 0 ? inputs.length : passwordAt)
    .filter(isUsernameType);
  const scope = scopeOf(passwordInput);
  const inScope = candidates.filter((input) => scopeOf(input) === scope);
  const scoped = inScope.length > 0 ? inScope : candidates;
  const hinted = scoped.find(
    (input) => hasAutocompleteToken(input, "username") || hasAutocompleteToken(input, "email"),
  );
  return hinted ?? scoped[0] ?? null;
}

function setValue(input: HTMLInputElement, value: string): void {
  const proto =
    input.ownerDocument.defaultView?.HTMLInputElement.prototype ?? HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
  if (setter) setter.call(input, value);
  else input.value = value;
  const EventCtor = input.ownerDocument.defaultView?.Event ?? Event;
  input.dispatchEvent(new EventCtor("input", { bubbles: true }));
  input.dispatchEvent(new EventCtor("change", { bubbles: true }));
}

/** With an `anchor` (the inline picker's field) the fill stays in that field's
 * scope (`scopeOf`) and never falls back to a page-wide search: a vanished
 * anchor fills nothing. Without one (popup, command) the first login form on
 * the page wins. */
export function fillCredential(
  username: string,
  password: string,
  anchor: HTMLInputElement | null = null,
): FillResult {
  const result: FillResult = { username: false, password: false };
  const inputs = findAllInputs(rootsOf(document)).filter(isVisible);
  let passwordField: HTMLInputElement | null;
  let userField: HTMLInputElement | null;
  if (anchor) {
    if (!isVisible(anchor)) return result;
    const scope = scopeOf(anchor);
    const own = inputs.filter((input) => scopeOf(input) === scope);
    if (inputType(anchor) === "password") {
      passwordField = anchor;
      userField = usernamePartner(own, anchor);
    } else {
      // A username anchor fills only its own form's password, so the first step
      // of a two-step login fills the username alone.
      userField = anchor;
      passwordField = own.find((input) => inputType(input) === "password") ?? null;
    }
  } else {
    passwordField = inputs.find((input) => inputType(input) === "password") ?? null;
    userField = passwordField
      ? usernamePartner(inputs, passwordField)
      : (inputs.find(isUsernameType) ?? null);
  }

  if (username && userField) {
    setValue(userField, username);
    result.username = true;
  }
  if (password && passwordField) {
    setValue(passwordField, password);
    result.password = true;
  }
  return result;
}

/** Every visible password input in the anchor's scope, or (without an anchor)
 * in the focused form or the first form that has one; returns the username
 * field's current value and the filled count. */
export function generateFill(
  password: string,
  anchor: HTMLInputElement | null = null,
): { username: string; filled: number } {
  let pool = findAllInputs(rootsOf(document)).filter(isVisible);
  let targets: HTMLInputElement[];
  if (anchor) {
    // `scopeOf`, not `instanceof`: an anchor in a same-origin iframe belongs to
    // another realm.
    const scope = scopeOf(anchor);
    pool = isVisible(anchor) ? pool.filter((input) => scopeOf(input) === scope) : [];
    targets = pool.filter((input) => inputType(input) === "password");
  } else {
    const passwords = pool.filter((input) => inputType(input) === "password");
    if (passwords.length === 0) return { username: "", filled: 0 };
    const active = document.activeElement;
    let scope: HTMLFormElement | null =
      active instanceof HTMLElement ? active.closest("form") : null;
    if (!scope || !passwords.some((input) => input.form === scope)) {
      scope = passwords[0].form;
    }
    targets = scope ? passwords.filter((input) => input.form === scope) : passwords;
  }
  if (targets.length === 0) return { username: "", filled: 0 };

  for (const target of targets) setValue(target, password);
  const userField = usernamePartner(pool, targets[0]);
  return { username: userField?.value ?? "", filled: targets.length };
}
