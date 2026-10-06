import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";
import {
  CREDENTIAL_DELAY_KEY,
  FAKE_DOMAIN,
  FAKE_ENTRIES,
  FAKE_GENERATED,
  FAKE_PASSWORD,
} from "./support/fake-app";
import {
  FIXTURE_ORIGIN,
  openFixture,
  openPopup,
  readConnects,
  readLog,
  setMode,
  startHarness,
  type Harness,
} from "./support/harness";
import { clickInline, inlineOf, waitForOptions, type Inline } from "./support/inline";

// The inline icon and picker, driven through real (trusted) mouse and keyboard
// input. The picker lives in a closed shadow root, so every read of it goes
// through `support/inline.ts`.

const OPTION = '[role="option"]';

let harness: Harness;

test.afterEach(async () => {
  if (harness) await harness.context.close();
});

/**
 * The index of the icon drawn in `field`: its centre inside the field's box and
 * within 30 px of the right edge. -1 when there is none.
 */
async function iconIndex(ui: Inline, field: Locator): Promise<number> {
  const box = await field.boundingBox();
  if (!box) return -1;
  const right = box.x + box.width;
  const count = await ui.count(".icon");
  for (let index = 0; index < count; index += 1) {
    const centre = await ui.center(".icon", index);
    if (
      centre &&
      centre.x >= box.x &&
      centre.x <= right &&
      centre.y >= box.y &&
      centre.y <= box.y + box.height &&
      right - centre.x <= 30
    ) {
      return index;
    }
  }
  return -1;
}

/** Opens the picker on `field` with a click and waits until a pick can pass. */
async function openOn(page: Page, ui: Inline, field: Locator): Promise<void> {
  await field.click();
  await waitForOptions(page, ui);
}

const GUARD_FIXTURES = [
  "guard-untrusted.html",
  "guard-timing.html",
  "guard-toplayer.html",
  "guard-style.html",
  "guard-tamper.html",
  "guard-overlay.html",
  "guard-occluded.html",
];

// [fixture, top-document field selectors, `#frame` field selectors]
const ICON_CASES: Array<[string, string[], string[]]> = [
  ["login.html", ["#username", "#password"], []],
  ["signup.html", ["#username", "#password", "#password-confirm"], []],
  ["hidden-fields.html", ["#username", "#password"], []],
  ["two-step.html", ["#username"], []],
  ["nested.html", ["#s-username", "#s-password"], ["#f-username", "#f-password"]],
  [
    "new-password.html",
    ["#a-username", "#a-password", "#b-username", "#b-password", "#b-confirm"],
    [],
  ],
  ["autofocus.html", ["#username", "#password"], []],
  // Below the fold: detection does not require the viewport.
  ["below-fold.html", ["#username", "#password"], []],
  // The formless search box is no partner of the password-only form.
  ["search-box.html", ["#password"], []],
  ["formless.html", ["#username", "#password"], ["#f-username", "#f-password"]],
  ["vanish.html", ["#username", "#password"], []],
  ["navigate.html", ["#username", "#password"], []],
  ["churn.html", ["#username"], []],
  // Rendered 2 s after `load`, as single-page apps do.
  ["late-form.html", ["#username", "#password"], []],
  ...GUARD_FIXTURES.map((name): [string, string[], string[]] => [
    name,
    ["#username", "#password"],
    [],
  ]),
];

test("draws an icon in every visible login field without connecting", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  for (const [name, selectors, frameSelectors] of ICON_CASES) {
    const page = await openFixture(harness, name);
    const ui = inlineOf(page);
    const fields = [
      ...selectors.map((selector) => page.locator(selector)),
      ...frameSelectors.map((selector) => page.frameLocator("#frame").locator(selector)),
    ];
    await expect.poll(() => ui.count(".icon"), { message: name }).toBe(fields.length);
    for (const field of fields) {
      expect(await iconIndex(ui, field), name).toBeGreaterThanOrEqual(0);
    }
    await page.close();
  }

  const page = await openFixture(harness, "no-fields.html");
  await page.waitForTimeout(1000);
  expect(await inlineOf(page).count(".icon")).toBe(0);

  expect(await readConnects(harness)).toBe(0);
});

test("autofocus and script focus show icons but never the picker", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "autofocus.html");
  const ui = inlineOf(page);
  await expect.poll(() => page.evaluate(() => document.activeElement?.id)).toBe("username");
  await expect.poll(() => ui.count(".icon")).toBe(2);
  expect(await ui.popoverOpen()).toBe(false);

  await page.evaluate(() => document.getElementById("password")?.focus());
  await page.waitForTimeout(600);
  expect(await ui.popoverOpen()).toBe(false);
});

