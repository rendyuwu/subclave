// The fill path, shared by the popup and the fill command. No framework, no
// storage: the service worker owns everything else.
//
// Field discovery walks the document, open shadow roots and same-origin iframes
// (`contentDocument`); filling goes through the native value setter so a
// framework listening to `input`/`change` sees a real edit.

export type FillResult = { username: boolean; password: boolean };

function rootsOf(doc: Document): Array<Document | ShadowRoot> {
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

function findAllInputs(doc: Document): HTMLInputElement[] {
  const found: HTMLInputElement[] = [];
  for (const root of rootsOf(doc)) {
    for (const element of Array.from(root.querySelectorAll("input"))) {
      // `querySelectorAll("input")` already filtered by tag; the cast is the
      // standard DOM-node narrowing, and it stays local.
      found.push(element as HTMLInputElement);
    }
  }
  return found;
}

function inputType(input: HTMLInputElement): string {
  return (input.getAttribute("type") ?? "text").toLowerCase();
}

/** Visible, non-zero-size and not fully off-screen; never filled, on any path. */
function isVisible(input: HTMLInputElement): boolean {
  const rect = input.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) return false;
  const style = getComputedStyle(input);
  if (style.visibility === "hidden" || style.display === "none") return false;
  if (input.checkVisibility?.({ checkOpacity: true, checkVisibilityCSS: true }) === false) return false;
  const view = input.ownerDocument.defaultView;
  if (
    view &&
    (rect.right <= 0 || rect.bottom <= 0 || rect.left >= view.innerWidth || rect.top >= view.innerHeight)
  ) {
    return false;
  }
  return true;
}

function isUsernameType(input: HTMLInputElement): boolean {
  const type = inputType(input);
  return type === "text" || type === "email";
}

/** The first visible text-like input before the password field, in the same
 * form (or the same container), preferring an explicit autocomplete hint. */
function usernamePartner(inputs: HTMLInputElement[], passwordInput: HTMLInputElement): HTMLInputElement | null {
  const passwordAt = inputs.findIndex((input) => input === passwordInput);
  const candidates = inputs
    .slice(0, passwordAt < 0 ? inputs.length : passwordAt)
    .filter((input) => isVisible(input) && isUsernameType(input));
  const form = passwordInput.form ?? passwordInput.closest("form");
  const inScope = candidates.filter((input) => (input.form ?? input.closest("form")) === form);
  const scoped = inScope.length > 0 ? inScope : candidates;
  const hinted = scoped.find((input) =>
    ["username", "email"].includes((input.getAttribute("autocomplete") ?? "").toLowerCase()),
  );
  return hinted ?? scoped[0] ?? null;
}

function setValue(input: HTMLInputElement, value: string): void {
  const proto = input.ownerDocument.defaultView?.HTMLInputElement.prototype ?? HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
  if (setter) setter.call(input, value);
  else input.value = value;
  const EventCtor = input.ownerDocument.defaultView?.Event ?? Event;
  input.dispatchEvent(new EventCtor("input", { bubbles: true }));
  input.dispatchEvent(new EventCtor("change", { bubbles: true }));
}

export function fillCredential(username: string, password: string): FillResult {
  const inputs = findAllInputs(document);
  const passwords = inputs.filter((input) => isVisible(input) && inputType(input) === "password");
  const passwordField = passwords[0] ?? null;
  const userField = passwordField
    ? usernamePartner(inputs, passwordField)
    : (inputs.find((input) => isVisible(input) && isUsernameType(input)) ?? null);

  const result: FillResult = { username: false, password: false };
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

/** Every visible password input in the focused form, or the first form that has
 * one; returns the username field's current value and the filled count. */
export function generateFill(password: string): { username: string; filled: number } {
  const inputs = findAllInputs(document);
  const passwords = inputs.filter((input) => isVisible(input) && inputType(input) === "password");
  if (passwords.length === 0) return { username: "", filled: 0 };

  const active = document.activeElement;
  let scope: HTMLFormElement | null = active instanceof HTMLElement ? active.closest("form") : null;
  if (!scope || !passwords.some((input) => input.form === scope)) {
    scope = passwords[0].form;
  }
  const targets = scope ? passwords.filter((input) => input.form === scope) : passwords;

  for (const target of targets) setValue(target, password);
  const userField = usernamePartner(inputs, targets[0]);
  return { username: userField?.value ?? "", filled: targets.length };
}
