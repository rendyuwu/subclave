/**
 * Shared plumbing for the comment-citation checks: the repository root, the
 * directories and files the rule binds, the check counter and its reporter.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

/**
 * The directories the rule binds. Everything else is generated or vendored.
 */
export const ROOT_DIRS = ["scripts", "src", "src-tauri/src"];

/**
 * Hand-written source that sits in no scanned directory.
 *
 * Both are single files at a level that holds mostly generated or vendored
 * content, so scanning their directories would sweep in far more than the rule
 * governs. Named individually and asserted to exist, because a typo here would
 * silently scan nothing and read as coverage.
 *
 * Not exhaustive over the repository root by design: `tsconfig.json` carries a
 * long comment, but `.json` is not a syntax this check's extractor knows, and
 * inventing one for a file with a single comment in it would be a worse trade
 * than the gap. That gap is real and is named here rather than left implicit.
 */
export const ROOT_FILES = ["src-tauri/build.rs", "vite.config.ts"];

export let failed = 0;
export function check(label: string, ok: boolean, detail?: unknown): void {
  if (ok) {
    console.log(`  ok: ${label}`);
    return;
  }
  console.error(`  FAIL: ${label}`, detail === undefined ? "" : JSON.stringify(detail, null, 1));
  failed++;
}

export const sortedSet = (xs: string[]): boolean =>
  xs.every((x, i) => i === 0 || xs[i - 1] < x) && new Set(xs).size === xs.length;
