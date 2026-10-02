import { defineConfig } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { FIXTURE_PORT } from "./support/harness";

// Two projects, one per E2E build:
//
//   chrome         dist/e2e-chrome         (declarative content script)
//   chrome-inject  dist/e2e-chrome-inject  (no content_scripts key, so every
//                                           fill has to go through the
//                                           chrome.scripting.executeScript
//                                           fallback)
//
// `inline.spec.ts`, `guard.spec.ts` and `save.spec.ts` run under `chrome` only:
// the inline UI lives in the declarative content script, which `chrome-inject`
// does not have.
//
// Both carry the same manifest `key`, so the extension id, and therefore the
// native host name, are identical.
//
// Two environment facts the specs are written around:
//
//   1. `chrome.action.openPopup()` cannot render a usable popup in a headless
//      persistent context: it resolves without one. The command specs delete
//      the API first, which is the same condition as Firefox and Chrome before
//      127, and the multi-match branch then takes its documented
//      newest-`lastUsedAt` fallback. The real openPopup path needs a headed
//      browser, so these headless specs do not exercise it.
//   2. A key press synthesized over CDP does not reach Chrome's own
//      `chrome.commands` handling, so `fill.spec.ts` falls back to sending the
//      service worker's internal `{ type: "fill-command" }` message from the
//      popup page, which is the same function the command listener calls.
//
// New-headless Chromium does load unpacked extensions on this machine
// (xvfb-run is installed as well). Should a runner ever refuse, run the same
// projects under `xvfb-run -a pnpm test` with `headless: false`.
export default defineConfig({
  testDir: path.dirname(fileURLToPath(import.meta.url)),
  timeout: 60_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  // Extension storage and the fixture origin are process-global; serial keeps
  // the connect counting in `fill.spec.ts` honest.
  workers: 1,
  reporter: [["list"]],
  use: {
    headless: true,
    baseURL: `http://127.0.0.1:${FIXTURE_PORT}`,
  },
  webServer: {
    command: "node test/support/static-server.mjs",
    cwd: path.resolve(path.dirname(fileURLToPath(import.meta.url)), ".."),
    env: { E2E_PORT: String(FIXTURE_PORT) },
    port: FIXTURE_PORT,
    reuseExistingServer: !process.env.CI,
    stdout: "ignore",
  },
  // `sites.spec.ts` loads live sites and runs only under `test/sites.config.ts`.
  projects: [
    { name: "chrome", testIgnore: ["**/sites.spec.ts"] },
    {
      name: "chrome-inject",
      testIgnore: ["**/inline.spec.ts", "**/guard.spec.ts", "**/save.spec.ts", "**/sites.spec.ts"],
    },
  ],
});
