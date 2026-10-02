import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { FAKE_ENTRIES, FAKE_PASSWORD } from "./support/fake-app";
import {
  FIXTURE_ORIGIN,
  FIXTURE_PORT,
  openFixture,
  openPopup,
  readLog,
  setMode,
  startHarness,
  type Harness,
} from "./support/harness";
import { clickInline, inlineOf, waitForOptions, type Inline } from "./support/inline";

// The save prompt for a login typed and submitted on a page: the submit is
// captured in the content script, the pair waits in the service worker's
// `chrome.storage.session`, and the next page load (or the same page, when it
// stays) offers Add or Update.

const OPTION = '[role="option"]';
const LANDING = `${FIXTURE_ORIGIN}/landing.html`;
const SAVE_LOGIN = `${FIXTURE_ORIGIN}/save-login.html`;
/** Another site to the fake app, which compares origins. */
const OTHER_SITE = `http://localhost:${FIXTURE_PORT}/landing.html`;
/** `PENDING_SAVE_PREFIX` in `background.ts`. */
const PENDING_PREFIX = "subclave.pendingSave.";

let harness: Harness;

test.beforeEach(async ({}, testInfo) => {
  harness = await startHarness(testInfo);
});

test.afterEach(async () => {
  if (harness) await harness.context.close();
});

/** The form is wired in the same pass that draws the icons, so a fill before
 * that can be missed. */
async function ready(ui: Inline, icons: number): Promise<void> {
  await expect.poll(() => ui.count(".icon")).toBe(icons);
}

async function signIn(page: Page, ui: Inline, username: string, password: string): Promise<void> {
  await ready(ui, 2);
  await page.locator("#username").fill(username);
  await page.locator("#password").fill(password);
  await page.locator("#submit").click();
}

async function changePassword(
  page: Page,
  ui: Inline,
  username: string,
  current: string,
  next: string,
): Promise<void> {
  await ready(ui, 4);
  await page.locator("#username").fill(username);
  await page.locator("#current").fill(current);
  await page.locator("#new").fill(next);
  await page.locator("#repeat").fill(next);
  await page.locator("#submit").click();
}

async function calls(action: string): Promise<Array<Record<string, unknown> | null>> {
  return (await readLog(harness))
    .filter((entry) => entry.action === action)
    .map((entry) => entry.params);
}

async function lastCall(action: string): Promise<Record<string, unknown> | null | undefined> {
  return (await calls(action)).at(-1);
}

async function pendingKeys(): Promise<string[]> {
  const stored = await harness.worker.evaluate(() => chrome.storage.session.get(null));
  return Object.keys(stored).filter((key) => key.startsWith(PENDING_PREFIX));
}

test("a sign-in with no stored login offers Add on the next page, and Add saves it", async () => {
  await setMode(harness, "no-fields");
  const page = await openFixture(harness, "save-login.html");
  const ui = inlineOf(page);
  await signIn(page, ui, "newuser", "typed-pw");
  await page.waitForURL("**/landing.html");
  await expect.poll(() => ui.pickerText()).toContain("Save login for 127.0.0.1?");
  const text = await ui.pickerText();
  for (const part of ["Add", "newuser", "Cancel"]) expect(text).toContain(part);
  expect(text).not.toContain("Update");
  await waitForOptions(page, ui);
  await clickInline(page, ui, OPTION, 0);
  await expect
    .poll(() => lastCall("save-login"))
    .toEqual({ url: SAVE_LOGIN, username: "newuser", password: "typed-pw", via: "inline" });
  await expect.poll(() => ui.popoverOpen()).toBe(false);
  const checked = await lastCall("check-login");
  expect(checked?.url).toBe(SAVE_LOGIN);
  expect(checked?.pageUrl).toBe(LANDING);
});

