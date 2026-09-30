#!/usr/bin/env node
/**
 * Case-collision audit: nothing in the checkout may be ambiguous in case.
 * Run: `npx tsx scripts/case-collision-verify.ts`.
 *
 * macOS and Windows check this repository out on a case-insensitive filesystem,
 * and the CI bundle matrix builds it there. Two tracked paths that differ only
 * in case are two entries in git and one file on those runners: the checkout
 * keeps whichever came last and the other is gone. That is how M2 reached CI:
 * `GroupTree.tsx` and an all-lowercase sibling helper were both checked in, the
 * bundle job kept one of them, `import { Chip } from "./GroupTree"` resolved to
 * the surviving `.ts`, and all four bundle jobs failed on a component nobody
 * had touched.
 *
 * Three ways it bites, so three checks:
 *   A. a directory name repeated in case, so one of the two trees is lost
 *   B. one whole path repeated in case, so one of the two files is lost
 *   C. two module files in one directory sharing a stem in case
 *
 * C is the one that shipped: it breaks even though the extensions differ,
 * because an extensionless specifier is resolved by trying `.ts` before `.tsx`,
 * and on a case-insensitive filesystem both candidates answer to `./GroupTree`.
 * It is scoped to extensions a resolver actually collapses, so `Cargo.lock`
 * beside `Cargo.toml` - a tolerated, unrelated pattern - is not reported.
 *
 * Linux hides all of it and the frontend job runs on Linux, so this check is
 * the only thing that reports the problem before the bundle job has spent
 * minutes compiling Rust.
 */
import { execFileSync } from "node:child_process";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));

/** Extensions `import "./Name"` collapses together; first match wins, in this order. */
const MODULE_EXTENSIONS = [".ts", ".tsx", ".mts", ".js", ".jsx", ".mjs", ".cjs"];

// Exactly the paths a fresh checkout materialises. Untracked files cannot break
// CI, and a local one shadowing a tracked file is the developer's own doing.
const tracked = execFileSync("git", ["-C", root, "ls-files", "-z"], {
  encoding: "utf8",
})
  .split("\0")
  .filter(Boolean);

/** Groups `key -> name` pairs and returns the keys naming more than one spelling. */
function collisions(pairs: Array<[string, string]>): string[][] {
  const byKey = new Map<string, Set<string>>();
  for (const [key, name] of pairs) {
    const bucket = byKey.get(key) ?? new Set<string>();
    bucket.add(name);
    byKey.set(key, bucket);
  }
  return [...byKey.values()].filter((names) => names.size > 1).map((names) => [...names].sort());
}

/** Reports one section and returns how many collisions it found. */
function section(title: string, pairs: Array<[string, string]>, ok: string): number {
  console.log(title);
  const found = collisions(pairs);
  for (const group of found) console.error(`  CLASH: ${group.join(" vs ")}`);
  if (found.length === 0) console.log(`  ok: ${ok}`);
  return found.length;
}

const splitDirectory = (path: string) => {
  const cut = path.lastIndexOf("/");
  return { dir: cut === -1 ? "" : path.slice(0, cut + 1), name: path.slice(cut + 1) };
};

const ownDirs: Array<[string, string]> = [];
const wholePaths: Array<[string, string]> = [];
const moduleStems: Array<[string, string]> = [];
for (const path of tracked) {
  wholePaths.push([path.toLowerCase(), path]);
  const { dir, name } = splitDirectory(path);
  const segments = dir.split("/").filter(Boolean);
  for (let i = 1; i <= segments.length; i++) {
    const ancestor = segments.slice(0, i).join("/");
    ownDirs.push([ancestor.toLowerCase(), ancestor]);
  }
  const ext = MODULE_EXTENSIONS.find((e) => name.endsWith(e));
  if (ext) {
    moduleStems.push([`${dir.toLowerCase()}${name.slice(0, -ext.length).toLowerCase()}`, path]);
  }
}

let failed = 0;
failed += section(
  "[A] directory names repeated in case",
  ownDirs,
  "every directory name is unique",
);
failed += section("[B] whole paths repeated in case", wholePaths, "every path is unique in case");
failed += section(
  "[C] sibling module files sharing a stem in case",
  moduleStems,
  "no two sibling modules share a stem",
);

if (failed > 0) {
  throw new Error(
    `${failed} case collision(s): rename one of each pair, since a case-insensitive checkout cannot hold both.`,
  );
}
console.log("\nAll checks passed: nothing in the checkout is ambiguous in case.");