test("with Show in login fields off there are no icons and the popup still fills", async ({}, testInfo) => {
  harness = await startHarness(testInfo, { showInLoginFields: false });
  const page = await openFixture(harness, "login.html");
  await page.waitForTimeout(1000);
  expect(await inlineOf(page).count(".icon")).toBe(0);

  const popup = await openPopup(harness.context, harness.extensionId, page);
  await popup.getByRole("option").first().click();
  await expect(page.locator("#username")).toHaveValue(FAKE_ENTRIES[0].username);
  await expect(page.locator("#password")).toHaveValue(FAKE_PASSWORD);
});

test("an inline fill never touches hidden, zero-size or decoy inputs", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "hidden-fields.html");
  const ui = inlineOf(page);
  await openOn(page, ui, page.locator("#username"));
  await clickInline(page, ui, OPTION, 0);

  await expect(page.locator("#password")).toHaveValue(FAKE_PASSWORD);
  await expect(page.locator("#hidden-password")).toHaveValue("");
  await expect(page.locator("#zero-password")).toHaveValue("");
  await expect(page.locator("#decoy-password")).toHaveValue("");
  const events = await page.evaluate(() => window.__events);
  expect(events["hidden:input"]).toBe(0);
  expect(events["zero:change"]).toBe(0);
});

test("opens on a field click, on the icon and on ArrowDown, for this host only", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "login.html");
  const ui = inlineOf(page);
  const username = page.locator("#username");

  await username.click();
  await expect.poll(() => ui.popoverOpen()).toBe(true);
  await expect.poll(() => ui.count(OPTION)).toBe(FAKE_ENTRIES.length);
  const text = await ui.pickerText();
  for (const entry of FAKE_ENTRIES) {
    expect(text).toContain(entry.title);
    expect(text).toContain(entry.username);
    expect(text).toContain(entry.group);
  }
  const logins = (await readLog(harness)).filter((entry) => entry.action === "get-logins").at(-1);
  expect(logins?.params?.scope).toBe("host");
  expect(logins?.params?.url).toBe(`${FIXTURE_ORIGIN}/login.html`);

  await page.keyboard.press("Escape");
  await expect.poll(() => ui.popoverOpen()).toBe(false);
  const icon = await iconIndex(ui, username);
  expect(icon).toBeGreaterThanOrEqual(0);
  await clickInline(page, ui, ".icon", icon);
  await expect.poll(() => ui.popoverOpen()).toBe(true);

  await page.keyboard.press("Escape");
  await expect.poll(() => ui.popoverOpen()).toBe(false);
  await username.focus();
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => ui.popoverOpen()).toBe(true);
  await page.keyboard.press("Escape");
  await expect.poll(() => ui.popoverOpen()).toBe(false);
  expect(await page.evaluate(() => document.activeElement?.id)).toBe("username");
});

test("the footer counts the logins left for the toolbar button", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  await setMode(harness, "no-fields");
  const page = await openFixture(harness, "login.html");
  const ui = inlineOf(page);
  await page.locator("#username").click();
  await expect.poll(() => ui.pickerText()).toContain("No logins for 127.0.0.1.");
  expect(await ui.pickerText()).toContain(
    `1 more login on ${FAKE_DOMAIN}: use the Subclave toolbar button`,
  );

  await setMode(harness, "ok");
  const second = await openFixture(harness, "login.html");
  const secondUi = inlineOf(second);
  await second.locator("#username").click();
  await expect
    .poll(() => secondUi.pickerText())
    .toContain(`2 more logins on ${FAKE_DOMAIN}: use the Subclave toolbar button`);
});

test("a pick fills the field's login through via inline, by pointer and by keyboard", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "login.html");
  const ui = inlineOf(page);
  await openOn(page, ui, page.locator("#username"));
  await clickInline(page, ui, OPTION, 0);

  await expect(page.locator("#username")).toHaveValue(FAKE_ENTRIES[0].username);
  await expect(page.locator("#password")).toHaveValue(FAKE_PASSWORD);
  const events = await page.evaluate(() => window.__events);
  expect(events["username:input"]).toBe(1);
  expect(events["username:change"]).toBe(1);
  expect(events["password:input"]).toBe(1);
  expect(events["password:change"]).toBe(1);
  const credential = (await readLog(harness))
    .filter((entry) => entry.action === "get-credential")
    .at(-1);
  expect(credential?.params).toEqual({
    id: FAKE_ENTRIES[0].id,
    url: `${FIXTURE_ORIGIN}/login.html`,
    via: "inline",
  });
  await expect.poll(() => ui.popoverOpen()).toBe(false);

  const keyboard = await openFixture(harness, "login.html");
  const keyboardUi = inlineOf(keyboard);
  await keyboard.locator("#username").focus();
  await keyboard.keyboard.press("ArrowDown");
  await waitForOptions(keyboard, keyboardUi);
  await keyboard.keyboard.press("ArrowDown");
  await keyboard.keyboard.press("Enter");
  await expect(keyboard.locator("#username")).toHaveValue(FAKE_ENTRIES[1].username);
  await expect(keyboard.locator("#password")).toHaveValue(FAKE_PASSWORD);
});

