import { expect, test } from "@playwright/test";
import type { Frame, Page } from "@playwright/test";
import { appendFileSync, existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { FAKE_ENTRIES, FAKE_PASSWORD } from "./support/fake-app";
import { openPopup, startHarness, type Harness } from "./support/harness";
import { clickInline, inlineOf, waitForOptions } from "./support/inline";

// The live-site checklist (`sites.md`), automated. The E2E build's fake app
// answers every page with the same fake logins, so no account and no app are
// needed, and nothing is submitted: a path is `ok` once the page's login
// fields hold the fake values. Run with `pnpm test:sites` (add `--headed` to
// watch it, `--grep GitHub` for one site).

type Site = { n: number; name: string; url: string };
type Outcome = "ok" | "miss" | "blocked";

const SITES_MD = path.join(path.dirname(fileURLToPath(import.meta.url)), "sites.md");
const OPTION = '[role="option"]';
const USERNAMES = FAKE_ENTRIES.map((e) => e.username);
/** How long a page gets to show a login field before it counts as blocked: a
 *  bot check, an error page and a moved login page all show none. */
const FIELD_WAIT_MS = 15_000;

/** The rows of the `## Sites` table: `| # | Site | Login page |`. */
function readSites(): Site[] {
  const text = readFileSync(SITES_MD, "utf8");
  const table = text.slice(text.indexOf("## Sites"), text.indexOf("## Runs"));
  const sites: Site[] = [];
  for (const line of table.split("\n")) {
    const cells = line.split("|").map((c) => c.trim());
    const n = Number(cells[1]);
    if (!Number.isInteger(n) || n <= 0) continue;
    sites.push({ n, name: cells[2], url: cells[3].replace(/`/g, "") });
  }
  return sites;
}

type FieldState = { login: boolean; user: boolean; pass: boolean; hasPass: boolean };

const NO_FIELDS: FieldState = { login: false, user: false, pass: false, hasPass: false };

/** `work`, or `fallback` once `ms` pass: a page that keeps its main thread
 *  busy (a proof-of-work bot check) never answers an evaluate. */
function within<T>(work: Promise<T>, ms: number, fallback: T): Promise<T> {
  return Promise.race([
    work.catch(() => fallback),
    new Promise<T>((resolve) => setTimeout(() => resolve(fallback), ms)),
  ]);
}

/** What one frame's inputs hold, open shadow roots included. `login` is any
 *  visible field a login form would have. */
function fieldState(frame: Frame): Promise<FieldState> {
  return within(
    frame.evaluate(
      ({ users, password }) => {
        const inputs: HTMLInputElement[] = [];
        const walk = (root: Document | ShadowRoot) => {
          inputs.push(...root.querySelectorAll("input"));
          for (const el of root.querySelectorAll("*")) if (el.shadowRoot) walk(el.shadowRoot);
        };
        walk(document);
        // What a person sees: laid out, not hidden or transparent, not
        // parked at negative page coordinates (an off-screen honeypot), and
        // not a decoy outside both the tab order and the accessibility tree.
        // Many two-step logins keep the next step's password field in the DOM
        // one of these ways, and the extension rightly leaves it alone.
        const visible = inputs.filter((i) => {
          const box = i.getBoundingClientRect();
          return (
            !(i.tabIndex < 0 && i.closest('[aria-hidden="true"]')) &&
            box.width > 0 &&
            box.height > 0 &&
            i.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true }) &&
            box.right + scrollX > 0 &&
            box.bottom + scrollY > 0
          );
        });
        return {
          login: visible.some(
            (i) =>
              i.type === "password" ||
              i.type === "email" ||
              /username|email/.test(i.autocomplete) ||
              /user|login|email|mail|account|identifier/i.test(`${i.name} ${i.id}`),
          ),
          user: inputs.some((i) => i.type !== "password" && users.includes(i.value)),
          pass: inputs.some((i) => i.type === "password" && i.value === password),
          hasPass: visible.some((i) => i.type === "password"),
        };
      },
      { users: USERNAMES, password: FAKE_PASSWORD },
    ),
    3_000,
    NO_FIELDS,
  );
}

async function pageState(page: Page): Promise<FieldState> {
  const all = await Promise.all(page.frames().map(fieldState));
  return {
    login: all.some((s) => s.login),
    user: all.some((s) => s.user),
    pass: all.some((s) => s.pass),
    hasPass: all.some((s) => s.hasPass),
  };
}

/** A fresh load of the login page, waiting for its first login field, or the
 *  reason none showed. */
async function load(h: Harness, url: string): Promise<{ page: Page; blocked: string | null }> {
  const page = await h.context.newPage();
  try {
    await page.goto(url, { waitUntil: "domcontentloaded", timeout: 30_000 });
    const deadline = Date.now() + FIELD_WAIT_MS;
    while (!(await pageState(page)).login) {
      if (Date.now() > deadline) {
        return { page, blocked: `no login field ("${await within(page.title(), 3_000, "")}")` };
      }
      await page.waitForTimeout(500);
    }
    // Room for the content script's detection debounce.
    await page.waitForTimeout(1_500);
    return { page, blocked: null };
  } catch (e) {
    return { page, blocked: String(e).split("\n")[0] };
  }
}

const PATHS: Record<string, (h: Harness, page: Page) => Promise<void>> = {
  async inline(_h, page) {
    const ui = inlineOf(page);
    if ((await ui.count(".icon")) === 0) throw new Error("no inline icon");
    await clickInline(page, ui, ".icon", 0);
    await waitForOptions(page, ui);
    await clickInline(page, ui, OPTION, 0);
  },
  async popup(h, page) {
    const popup = await openPopup(h.context, h.extensionId);
    await page.bringToFront();
    await popup.getByRole("option").first().click();
    await popup.close();
  },
  async command(h, page) {
    // As in `fill.spec.ts`: a synthesized key never reaches chrome.commands,
    // and headless Chromium resolves openPopup() without a popup, so the
    // command's own message is sent and it takes its newest-match fallback.
    const popup = await openPopup(h.context, h.extensionId);
    const [worker] = h.context.serviceWorkers();
    await worker.evaluate(() => {
      const action: { openPopup?: () => Promise<void> } = chrome.action;
      delete action.openPopup;
    });
    await page.bringToFront();
    await popup.evaluate(() => chrome.runtime.sendMessage({ type: "fill-command" }));
    await popup.close();
  },
};

for (const site of readSites()) {
  test(`${site.n} ${site.name}`, async ({}, testInfo) => {
    const harness = await startHarness(testInfo);
    const paths: Outcome[] = [];
    const notes: string[] = [];
    let form = "other";
    try {
      for (const [name, run] of Object.entries(PATHS)) {
        const { page, blocked } = await load(harness, site.url);
        if (blocked) {
          paths.push("blocked");
          notes.push(blocked);
        } else {
          try {
            await run(harness, page);
            await page.waitForTimeout(1_000);
            const state = await pageState(page);
            const ok = state.user && (state.pass || !state.hasPass);
            paths.push(ok ? "ok" : "miss");
            form = state.hasPass ? "one page" : "two step";
            if (!ok) notes.push(`${name}: fields not filled`);
          } catch (e) {
            paths.push("miss");
            notes.push(`${name}: ${String(e).split("\n")[0]}`);
          }
        }
        // A renderer kept busy by a proof-of-work check never acknowledges a
        // close; leave it to the context instead of waiting minutes.
        await within(page.close(), 5_000, undefined);
      }
    } finally {
      await within(harness.context.close(), 15_000, undefined);
    }

    // One row per site, appended as it finishes: a failed test restarts the
    // worker, so nothing collected in module state survives to the end. The
    // output directory is emptied at the start of every run.
    const note = [...new Set(notes)].join("; ").replace(/\|/g, "/").slice(0, 160);
    // No path filled, but at least one never reached a login form: the site
    // is unproven, not failed, and goes to the hand check.
    const blocked = !paths.includes("ok") && paths.includes("blocked");
    const pass = paths.includes("ok") ? "yes" : blocked ? "blocked" : "no";
    const out = path.join(testInfo.project.outputDir, "sites-run.md");
    if (!existsSync(out)) {
      appendFileSync(
        out,
        "| # | Form | Inline | Popup | Command | Pass | Notes |\n| - | - | - | - | - | - | - |\n",
      );
    }
    appendFileSync(out, `| ${site.n} | ${form} | ${paths.join(" | ")} | ${pass} | ${note} |\n`);

    test.skip(blocked, `blocked: check by hand (${note})`);
    expect(paths, note).toContain("ok");
  });
}
