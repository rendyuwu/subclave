import { expect } from "@playwright/test";
import type { CDPSession, Page } from "@playwright/test";
import { GUARD_DELAY_MS } from "../../src/content/guard";

// Playwright locators cannot pierce a closed shadow root, and the inline UI
// lives in one on purpose. CDP's DOM domain can, so every read here goes
// through `DOM.getDocument({ pierce: true })`. Node ids go stale on each
// document fetch, so every call starts from a fresh one.

export type Inline = {
  count(selector: string): Promise<number>;
  /** The centre of the `i`th match's border box, in top-viewport CSS px. */
  center(selector: string, index?: number): Promise<{ x: number; y: number } | null>;
  pickerText(): Promise<string>;
  attr(selector: string, name: string): Promise<string | null>;
  /** The first match's computed value of a CSS property. */
  style(selector: string, property: string): Promise<string | null>;
  popoverOpen(): Promise<boolean>;
  /** An untrusted click at the node's centre, dispatched from page script. */
  dispatchUntrustedClick(selector: string, index?: number): Promise<void>;
};

export function inlineOf(page: Page): Inline {
  let session: Promise<CDPSession> | null = null;
  const cdp = (): Promise<CDPSession> => (session ??= page.context().newCDPSession(page));

  /** The extension's shadow root, or null when the host is not in the page. */
  const root = async (): Promise<number | null> => {
    const { root: doc } = await (await cdp()).send("DOM.getDocument", { depth: -1, pierce: true });
    const html = doc.children?.find((child) => child.nodeName === "HTML");
    const host = html?.children?.find((child) => child.nodeName === "SUBCLAVE-INLINE");
    if (!host) return null;
    const shadow = host.shadowRoots?.[0];
    if (shadow?.shadowRootType !== "closed") {
      throw new Error(
        `subclave-inline shadow root is ${shadow?.shadowRootType ?? "missing"}, not closed`,
      );
    }
    return shadow.nodeId;
  };

  const nodes = async (selector: string): Promise<number[]> => {
    const nodeId = await root();
    if (nodeId === null) return [];
    const { nodeIds } = await (await cdp()).send("DOM.querySelectorAll", { nodeId, selector });
    return nodeIds;
  };

  const centerOf = async (nodeId: number): Promise<{ x: number; y: number } | null> => {
    try {
      const { model } = await (await cdp()).send("DOM.getBoxModel", { nodeId });
      const [x1, y1, , , x3, y3] = model.border;
      return { x: (x1 + x3) / 2, y: (y1 + y3) / 2 };
    } catch {
      // No layout box (the picker is closed, or the node is display: none).
      return null;
    }
  };

  /** Runs `fn` with the node as `this` and returns its JSON value. */
  const call = async <T>(nodeId: number, fn: string, args: unknown[] = []): Promise<T> => {
    const client = await cdp();
    const { object } = await client.send("DOM.resolveNode", { nodeId });
    const { result } = await client.send("Runtime.callFunctionOn", {
      objectId: object.objectId,
      functionDeclaration: fn,
      arguments: args.map((value) => ({ value })),
      returnByValue: true,
    });
    return result.value as T;
  };

  return {
    async count(selector) {
      return (await nodes(selector)).length;
    },
    async center(selector, index = 0) {
      const nodeId = (await nodes(selector))[index];
      return nodeId === undefined ? null : centerOf(nodeId);
    },
    async pickerText() {
      const [picker] = await nodes(".picker");
      return picker === undefined
        ? ""
        : call<string>(picker, "function () { return this.textContent ?? ''; }");
    },
    async attr(selector, name) {
      const [nodeId] = await nodes(selector);
      return nodeId === undefined
        ? null
        : call<string | null>(nodeId, "function (name) { return this.getAttribute(name); }", [
            name,
          ]);
    },
    async style(selector, property) {
      const [nodeId] = await nodes(selector);
      return nodeId === undefined
        ? null
        : call<string>(
            nodeId,
            "function (name) { return getComputedStyle(this).getPropertyValue(name); }",
            [property],
          );
    },
    async popoverOpen() {
      const [picker] = await nodes(".picker");
      return picker === undefined
        ? false
        : call<boolean>(picker, "function () { return this.matches(':popover-open'); }");
    },
    async dispatchUntrustedClick(selector, index = 0) {
      const nodeId = (await nodes(selector))[index];
      if (nodeId === undefined) throw new Error(`no ${selector} at ${index}`);
      const point = await centerOf(nodeId);
      if (!point) throw new Error(`${selector} at ${index} has no box`);
      await call<null>(
        nodeId,
        `function (x, y) {
          this.dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true, clientX: x, clientY: y }));
          return null;
        }`,
        [point.x, point.y],
      );
    },
  };
}

/** Waits for pickable rows, then past the guard's fresh-render delay. */
export async function waitForOptions(page: Page, ui: Inline): Promise<void> {
  await expect.poll(() => ui.count('[role="option"]')).toBeGreaterThan(0);
  await page.waitForTimeout(GUARD_DELAY_MS + 100);
}

/** A trusted click at the centre of the `index`th match. */
export async function clickInline(
  page: Page,
  ui: Inline,
  selector: string,
  index = 0,
): Promise<void> {
  const point = await ui.center(selector, index);
  if (!point) throw new Error(`no ${selector} box at ${index}`);
  await page.mouse.click(point.x, point.y);
}