test("fills the picked field's form inside a shadow root and a same-origin iframe", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "nested.html");
  const ui = inlineOf(page);
  const frame = page.frameLocator("#frame");

  await openOn(page, ui, frame.locator("#f-username"));
  await clickInline(page, ui, OPTION, 0);
  await expect(frame.locator("#f-username")).toHaveValue(FAKE_ENTRIES[0].username);
  await expect(frame.locator("#f-password")).toHaveValue(FAKE_PASSWORD);
  await expect(page.locator("#s-username")).toHaveValue("");
  await expect(page.locator("#s-password")).toHaveValue("");

  await openOn(page, ui, page.locator("#s-username"));
  await clickInline(page, ui, OPTION, 0);
  await expect(page.locator("#s-username")).toHaveValue(FAKE_ENTRIES[0].username);
  await expect(page.locator("#s-password")).toHaveValue(FAKE_PASSWORD);
});

test("a two-step login fills the username, then the password", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "two-step.html");
  const ui = inlineOf(page);

  await openOn(page, ui, page.locator("#username"));
  await clickInline(page, ui, OPTION, 0);
  await expect(page.locator("#username")).toHaveValue(FAKE_ENTRIES[0].username);
  await expect(page.locator("#password")).toHaveValue("");

  await page.locator("#next").click();
  const password = page.locator("#password");
  await expect.poll(() => iconIndex(ui, password)).toBeGreaterThanOrEqual(0);
  await openOn(page, ui, password);
  await clickInline(page, ui, OPTION, 0);
  await expect(password).toHaveValue(FAKE_PASSWORD);
});

test("new-password fields offer generate and update rows", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  await setMode(harness, "no-fields");
  const signup = await openFixture(harness, "signup.html");
  const ui = inlineOf(signup);
  await signup.locator("#username").fill("newuser");
  await openOn(signup, ui, signup.locator("#password"));
  expect(await ui.pickerText()).toMatch(/^Generate for this site/);
  await clickInline(signup, ui, OPTION, 0);
  await expect(signup.locator("#password")).toHaveValue(FAKE_GENERATED);
  await expect(signup.locator("#password-confirm")).toHaveValue(FAKE_GENERATED);
  await expect
    .poll(async () => (await readLog(harness)).some((entry) => entry.action === "save-login"))
    .toBe(true);
  const created = (await readLog(harness)).find((entry) => entry.action === "save-login");
  expect(created?.params?.username).toBe("newuser");
  expect(created?.params?.via).toBe("inline");
  expect(created?.params?.url).toBe(`${FIXTURE_ORIGIN}/signup.html`);
  expect(created?.params?.entryId).toBeUndefined();

  await setMode(harness, "ok");
  const update = await openFixture(harness, "signup.html");
  const updateUi = inlineOf(update);
  await openOn(update, updateUi, update.locator("#password"));
  const text = await updateUi.pickerText();
  const order = [
    `Update ${FAKE_ENTRIES[0].title}`,
    `Update ${FAKE_ENTRIES[1].title}`,
    "New entry",
  ].map((label) => text.indexOf(label));
  expect(order[0]).toBe(0);
  expect(order[1]).toBeGreaterThan(order[0]);
  expect(order[2]).toBeGreaterThan(order[1]);
  await clickInline(update, updateUi, OPTION, 1);
  await expect
    .poll(
      async () =>
        (await readLog(harness)).filter((entry) => entry.action === "save-login").at(-1)?.params
          ?.entryId,
    )
    .toBe(FAKE_ENTRIES[1].id);

  await update.locator("#username").click();
  await expect.poll(() => updateUi.pickerText()).toMatch(new RegExp(`^${FAKE_ENTRIES[0].title}`));

  await setMode(harness, "no-fields");
  const forms = await openFixture(harness, "new-password.html");
  const formsUi = inlineOf(forms);
  await forms.locator("#a-password").click();
  await expect.poll(() => formsUi.pickerText()).toMatch(/^Generate for this site/);
  await forms.keyboard.press("Escape");
  await expect.poll(() => formsUi.popoverOpen()).toBe(false);
  await forms.locator("#b-password").click();
  await expect.poll(() => formsUi.pickerText()).toMatch(/^Generate for this site/);
});

