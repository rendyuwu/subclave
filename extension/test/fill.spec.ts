import { expect, test } from "@playwright/test";
import type { BrowserContext, Page, Worker } from "@playwright/test";
import {
  CONNECTS_KEY,
  CREDENTIALS_KEY,
  DROP_AUTH_KEY,
  FAKE_CLIENT_ID,
  FAKE_ENTRIES,
  FAKE_GENERATED,
  FAKE_PASSWORD,
  FAKE_SECRET_B64,
  LOG_KEY,
  MODE_KEY,
  type FakeMode,
  type LogEntry,
} from "./support/fake-app";
import { FIXTURE_ORIGIN, launchExtension, openPopup, serviceWorker } from "./support/harness";

declare global {
  interface Window {
    __events: Record<string, number>;
  }
}

type Harness = { context: BrowserContext; worker: Worker; extensionId: string };

let harness: Harness;

test.beforeEach(async ({}, testInfo) => {
  const context = await launchExtension(testInfo.project.name, testInfo.outputPath("profile"));
  const worker = await serviceWorker(context);
  harness = { context, worker, extensionId: new URL(worker.url()).host };
  // The fake app's mode, log and connect count all live in storage because an
  // MV3 worker can be suspended between steps; the credentials are seeded so the
  // browser starts out paired, which every non-pairing test wants.
  await worker.evaluate(
    async (seed) => {
      await chrome.storage.local.clear();
      await chrome.storage.local.set({
        [seed.modeKey]: "ok",
        [seed.logKey]: [],
        [seed.connectsKey]: 0,
        [seed.credentialsKey]: seed.credentials,
        "subclave.showInLoginFields": true,
      });
    },
    {
      modeKey: MODE_KEY,
      logKey: LOG_KEY,
      connectsKey: CONNECTS_KEY,
      credentialsKey: CREDENTIALS_KEY,
      credentials: { clientId: FAKE_CLIENT_ID, secret: FAKE_SECRET_B64 },
    },
  );
});

test.afterEach(async () => {
  if (harness) await harness.context.close();
});

async function openFixture(name: string): Promise<Page> {
  const page = await harness.context.newPage();
  await page.goto(`${FIXTURE_ORIGIN}/${name}`);
  await page.bringToFront();
  return page;
}

async function openPopupPage(): Promise<Page> {
  return openPopup(harness.context, harness.extensionId);
}

async function setMode(mode: FakeMode): Promise<void> {
  await harness.worker.evaluate(({ key, value }) => chrome.storage.local.set({ [key]: value }), {
    key: MODE_KEY,
    value: mode,
  });
}

async function readLog(): Promise<LogEntry[]> {
  const raw: unknown = await harness.worker.evaluate(async (key) => {
    const stored = await chrome.storage.local.get([key]);
    return stored[key];
  }, LOG_KEY);
  // The fake app is the only writer of this key and only ever stores its own
  // entries, so the shape is known.
  return Array.isArray(raw) ? (raw as LogEntry[]) : [];
}

async function readConnects(): Promise<number> {
  const raw: unknown = await harness.worker.evaluate(async (key) => {
    const stored = await chrome.storage.local.get([key]);
    return stored[key];
  }, CONNECTS_KEY);
  return typeof raw === "number" ? raw : -1;
}

test("lists every match and fills username and password", async () => {
  const fixture = await openFixture("login.html");
  const popup = await openPopupPage();
  await fixture.bringToFront();

  const options = popup.getByRole("option");
  await expect(options).toHaveCount(FAKE_ENTRIES.length);
  for (const [index, entry] of FAKE_ENTRIES.entries()) {
    await expect(options.nth(index)).toContainText(entry.title);
    await expect(options.nth(index)).toContainText(entry.username);
    await expect(options.nth(index)).toContainText(entry.group);
  }

  await options.first().click();
  await expect(popup.getByText("Filled")).toBeVisible();
  await expect(fixture.locator("#username")).toHaveValue(FAKE_ENTRIES[0].username);
  await expect(fixture.locator("#password")).toHaveValue(FAKE_PASSWORD);

  const events = await fixture.evaluate(() => window.__events);
  expect(events["username:input"]).toBe(1);
  expect(events["username:change"]).toBe(1);
  expect(events["password:input"]).toBe(1);
  expect(events["password:change"]).toBe(1);
});