test("a new password for a stored username offers Update for that entry", async () => {
  const page = await openFixture(harness, "save-login.html");
  const ui = inlineOf(page);
  await signIn(page, ui, FAKE_ENTRIES[1].username, "new-pw");
  await page.waitForURL("**/landing.html");
  await expect
    .poll(() => ui.pickerText())
    .toMatch(new RegExp(`^Save login for 127\\.0\\.0\\.1\\?Update ${FAKE_ENTRIES[1].title}`));
  await waitForOptions(page, ui);
  await clickInline(page, ui, OPTION, 0);
  await expect.poll(() => lastCall("save-login")).toBeTruthy();
  const params = await lastCall("save-login");
  expect(params?.entryId).toBe(FAKE_ENTRIES[1].id);
  expect(params?.password).toBe("new-pw");
  expect(params?.via).toBe("inline");
});

test("an unknown username picks the entry to update from a list", async () => {
  const page = await openFixture(harness, "save-login.html");
  const ui = inlineOf(page);
  await signIn(page, ui, "carol", "pw");
  await page.waitForURL("**/landing.html");
  await expect
    .poll(() => ui.pickerText())
    .toBe("Save login for 127.0.0.1?AddcarolUpdate an existing loginCancel");
  await waitForOptions(page, ui);
  await clickInline(page, ui, OPTION, 1);
  await expect.poll(() => ui.pickerText()).toContain(`Update ${FAKE_ENTRIES[0].title}`);
  await waitForOptions(page, ui);
  expect(await ui.pickerText()).toContain(`Update ${FAKE_ENTRIES[1].title}`);
  await clickInline(page, ui, OPTION, 1);
  await expect.poll(() => lastCall("save-login")).toBeTruthy();
  const params = await lastCall("save-login");
  expect(params?.entryId).toBe(FAKE_ENTRIES[1].id);
  expect(params?.username).toBe("carol");
});

test("a stored login gets no prompt and is not checked again", async () => {
  const page = await openFixture(harness, "save-login.html");
  const ui = inlineOf(page);
  await signIn(page, ui, FAKE_ENTRIES[0].username, FAKE_PASSWORD);
  await expect
    .poll(async () =>
      (await readLog(harness)).some(
        (entry) => entry.action === "check-login" && entry.result?.state === "unchanged",
      ),
    )
    .toBe(true);
  await page.waitForTimeout(600);
  expect(await ui.popoverOpen()).toBe(false);
  const checks = (await calls("check-login")).length;
  await page.goto(LANDING);
  await page.waitForTimeout(1000);
  expect((await calls("check-login")).length).toBe(checks);
});

test("a locked vault keeps the sign-in for later and Cancel drops it", async () => {
  await setMode(harness, "locked");
  const page = await openFixture(harness, "save-login.html");
  const ui = inlineOf(page);
  await signIn(page, ui, "bob", "new-pw");
  await page.waitForURL("**/landing.html");
  await page.waitForTimeout(1000);
  expect(await ui.popoverOpen()).toBe(false);
  await setMode(harness, "ok");
  await page.goto(LANDING);
  await expect.poll(() => ui.pickerText()).toContain(`Update ${FAKE_ENTRIES[1].title}`);
  await waitForOptions(page, ui);
  await clickInline(page, ui, OPTION, (await ui.count(OPTION)) - 1);
  await expect.poll(() => ui.popoverOpen()).toBe(false);
  await page.goto(LANDING);
  await page.waitForTimeout(1000);
  expect(await ui.popoverOpen()).toBe(false);
  expect(await calls("save-login")).toEqual([]);
});

test("a pending sign-in shows on at most three page loads", async () => {
  const page = await openFixture(harness, "change-password.html");
  const ui = inlineOf(page);
  await changePassword(page, ui, "alice", FAKE_PASSWORD, "next-pw");
  await expect.poll(() => ui.popoverOpen()).toBe(true);
  for (let load = 2; load <= 3; load += 1) {
    await page.goto(LANDING);
    await expect.poll(() => ui.popoverOpen()).toBe(true);
  }
  await page.goto(LANDING);
  await page.waitForTimeout(1000);
  expect(await ui.popoverOpen()).toBe(false);
});

