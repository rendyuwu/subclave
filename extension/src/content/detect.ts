// Login-field detection for the inline icon. Only finds fields; it never
// computes a URL match (Rust does that) and never reads a value.

import {
  findAllInputs,
  hasAutocompleteToken,
  inputType,
  isRendered,
  isUsernameType,
  rootsOf,
  scopeOf,
  usernamePartner,
} from "./fill";

export type Field = { input: HTMLInputElement; newPassword: boolean };

/**
 * Every rendered password input (`isRendered`: below the fold counts), its
 * username partner in the same scope (`scopeOf`), and any standalone
 * `autocomplete="username"`/`"email"` input (the first step of a two-step
 * login), in document order. A password is a new-password field when it says
 * so, or when another rendered password shares its scope (a sign-up with a
 * confirm field); hidden and zero-size inputs never count.
 */
export function scan(): { fields: Field[]; roots: Array<Document | ShadowRoot> } {
  const roots = rootsOf(document);
  const inputs = findAllInputs(roots);
  const rendered = inputs.filter(isRendered);

  const found = new Map<HTMLInputElement, boolean>();
  for (const password of rendered.filter((input) => inputType(input) === "password")) {
    const scope = scopeOf(password);
    const own = rendered.filter((input) => scopeOf(input) === scope);
    found.set(
      password,
      hasAutocompleteToken(password, "new-password") ||
        own.some((other) => other !== password && inputType(other) === "password"),
    );
    const partner = usernamePartner(own, password);
    if (partner && !found.has(partner)) found.set(partner, false);
  }
  for (const input of rendered) {
    if (
      !found.has(input) &&
      isUsernameType(input) &&
      (hasAutocompleteToken(input, "username") || hasAutocompleteToken(input, "email"))
    ) {
      found.set(input, false);
    }
  }

  const fields = inputs
    .filter((input) => found.has(input))
    .map((input) => ({ input, newPassword: found.get(input) === true }));
  return { fields, roots };
}

/** `getBoundingClientRect()` in top-viewport coordinates: a field inside a
 * same-origin iframe is offset by each enclosing frame's content box. */
export function topRect(element: Element): DOMRect {
  const rect = element.getBoundingClientRect();
  let left = rect.left;
  let top = rect.top;
  let frame = element.ownerDocument.defaultView?.frameElement ?? null;
  while (frame) {
    const box = frame.getBoundingClientRect();
    const style = frame.ownerDocument.defaultView?.getComputedStyle(frame);
    left += box.left + frame.clientLeft + (parseFloat(style?.paddingLeft ?? "") || 0);
    top += box.top + frame.clientTop + (parseFloat(style?.paddingTop ?? "") || 0);
    frame = frame.ownerDocument.defaultView?.frameElement ?? null;
  }
  return new DOMRect(left, top, rect.width, rect.height);
}