test("never fills hidden or zero-size password inputs, on any path", async () => {
  const fixture = await openFixture("hidden-fields.html");
  const popup = await openPopupPage();
  await fixture.bringToFront();

  await popup.getByRole("option").first().click();
  await expect(fixture.locator("#password")).toHaveValue(FAKE_PASSWORD);
  await expect(fixture.locator("#hidden-password")).toHaveValue("");
  await expect(fixture.locator("#zero-password")).toHaveValue("");

  await fixture.bringToFront();
  await popup.getByRole("button", { name: "New entry" }).click();
  await expect(fixture.locator("#password")).toHaveValue(FAKE_GENERATED);
  await expect(fixture.locator("#hidden-password")).toHaveValue("");
  await expect(fixture.locator("#zero-password")).toHaveValue("");

  await fixture.bringToFront();
  // Same missing-openPopup condition as the command test, so the command path
  // reaches the fill instead of stopping at an invisible popup.
  await harness.worker.evaluate(() => {
    const action: { openPopup?: () => Promise<void> } = chrome.action;
    delete action.openPopup;
  });
  await popup.evaluate(() => chrome.runtime.sendMessage({ type: "fill-command" }));
  await expect(fixture.locator("#password")).toHaveValue(FAKE_PASSWORD);
  await expect(fixture.locator("#hidden-password")).toHaveValue("");
  await expect(fixture.locator("#zero-password")).toHaveValue("");

  const events = await fixture.evaluate(() => window.__events);
  expect(events["hidden:input"]).toBe(0);
  expect(events["zero:change"]).toBe(0);
});

test("reports a page with no fillable inputs", async () => {
  const fixture = await openFixture("no-fields.html");
  const popup = await openPopupPage();
  await fixture.bringToFront();

  await popup.getByRole("option").first().click();
  await expect(popup.getByText("No login fields found on this page.")).toBeVisible();
});

test("fills through the executeScript fallback when the content script is absent", async ({}, testInfo) => {
  const fixture = await openFixture("login.html");
  if (testInfo.project.name === "chrome-inject") {
    const missing = await harness.worker.evaluate(async () => {
      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      if (!tab?.id) return false;
      try {
        await chrome.tabs.sendMessage(tab.id, { type: "subclave:ping" });
        return false;
      } catch {
        return true;
      }
    });
    expect(missing).toBe(true);
  }

  const popup = await openPopupPage();
  await fixture.bringToFront();
  await popup.getByRole("option").first().click();
  await expect(fixture.locator("#username")).toHaveValue(FAKE_ENTRIES[0].username);
  await expect(fixture.locator("#password")).toHaveValue(FAKE_PASSWORD);
});

test("the fill command fills the newest match", async () => {
  const fixture = await openFixture("login.html");
  const popup = await openPopupPage();
  // Model the browsers without `chrome.action.openPopup` (Firefox, Chrome
  // before 127), which is the condition the newest-lastUsedAt fallback exists
  // for. Headless Chromium resolves openPopup() without ever rendering one, so
  // leaving it in place would swallow the fill instead of falling back.
  await harness.worker.evaluate(() => {
    const action: { openPopup?: () => Promise<void> } = chrome.action;
    delete action.openPopup;
  });
  await fixture.bringToFront();

  await fixture.keyboard.press("Control+Shift+L");
  await fixture.waitForTimeout(750);
  if ((await fixture.locator("#username").inputValue()) === "") {
    // CDP-synthesized keys do not reach Chrome's own chrome.commands handling;
    // this message runs the same function the command listener calls.
    await popup.evaluate(() => chrome.runtime.sendMessage({ type: "fill-command" }));
  }

  // Two matches and no openPopup, so the command takes its documented
  // newest-lastUsedAt fallback.
  await expect(fixture.locator("#username")).toHaveValue(FAKE_ENTRIES[1].username);
  await expect(fixture.locator("#password")).toHaveValue(FAKE_PASSWORD);

  const credential = (await readLog()).filter((entry) => entry.action === "get-credential").at(-1);
  // `get-credential` carries the entry as `id`, not `entryId` (save-login is the
  // one that uses `entryId`).
  expect(credential?.params?.id).toBe(FAKE_ENTRIES[1].id);
  expect(credential?.params?.via).toBe("command");
});

test("generates a password, fills the signup form and saves a new entry", async () => {
  await setMode("no-fields");
  const fixture = await openFixture("signup.html");
  await fixture.locator("#username").fill("newuser");

  const popup = await openPopupPage();
  await fixture.bringToFront();
  await popup.getByRole("button", { name: "Generate for this site" }).click();
  await expect(fixture.locator("#password")).toHaveValue(FAKE_GENERATED);
  await expect(fixture.locator("#password-confirm")).toHaveValue(FAKE_GENERATED);

  const saved = (await readLog()).find((entry) => entry.action === "save-login");
  expect(saved?.params?.username).toBe("newuser");
  expect(saved?.params?.entryId).toBeUndefined();
  expect(saved?.params?.via).toBe("popup");
  expect(saved?.result?.created).toBe(true);
});

