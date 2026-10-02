import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { FAKE_ENTRIES } from "./support/fake-app";
import { openFixture, readLog, setMode, startHarness, type Harness } from "./support/harness";
import { clickInline, inlineOf, waitForOptions, type Inline } from "./support/inline";

// One attack fixture per clickjacking guard. Each refused pick must show the
// toolbar message tagged with the guard that fired, and must never reach the
// app for a credential.

const OPTION = '[role="option"]';
const REFUSAL = "Use the Subclave toolbar button to fill on this page.";

let harness: Harness;

test.beforeEach(async ({}, testInfo) => {
  harness = await startHarness(testInfo);
});

test.afterEach(async () => {
  if (harness) await harness.context.close();
});

/** Opens `fixture` and the picker on `#username`, ready for a pick. */
async function openPicker(name: string): Promise<{ page: Page; ui: Inline }> {
  const page = await openFixture(harness, name);
  const ui = inlineOf(page);
  await page.locator("#username").click();
  await waitForOptions(page, ui);
  return { page, ui };
}

async function expectRefused(page: Page, ui: Inline, guard: number): Promise<void> {
  await expect.poll(() => ui.pickerText()).toContain(REFUSAL);
  expect(await ui.attr(".message", "data-guard")).toBe(String(guard));
  await expect(page.locator("#username")).toHaveValue("");
  expect((await readLog(harness)).some((entry) => entry.action === "get-credential")).toBe(false);
}

test("guard 1: untrusted events neither open the picker nor pick", async () => {
  const page = await openFixture(harness, "guard-untrusted.html");
  const ui = inlineOf(page);
  await page.evaluate("window.__syntheticOpen()");
  await page.waitForTimeout(600);
  expect(await ui.popoverOpen()).toBe(false);

  await page.locator("#username").click();
  await waitForOptions(page, ui);
  await ui.dispatchUntrustedClick(OPTION, 0);
  await expectRefused(page, ui, 1);
});

test("guard 1: a click right after the options render is refused", async () => {
  const page = await openFixture(harness, "guard-timing.html");
  const ui = inlineOf(page);
  await page.locator("#username").click();
  // Click the moment the first row exists. A runner slow enough to land this
  // after the delay turns the test red; it can never pass falsely.
  await expect.poll(() => ui.center(OPTION), { intervals: [10] }).not.toBeNull();
  await clickInline(page, ui, OPTION, 0);
  await expectRefused(page, ui, 1);
});

test("guard 1: a click after the anchor moved restarts the delay", async () => {
  const { page, ui } = await openPicker("guard-timing.html");
  const centre = await ui.center(OPTION, 0);
  if (!centre) throw new Error("no option box");
  await page.evaluate("window.__shift()");
  await page.mouse.click(centre.x, centre.y);
  await expectRefused(page, ui, 1);
});

test("guard 2: another open popover refuses the pick", async () => {
  const { page, ui } = await openPicker("guard-toplayer.html");
  await page.evaluate("window.__promo()");
  await clickInline(page, ui, OPTION, 0);
  await expectRefused(page, ui, 2);
});

test("guard 2: a modal dialog refuses the picker on open", async () => {
  const page = await openFixture(harness, "guard-toplayer.html");
  const ui = inlineOf(page);
  await page.evaluate("window.__modal()");
  await expect.poll(() => ui.count(".icon")).toBe(4);
  await page.locator("#m-username").click();
  await expectRefused(page, ui, 2);
  expect(await ui.count(OPTION)).toBe(0);
});

for (const [label, hook] of [
  ["a faded <html>", 'window.__fade("html")'],
  ["a faded <body>", 'window.__fade("body")'],
]) {
  test(`guard 3: ${label} refuses the pick`, async () => {
    const { page, ui } = await openPicker("guard-style.html");
    await page.evaluate(hook);
    await clickInline(page, ui, OPTION, 0);
    await expectRefused(page, ui, 3);
  });
}

test("page CSS on the host never reaches the picker", async () => {
  const { page, ui } = await openPicker("guard-style.html");
  await page.evaluate("window.__styleHost()");
  // The `:host` rules are `!important`, and for important declarations the
  // shadow context beats the page: the host keeps its own values, so nothing
  // inherits into the picker and the text stays painted at full size.
  const host = await page.evaluate(() => {
    const element = document.querySelector("subclave-inline");
    if (!element) return null;
    const style = getComputedStyle(element);
    return { filter: style.filter, zoom: style.zoom };
  });
  expect(host).toEqual({ filter: "none", zoom: "1" });
  const color = await ui.style(".title", "color");
  expect(color).not.toBe("rgba(0, 0, 0, 0)");
  expect(await ui.style(".title", "-webkit-text-fill-color")).toBe(color);
  expect(await ui.style(".title", "text-indent")).toBe("0px");

  await clickInline(page, ui, OPTION, 0);
  await expect(page.locator("#username")).toHaveValue(FAKE_ENTRIES[0].username);
});

for (const [label, hook] of [
  ["an attribute on the host", "window.__tamperAttr()"],
  ["moving the host", "window.__tamperMove()"],
]) {
  test(`guard 4: ${label} closes the picker`, async () => {
    const { page, ui } = await openPicker("guard-tamper.html");
    const centre = await ui.center(OPTION, 0);
    if (!centre) throw new Error("no option box");
    await page.evaluate(hook);
    await expect.poll(() => ui.popoverOpen()).toBe(false);
    await page.mouse.click(centre.x, centre.y);
    await expect(page.locator("#username")).toHaveValue("");
    expect((await readLog(harness)).some((entry) => entry.action === "get-credential")).toBe(false);
  });
}

test("guard 5: an overlay that takes the click refuses the pick", async () => {
  const { page, ui } = await openPicker("guard-overlay.html");
  await page.evaluate("window.__overlay()");
  await clickInline(page, ui, OPTION, 0);
  await expectRefused(page, ui, 5);
});

test("guard 6: an opaque click-through overlay refuses the pick", async () => {
  const { page, ui } = await openPicker("guard-occluded.html");
  await page.evaluate("window.__overlay()");
  // The visibility observer reports after its 100 ms delay plus frame
  // scheduling on a shared runner.
  await page.waitForTimeout(600);
  await clickInline(page, ui, OPTION, 0);
  await expectRefused(page, ui, 6);
});

test("generate rows pass the same guards", async () => {
  await setMode(harness, "no-fields");
  const page = await openFixture(harness, "signup.html");
  const ui = inlineOf(page);
  await page.locator("#password").click();
  await waitForOptions(page, ui);
  expect(await ui.pickerText()).toMatch(/^Generate for this site/);
  await ui.dispatchUntrustedClick(OPTION, 0);
  await expectRefused(page, ui, 1);
  await expect(page.locator("#password")).toHaveValue("");
  const log = await readLog(harness);
  expect(
    log.some((entry) => entry.action === "generate-password" || entry.action === "save-login"),
  ).toBe(false);
});