test("a change-password form that stays on the page prompts in place with the new password", async () => {
  const page = await openFixture(harness, "change-password.html");
  const ui = inlineOf(page);
  await changePassword(page, ui, "alice", FAKE_PASSWORD, "next-pw");
  await expect
    .poll(() => ui.pickerText())
    .toMatch(new RegExp(`^Save login for 127\\.0\\.0\\.1\\?Update ${FAKE_ENTRIES[0].title}`));
  expect(page.url()).toBe(`${FIXTURE_ORIGIN}/change-password.html`);
  await waitForOptions(page, ui);
  await clickInline(page, ui, OPTION, 0);
  await expect.poll(() => lastCall("save-login")).toBeTruthy();
  const params = await lastCall("save-login");
  expect(params?.password).toBe("next-pw");
  expect(params?.entryId).toBe(FAKE_ENTRIES[0].id);
});

test("an untrusted submit is ignored", async () => {
  const page = await openFixture(harness, "save-login.html");
  const ui = inlineOf(page);
  await ready(ui, 2);
  await page.locator("#username").fill("bob");
  await page.locator("#password").fill("new-pw");
  await page.evaluate("window.__untrustedSubmit()");
  await page.goto(LANDING);
  await page.waitForTimeout(1500);
  expect(await ui.popoverOpen()).toBe(false);
  expect(await calls("check-login")).toEqual([]);
});

test("the sign-in survives a service worker restart", async () => {
  const page = await openFixture(harness, "change-password.html");
  const ui = inlineOf(page);
  // The marker lives only in this worker instance's global scope.
  await harness.worker.evaluate(() => {
    const scope: typeof globalThis & { subclaveMarker?: boolean } = globalThis;
    scope.subclaveMarker = true;
  });
  await changePassword(page, ui, "alice", FAKE_PASSWORD, "next-pw");
  await expect.poll(() => ui.popoverOpen()).toBe(true);

  // Playwright keeps one `Worker` handle across the restart (no second
  // `serviceworker` event), so the marker is read back through it.
  const popup = await openPopup(harness.context, harness.extensionId);
  const cdp = await harness.context.newCDPSession(popup);
  await cdp.send("ServiceWorker.enable");
  await cdp.send("ServiceWorker.stopAllWorkers");
  await popup.close();

  await page.goto(LANDING);
  await expect.poll(() => ui.pickerText()).toContain(`Update ${FAKE_ENTRIES[0].title}`);
  expect(
    await harness.worker.evaluate(() => {
      const scope: typeof globalThis & { subclaveMarker?: boolean } = globalThis;
      return scope.subclaveMarker;
    }),
  ).toBeUndefined();
});

test("a submit run by page script without a user gesture is ignored", async () => {
  // CDP reads only until the planted submit has run: Playwright's `evaluate`,
  // `fill` and `click` would all grant the page a user activation.
  const page = await openFixture(harness, "planted-submit.html");
  const ui = inlineOf(page);
  await ready(ui, 2);
  await page.waitForTimeout(3000);
  // The pair would be stored about a second before its `check-login`, so this
  // read is the one with margin.
  expect(await pendingKeys()).toEqual([]);
  expect(await ui.popoverOpen()).toBe(false);
  expect(await calls("check-login")).toEqual([]);
  expect(await page.title()).toBe("submitted");
});

test("a click that leaves the password in the form shows no prompt in place", async () => {
  const page = await openFixture(harness, "change-password.html");
  const ui = inlineOf(page);
  await ready(ui, 4);
  await page.locator("#username").fill("alice");
  await page.locator("#current").fill(FAKE_PASSWORD);
  await page.locator("#new").fill("next-pw");
  await page.locator("#repeat").fill("next-pw");
  await page.locator("#toggle").click();
  await page.waitForTimeout(1500);
  expect(await ui.popoverOpen()).toBe(false);
});

