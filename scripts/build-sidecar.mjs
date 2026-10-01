#!/usr/bin/env node
/**
 * Build the `subclave-proxy` sidecar and stage it where Tauri's `externalBin`
 * expects it: `src-tauri/binaries/subclave-proxy-<target-triple>`.
 *
 * Tauri checks the target triple in the filename, and cargo only nests its
 * output under `target/<triple>/` when `--target` is passed, so the triple is
 * resolved first and passed to cargo whenever one is known.
 *
 * The profile follows the app's, not this script's flag alone: a debug app
 * computes the `.dev` socket name, so a release sidecar next to it would be a
 * silently dead channel. `tauri dev` and `tauri build --debug` both set
 * `TAURI_ENV_DEBUG`.
 *
 * Usage: node scripts/build-sidecar.mjs [--debug|--release]
 */
import { spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..");
const srcTauri = join(repoRoot, "src-tauri");

const args = process.argv.slice(2);
const forceRelease = args.includes("--release");
const forceDebug = args.includes("--debug");
const debug = forceRelease ? false : forceDebug || process.env.TAURI_ENV_DEBUG === "true";
const profile = debug ? "debug" : "release";

/** The triple Tauri will look for, or null when none can be determined. */
function targetTriple() {
  const fromEnv = process.env.TAURI_ENV_TARGET_TRIPLE || process.env.CARGO_BUILD_TARGET;
  if (fromEnv) return fromEnv;
  const probe = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  if (probe.status !== 0) return null;
  const host = (probe.stdout || "").split("\n").find((line) => line.startsWith("host:"));
  return host ? host.slice("host:".length).trim() : null;
}

const triple = targetTriple();
const exe = process.platform === "win32" ? ".exe" : "";

const cargoArgs = ["build", "-p", "subclave-proxy"];
if (!debug) cargoArgs.push("--release");
if (triple) cargoArgs.push("--target", triple);

console.log(`build-sidecar: cargo ${cargoArgs.join(" ")} (${profile})`);
const build = spawnSync("cargo", cargoArgs, { cwd: srcTauri, stdio: "inherit" });
if (build.status !== 0) {
  process.exit(build.status ?? 1);
}

const sourceDir = triple
  ? join(srcTauri, "target", triple, profile)
  : join(srcTauri, "target", profile);
const source = join(sourceDir, `subclave-proxy${exe}`);
if (!existsSync(source)) {
  console.error(`build-sidecar: expected binary missing at ${source}`);
  process.exit(1);
}

const binariesDir = join(srcTauri, "binaries");
mkdirSync(binariesDir, { recursive: true });
const name = triple ? `subclave-proxy-${triple}${exe}` : `subclave-proxy${exe}`;
const dest = join(binariesDir, name);
if (!triple) {
  console.warn(
    "build-sidecar: no target triple resolved; staged without the triple suffix, which externalBin cannot use.",
  );
}
copyFileSync(source, dest);
chmodSync(dest, 0o755);
console.log(`build-sidecar: staged ${dest}`);
