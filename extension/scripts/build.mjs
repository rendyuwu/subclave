import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "vite";

// Builds one extension variant into `dist/<variant>`:
//
//   node scripts/build.mjs --target chrome|firefox [--test] [--no-content] [--out <dir>]
//
// `--test` uses `test/support/test-background.ts` as the service worker entry
// and picks the `e2e-chrome` / `e2e-chrome-inject` variant names, which is what
// the Playwright projects load. `--no-content` drops the `content_scripts` key
// from the copied manifest (the `chrome-inject` project exercises the
// `chrome.scripting.executeScript` fallback), never the built `content.js`.

const extDir = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const argv = process.argv.slice(2);
const flag = (name) => argv.includes(name);
const option = (name) => {
  const at = argv.indexOf(name);
  return at >= 0 ? argv[at + 1] : undefined;
};

const test = flag("--test");
const noContent = flag("--no-content");
const target = test ? "chrome" : (option("--target") ?? "chrome");
if (target !== "chrome" && target !== "firefox") {
  console.error(`build: --target must be chrome or firefox, got "${target}"`);
  process.exit(1);
}

const variant = test ? (noContent ? "e2e-chrome-inject" : "e2e-chrome") : target;
const outDir = path.resolve(extDir, option("--out") ?? path.join("dist", variant));

rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });

const backgroundEntry = test
  ? path.resolve(extDir, "test", "support", "test-background.ts")
  : undefined;

const viteConfig = path.resolve(extDir, "vite.config.ts");
for (const entry of ["popup", "background", "content"]) {
  process.env.EXT_ENTRY = entry;
  process.env.EXT_TARGET = target;
  process.env.EXT_OUT = outDir;
  if (entry === "background" && backgroundEntry) {
    process.env.EXT_ENTRY_FILE = backgroundEntry;
  } else {
    delete process.env.EXT_ENTRY_FILE;
  }
  await build({ configFile: viteConfig, logLevel: "warn" });
}

// The content script loads on every page, so its size is a hard budget.
const CONTENT_BUDGET = 30 * 1024;
const contentSize = statSync(path.join(outDir, "content.js")).size;
if (contentSize > CONTENT_BUDGET) {
  console.error(
    `build: content.js is ${contentSize} bytes, over the ${CONTENT_BUDGET}-byte budget`,
  );
  process.exit(1);
}

const manifest = JSON.parse(readFileSync(path.join(extDir, `manifest.${target}.json`), "utf8"));
if (noContent) delete manifest.content_scripts;
writeFileSync(path.join(outDir, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);

const icons = path.join(extDir, "public", "icons");
if (existsSync(icons)) cpSync(icons, path.join(outDir, "icons"), { recursive: true });

console.log(`build: ${variant} -> ${path.relative(process.cwd(), outDir)}`);