test("a prompt saves only the sign-in it shows", async () => {
  const page = await openFixture(harness, "save-login.html");
  const ui = inlineOf(page);
  await signIn(page, ui, FAKE_ENTRIES[1].username, "new-pw");
  await page.waitForURL("**/landing.html");
  await expect.poll(() => ui.pickerText()).toContain(`Update ${FAKE_ENTRIES[1].title}`);
  // A newer sign-in in the same tab, as a later submit would store it.
  const [key] = await pendingKeys();
  await harness.worker.evaluate(async (pendingKey) => {
    const stored = await chrome.storage.session.get(pendingKey);
    await chrome.storage.session.set({
      [pendingKey]: { ...(stored[pendingKey] as object), id: "newer" },
    });
  }, key);
  await waitForOptions(page, ui);
  await clickInline(page, ui, OPTION, 0);
  await expect
    .poll(() => ui.pickerText())
    .toContain("This sign-in is no longer waiting to be saved.");
  expect(await calls("save-login")).toEqual([]);
});

test("a new submit closes a shown prompt", async () => {
  const page = await openFixture(harness, "change-password.html");
  const ui = inlineOf(page);
  await changePassword(page, ui, "alice", FAKE_PASSWORD, "next-pw");
  await expect.poll(() => ui.pickerText()).toContain(`Update ${FAKE_ENTRIES[0].title}`);
  await changePassword(page, ui, FAKE_ENTRIES[1].username, FAKE_PASSWORD, "other-pw");
  await expect.poll(() => ui.popoverOpen()).toBe(false);
  await expect
    .poll(() => ui.pickerText())
    .toMatch(new RegExp(`^Save login for 127\\.0\\.0\\.1\\?Update ${FAKE_ENTRIES[1].title}`));
  expect(await ui.popoverOpen()).toBe(true);
});

test("a sign-in waits while the tab is on another site", async () => {
  const page = await openFixture(harness, "save-login.html");
  const ui = inlineOf(page);
  await signIn(page, ui, FAKE_ENTRIES[1].username, "new-pw");
  await page.waitForURL("**/landing.html");
  await expect.poll(() => ui.popoverOpen()).toBe(true);
  await page.goto(OTHER_SITE);
  await expect.poll(async () => (await lastCall("check-login"))?.pageUrl).toBe(OTHER_SITE);
  await page.waitForTimeout(500);
  expect(await ui.popoverOpen()).toBe(false);
  await page.goto(LANDING);
  await expect.poll(() => ui.pickerText()).toContain(`Update ${FAKE_ENTRIES[1].title}`);
});

test("a card code is not saved as a login", async () => {
  const page = await openFixture(harness, "checkout.html");
  const ui = inlineOf(page);
  await ready(ui, 2);
  await page.locator("#email").fill("alice@example.com");
  await page.locator("#cvv").fill("123");
  await page.locator("#submit").click();
  await page.waitForURL("**/landing.html");
  await page.waitForTimeout(1500);
  expect(await ui.popoverOpen()).toBe(false);
  expect(await calls("check-login")).toEqual([]);
});

test("a password shown as text at submit still prompts", async () => {
  await setMode(harness, "no-fields");
  const page = await openFixture(harness, "save-login.html");
  const ui = inlineOf(page);
  await ready(ui, 2);
  await page.locator("#username").fill("newuser");
  await page.locator("#password").fill("typed-pw");
  await page.locator("#reveal").click();
  await expect(page.locator("#password")).toHaveAttribute("type", "text");
  await page.locator("#submit").click();
  await page.waitForURL("**/landing.html");
  await expect.poll(() => ui.pickerText()).toBe("Save login for 127.0.0.1?AddnewuserCancel");
});

test("the corner prompt stays in view when the window narrows", async () => {
  await setMode(harness, "no-fields");
  const page = await openFixture(harness, "save-login.html");
  const ui = inlineOf(page);
  await signIn(page, ui, "newuser", "typed-pw");
  await page.waitForURL("**/landing.html");
  await expect.poll(() => ui.popoverOpen()).toBe(true);
  await page.setViewportSize({ width: 600, height: 600 });
  // `.picker` is `box-sizing: border-box`, so left + width is its right edge.
  await expect
    .poll(
      async () =>
        parseFloat((await ui.style(".picker", "left")) ?? "NaN") +
        parseFloat((await ui.style(".picker", "width")) ?? "NaN"),
    )
    .toBeLessThanOrEqual(600);
  expect(await ui.popoverOpen()).toBe(true);
});
