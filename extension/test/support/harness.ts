import { chromium } from "@playwright/test";
import type { BrowserContext, Page, TestInfo, Worker } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  CONNECTS_KEY,
  CREDENTIALS_KEY,
  FAKE_CLIENT_ID,
  FAKE_SECRET_B64,
  LOG_KEY,
  MODE_KEY,
  type FakeMode,
  type LogEntry,
} from "./fake-app";

declare global {
  interface Window {
    /** Event counters the fixtures keep, keyed `<field>:<event>`. */
    __events: Record<string, number>;
  }
}

// Shared launch mechanics for the extension specs, and the one place the fixture
// port is written down (`playwright.config.ts` starts the server on it).

export const FIXTURE_PORT = 41731;
export const FIXTURE_ORIGIN = `http://127.0.0.1:${FIXTURE_PORT}`;

export const EXTENSION_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");

// `chrome` loads the normal E2E build; `chrome-inject` loads the one whose
// manifest has no `content_scripts`, so the executeScript fallback is exercised.
const DIST_BY_PROJECT: Record<string, string> = {
  chrome: "e2e-chrome",
  "chrome-inject": "e2e-chrome-inject",
};

/**
 * Playwright's only way into an MV3 extension is a persistent context with the
 * unpacked build loaded. `channel: "chromium"` is the new headless mode, which
 * does load unpacked extensions here (`xvfb-run` is installed too, so a headed
 * run stays available if a runner ever refuses them).
 */
export function launchExtension(projectName: string, profileDir: string): Promise<BrowserContext> {
  const dist = path.resolve(EXTENSION_DIR, "dist", DIST_BY_PROJECT[projectName] ?? "e2e-chrome");
  return chromium.launchPersistentContext(profileDir, {
    channel: "chromium",
    headless: true,
    args: [`--disable-extensions-except=${dist}`, `--load-extension=${dist}`],
  });
}

export async function serviceWorker(context: BrowserContext): Promise<Worker> {
  const [existing] = context.serviceWorkers();
  return existing ?? (await context.waitForEvent("serviceworker"));
}

/** The popup runs as an ordinary tab; its id comes from the service worker URL. */
export async function openPopup(context: BrowserContext, extensionId: string): Promise<Page> {
  const popup = await context.newPage();
  await popup.goto(`chrome-extension://${extensionId}/popup.html`);
  return popup;
}

export type Harness = { context: BrowserContext; worker: Worker; extensionId: string };

/** Launches the extension with a clean store, paired, in mode `ok`. */
export async function startHarness(
  testInfo: TestInfo,
  { showInLoginFields = true }: { showInLoginFields?: boolean } = {},
): Promise<Harness> {
  const context = await launchExtension(testInfo.project.name, testInfo.outputPath("profile"));
  const worker = await serviceWorker(context);
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
        "subclave.showInLoginFields": seed.showInLoginFields,
      });
    },
    {
      modeKey: MODE_KEY,
      logKey: LOG_KEY,
      connectsKey: CONNECTS_KEY,
      credentialsKey: CREDENTIALS_KEY,
      credentials: { clientId: FAKE_CLIENT_ID, secret: FAKE_SECRET_B64 },
      showInLoginFields,
    },
  );
  return { context, worker, extensionId: new URL(worker.url()).host };
}

export async function openFixture(h: Harness, name: string): Promise<Page> {
  const page = await h.context.newPage();
  await page.goto(`${FIXTURE_ORIGIN}/${name}`);
  await page.bringToFront();
  return page;
}

export async function setMode(h: Harness, mode: FakeMode): Promise<void> {
  await h.worker.evaluate(({ key, value }) => chrome.storage.local.set({ [key]: value }), {
    key: MODE_KEY,
    value: mode,
  });
}

export async function readLog(h: Harness): Promise<LogEntry[]> {
  const raw: unknown = await h.worker.evaluate(async (key) => {
    const stored = await chrome.storage.local.get([key]);
    return stored[key];
  }, LOG_KEY);
  // The fake app is the only writer of this key and only ever stores its own
  // entries, so the shape is known.
  return Array.isArray(raw) ? (raw as LogEntry[]) : [];
}

export async function readConnects(h: Harness): Promise<number> {
  const raw: unknown = await h.worker.evaluate(async (key) => {
    const stored = await chrome.storage.local.get([key]);
    return stored[key];
  }, CONNECTS_KEY);
  return typeof raw === "number" ? raw : -1;
}
