import { defineConfig } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

// The live-site run behind `pnpm test:sites`, kept out of `pnpm test` and CI:
// it loads real login pages, which change and block automated browsers. One
// test per row of the Sites table in `sites.md`; see that file for the rules.
export default defineConfig({
  testDir: path.dirname(fileURLToPath(import.meta.url)),
  testMatch: "sites.spec.ts",
  // Three loads of up to 30 s plus the 15 s field wait each, then the paths.
  timeout: 240_000,
  expect: { timeout: 10_000 },
  workers: 1,
  reporter: [["list"]],
  // A popup with no option must fail the path, not hang the whole site.
  use: { headless: true, actionTimeout: 15_000 },
  projects: [{ name: "chrome" }],
});
