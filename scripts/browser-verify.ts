/**
 * Cross-file contract checks for the browser integration.
 * Run: `pnpm run verify browser`.
 *
 * WHY IT EXISTS. The wire contract is authored twice on purpose: once in Rust
 * (`src-tauri/src/modules/browser/protocol.rs` and the proxy's
 * `src-tauri/subclave-proxy/src/frame.rs`) and once in the extension
 * (`extension/src/lib/protocol.ts`). Each half compiles and tests green on its
 * own while the two disagree, and the mismatch only shows up at runtime as a
 * silently unhandled action or an extension that can never authenticate. This
 * script is the one place that reads both trees and asserts they agree, plus
 * the three derived facts (the extension id the manifest's key produces, the
 * registry key set the Rust writer and the NSIS uninstaller must share, and the
 * sidecar name the bundle config promises).
 */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

let failed = 0;
function check(label: string, ok: boolean, detail?: string): void {
  if (ok) {
    console.log(`  ok   ${label}`);
  } else {
    failed += 1;
    console.error(`  FAIL ${label}${detail ? `: ${detail}` : ""}`);
  }
}

/** `AppNotRunning` -> `app-not-running`, the serde `rename_all = "kebab-case"`. */
function kebabCase(pascal: string): string {
  return pascal.replace(/([a-z0-9])([A-Z])/g, "$1-$2").toLowerCase();
}

/** Variant identifiers of `pub enum NmError { ... }`. */
function rustErrorVariants(src: string): string[] {
  const block = src.match(/pub enum NmError \{([^}]*)\}/);
  if (!block) return [];
  return block[1]
    .split(/[\n,]/)
    .map((line) => line.trim())
    .filter((line) => /^[A-Z][A-Za-z0-9]*$/.test(line));
}

/** The quoted members of `export type NmErrorCode = "a" | "b" | ...;`. */
function tsErrorCodes(src: string): string[] {
  const union = src.match(/export type NmErrorCode =([^;]*);/);
  if (!union) return [];
  return [...union[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}

/** The right-hand side of a `const NAME = VALUE;` on either side, normalized. */
function constRhs(src: string, name: string): string | null {
  const re = new RegExp(`(?:pub const ${name}: [A-Za-z0-9_:]+|export const ${name}) = ([^;]+);`);
  const m = src.match(re);
  return m ? m[1].replace(/\s+/g, "") : null;
}

const protocolRs = readFileSync(join(ROOT, "src-tauri/src/modules/browser/protocol.rs"), "utf8");
const frameRs = readFileSync(join(ROOT, "src-tauri/subclave-proxy/src/frame.rs"), "utf8");
const manifestsRs = readFileSync(join(ROOT, "src-tauri/src/modules/browser/manifests.rs"), "utf8");
const protocolTs = readFileSync(join(ROOT, "extension/src/lib/protocol.ts"), "utf8");
const chromeManifest = JSON.parse(
  readFileSync(join(ROOT, "extension/manifest.chrome.json"), "utf8"),
) as { key?: string };

console.log("[parser] the extractors recognize their own shapes");
{
  check("kebabCase(AppNotRunning)", kebabCase("AppNotRunning") === "app-not-running");
  check("kebabCase(NoMatch)", kebabCase("NoMatch") === "no-match");
  check(
    "rustErrorVariants finds the enum",
    rustErrorVariants("pub enum NmError {\n    BadRequest,\n    TooLarge,\n}").length === 2,
  );
}

console.log("\n[errors] the ten NmError codes agree");
{
  const rust = rustErrorVariants(protocolRs).map(kebabCase).sort();
  const ts = [...tsErrorCodes(protocolTs)].sort();
  check(
    "all ten codes, same set",
    rust.length === 10 && ts.length === 10 && rust.join(",") === ts.join(","),
    `rust=[${rust.join(",")}] ts=[${ts.join(",")}]`,
  );
}

console.log("\n[constants] version, host name and frame caps agree");
{
  check("PROTOCOL_VERSION", constRhs(protocolRs, "PROTOCOL_VERSION") === "1");
  check("protocol.ts PROTOCOL_VERSION", constRhs(protocolTs, "PROTOCOL_VERSION") === "1");

  const host = manifestsRs.match(/pub const HOST_NAME: &str = "([^"]+)"/)?.[1];
  const nativeHost = protocolTs.match(/export const NATIVE_HOST = "([^"]+)"/)?.[1];
  check("NATIVE_HOST === HOST_NAME", host === nativeHost, `${nativeHost} vs ${host}`);

  for (const cap of ["MAX_REQUEST_FRAME", "MAX_RESPONSE_FRAME", "MAX_STREAM_FRAME"]) {
    const rs = constRhs(frameRs, cap);
    const tsValue = constRhs(protocolTs, cap);
    check(cap, rs !== null && rs === tsValue, `rust=${rs} ts=${tsValue}`);
  }

  // The proxy authors the error-envelope version itself (it cannot see
  // `protocol.rs`), so the duplicate is pinned here rather than left to drift.
  const frameVersion = frameRs.match(/pub fn error_frame[\s\S]*?"v":\s*(\d+)/)?.[1];
  check(
    "error_frame envelope version === PROTOCOL_VERSION",
    frameVersion !== null && frameVersion === constRhs(protocolRs, "PROTOCOL_VERSION"),
    `frame=${frameVersion}`,
  );
}

console.log("\n[extension id] the manifest key derives the pinned id");
{
  const key = chromeManifest.key ?? "";
  const der = Buffer.from(key, "base64");
  const nibbles = createHash("sha256").update(der).digest("hex").slice(0, 32);
  const derived = [...nibbles]
    .map((nibble) => String.fromCharCode("a".charCodeAt(0) + parseInt(nibble, 16)))
    .join("");
  console.log(`  derived extension id: ${derived}`);
  const pinned = manifestsRs.match(/pub const CHROMIUM_EXTENSION_ID: &str = "([^"]+)"/)?.[1];
  check(
    "sha256(DER SPKI) a-p === CHROMIUM_EXTENSION_ID",
    derived.length === 32 && derived === pinned,
    `derived=${derived} pinned=${pinned}`,
  );
}

console.log("\n[registry] the writer and the uninstaller name the same keys");
{
  const inRust = new Set(
    [...manifestsRs.matchAll(/r"([^"]+NativeMessagingHosts\\[^"]+)"/g)].map((m) => m[1]),
  );
  const inNsh = new Set(
    [
      ...readFileSync(join(ROOT, "src-tauri/installer.nsh"), "utf8").matchAll(
        /DeleteRegKey HKCU "([^"]+)"/g,
      ),
    ].map((m) => m[1]),
  );
  const same = inRust.size === inNsh.size && [...inRust].every((path) => inNsh.has(path));
  check(
    "four identical HKCU paths",
    same,
    `rust=${[...inRust].join(" | ")} nsh=${[...inNsh].join(" | ")}`,
  );
}

console.log("\n[bundle] the sidecar rides externalBin");
{
  const conf = JSON.parse(readFileSync(join(ROOT, "src-tauri/tauri.conf.json"), "utf8")) as {
    bundle?: { externalBin?: string[] };
  };
  check(
    'externalBin names "binaries/subclave-proxy"',
    conf.bundle?.externalBin?.includes("binaries/subclave-proxy") ?? false,
  );
}

if (failed > 0) {
  console.error(`\nbrowser-verify: ${failed} check(s) failed`);
  process.exit(1);
}
console.log("\nbrowser-verify: all checks passed");