test("locked: shows the state and Unlock focuses the app", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  await setMode(harness, "locked");
  const page = await openFixture(harness, "login.html");
  const ui = inlineOf(page);
  await page.locator("#username").click();
  await expect.poll(() => ui.pickerText()).toContain("Subclave is locked");
  await clickInline(page, ui, OPTION, 0);
  await expect
    .poll(async () => (await readLog(harness)).some((entry) => entry.action === "focus-app"))
    .toBe(true);
});

test("app absent: shows the not-running state", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  await setMode(harness, "absent");
  const page = await openFixture(harness, "login.html");
  const ui = inlineOf(page);
  await page.locator("#username").click();
  await expect
    .poll(() => ui.pickerText())
    .toContain("Subclave is not running. Start it from your applications menu.");
});

test("unpaired: Pair with Subclave starts pairing", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  await setMode(harness, "unpaired");
  const page = await openFixture(harness, "login.html");
  const ui = inlineOf(page);
  await page.locator("#username").click();
  await expect.poll(() => ui.pickerText()).toContain("Pair with Subclave");
  await clickInline(page, ui, OPTION, 0);
  await expect
    .poll(async () => (await readLog(harness)).some((entry) => entry.action === "associate"))
    .toBe(true);
});

test("a login form below the fold gets its icons and fills once scrolled to", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "below-fold.html");
  const ui = inlineOf(page);
  await expect.poll(() => ui.count(".icon")).toBe(2);
  // `click` scrolls the field into view first.
  await openOn(page, ui, page.locator("#username"));
  await clickInline(page, ui, OPTION, 0);
  await expect(page.locator("#username")).toHaveValue(FAKE_ENTRIES[0].username);
  await expect(page.locator("#password")).toHaveValue(FAKE_PASSWORD);
});

test("a password-only form never fills the search box in front of it", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "search-box.html");
  const ui = inlineOf(page);
  await openOn(page, ui, page.locator("#password"));
  await clickInline(page, ui, OPTION, 0);
  await expect(page.locator("#password")).toHaveValue(FAKE_PASSWORD);
  await expect(page.locator("#search")).toHaveValue("");
});

test("formless logins in two documents are not one sign-up form", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "formless.html");
  const ui = inlineOf(page);
  await openOn(page, ui, page.locator("#password"));
  expect(await ui.pickerText()).toMatch(new RegExp(`^${FAKE_ENTRIES[0].title}`));
});

test("a field that vanishes at the pick fills nothing and says so", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "vanish.html");
  const ui = inlineOf(page);
  await openOn(page, ui, page.locator("#username"));
  await page.evaluate("window.__vanishOnClick()");
  await clickInline(page, ui, OPTION, 0);
  await expect.poll(() => ui.pickerText()).toContain("No login fields found on this page.");
  await expect(page.locator("#username")).toHaveValue("");
  await expect(page.locator("#password")).toHaveValue("");
});

test("a navigation between the pick and the fill leaves the new page untouched", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  await harness.worker.evaluate(({ key, value }) => chrome.storage.local.set({ [key]: value }), {
    key: CREDENTIAL_DELAY_KEY,
    value: 1000,
  });
  const page = await openFixture(harness, "navigate.html");
  const ui = inlineOf(page);
  await openOn(page, ui, page.locator("#username"));
  await page.evaluate("window.__navigateOnClick()");
  await clickInline(page, ui, OPTION, 0);
  await page.waitForURL("**/login.html?navigated");
  // The credential is released after the navigation; the fill that follows
  // reaches the new document within milliseconds.
  await expect
    .poll(async () =>
      (await readLog(harness)).some((entry) => entry.action === "get-credential" && entry.ok),
    )
    .toBe(true);
  await page.waitForTimeout(500);
  await expect(page.locator("#username")).toHaveValue("");
  await expect(page.locator("#password")).toHaveValue("");
});

test("detection keeps up with a page that never stops changing", async ({}, testInfo) => {
  harness = await startHarness(testInfo);
  const page = await openFixture(harness, "churn.html");
  const ui = inlineOf(page);
  await expect.poll(() => ui.count(".icon")).toBe(1);
  await page.locator("#next").click();
  // The ticker mutates every 50 ms, inside the 250 ms debounce for good; the
  // max wait still runs a pass.
  await expect.poll(() => ui.count(".icon")).toBe(2);
});