test("offers an update per match and keeps the entry id", async () => {
  const fixture = await openFixture("signup.html");
  const popup = await openPopupPage();
  await fixture.bringToFront();

  const update = popup.getByRole("button", { name: `Update ${FAKE_ENTRIES[1].title}` });
  await expect(update).toBeVisible();
  await update.click();
  await expect(fixture.locator("#password")).toHaveValue(FAKE_GENERATED);

  const saved = (await readLog()).find((entry) => entry.action === "save-login");
  expect(saved?.params?.entryId).toBe(FAKE_ENTRIES[1].id);
  expect(saved?.result?.created).toBe(false);
});

test("shows the locked state and focuses the app on Unlock", async () => {
  await setMode("locked");
  const popup = await openPopupPage();

  await expect(popup.getByText("Subclave is locked")).toBeVisible();
  await popup.getByRole("button", { name: "Unlock" }).click();
  await expect
    .poll(async () => (await readLog()).some((entry) => entry.action === "focus-app"))
    .toBe(true);
});

test("shows the not-running state when the app is absent", async () => {
  await setMode("absent");
  const popup = await openPopupPage();

  await expect(
    popup.getByText("Subclave is not running. Start it from your applications menu."),
  ).toBeVisible();
});

test("offers pairing and shows the six-digit code", async () => {
  await setMode("unpaired");
  // The popup runs as an ordinary tab, and `chrome.tabs.query` hides `url` on a
  // `chrome-extension://` tab (no `tabs` permission), so the service worker
  // needs a real page to be the active tab before it can call `get-logins`.
  const fixture = await openFixture("login.html");
  const popup = await openPopupPage();
  await fixture.bringToFront();

  const pair = popup.getByRole("button", { name: "Pair with Subclave" });
  await expect(pair).toBeVisible();
  await pair.click();
  await expect(popup.getByText("Waiting for Subclave")).toBeVisible();
  await expect(popup.getByText(/^\d{6}$/)).toBeVisible();
  await expect
    .poll(async () => (await readLog()).some((entry) => entry.action === "associate"))
    .toBe(true);
});

test("a generate on a page with no password field saves nothing", async () => {
  await setMode("no-fields");
  const fixture = await openFixture("no-fields.html");
  const popup = await openPopupPage();
  await fixture.bringToFront();

  await popup.getByRole("button", { name: "Generate for this site" }).click();
  await expect(popup.getByText("No login fields found on this page.")).toBeVisible();
  expect((await readLog()).some((entry) => entry.action === "save-login")).toBe(false);
});

test("a page load opens no connection and the first popup fill opens one", async () => {
  const fixture = await openFixture("login.html");
  expect(await readConnects()).toBe(0);

  const popup = await openPopupPage();
  await fixture.bringToFront();
  await popup.getByRole("option").first().click();
  await expect(fixture.locator("#password")).toHaveValue(FAKE_PASSWORD);

  expect(await readConnects()).toBe(1);
});

test("re-handshakes a replaced connection instead of forcing a re-pair", async () => {
  const fixture = await openFixture("login.html");
  const popup = await openPopupPage();
  await fixture.bringToFront();
  // Wait for the popup's own state read (a get-logins) to finish, so the drop
  // below lands on the fill's get-credential.
  await expect(popup.getByRole("option")).toHaveCount(FAKE_ENTRIES.length);

  const before = await readConnects();
  // The next authenticated action is answered `not-associated`, exactly as it
  // is after `close_all` drops the socket on lock while the native port stays
  // up and the worker still believes it is authenticated.
  await harness.worker.evaluate(({ key, value }) => chrome.storage.local.set({ [key]: value }), {
    key: DROP_AUTH_KEY,
    value: 1,
  });

  await popup.getByRole("option").first().click();
  await expect(popup.getByText("Filled")).toBeVisible();
  await expect(fixture.locator("#password")).toHaveValue(FAKE_PASSWORD);

  // The credentials survive, so the user is never asked to pair again, and the
  // vault gains no duplicate client.
  const stored: unknown = await harness.worker.evaluate(async (key) => {
    const values = await chrome.storage.local.get([key]);
    return values[key];
  }, CREDENTIALS_KEY);
  expect(stored).toEqual({ clientId: FAKE_CLIENT_ID, secret: FAKE_SECRET_B64 });

  const log = await readLog();
  expect(log.some((entry) => entry.code === "not-associated")).toBe(true);
  expect(log.some((entry) => entry.action === "get-credential" && entry.ok)).toBe(true);
  // A replaced connection costs one reconnect and one silent re-handshake.
  expect(await readConnects()).toBeGreaterThan(before);
});
