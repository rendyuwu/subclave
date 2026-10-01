import { chromium } from "@playwright/test";
import type { BrowserContext, Page, Worker } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
