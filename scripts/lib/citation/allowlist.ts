/**
 * The pinned allow-lists: the dependency sources a comment may cite, and the
 * upstream trackers whose numbers a comment may cite with the project named.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { check, ROOT, sortedSet } from "./report";

/**
 * The dependency sources a comment may cite, as `crate version`.
 *
 * SORTED, DEDUPLICATED AND FIXED IN LENGTH, all three asserted below. The point
 * of the pin is that widening it costs a diff in this file rather than
 * happening as a side effect of somebody's comment.
 *
 * Every version here is checked against `src-tauri/Cargo.lock` on every run, so
 * an entry cannot survive the bump that invalidates it. A crate that is NOT on
 * this list is not citable at all: `Cargo.lock` pins far more than two crates,
 * and the ones listed are the ones whose internals this repository actually
 * reasons about.
 *
 * DO NOT REBUILD THIS LIST FROM A VERSION SCAN. An earlier draft of it was
 * assembled by collecting the versions that appear in comments, and that method
 * is structurally blind to a crate cited WITHOUT one: a crate named in two
 * comments with no version beside it is invisible to a scan keyed on versions,
 * and the list came out an entry short. The set is a statement about which
 * dependencies this code reasons about, not a summary of what the comments
 * happen to say, and only the lockfile can confirm an entry.
 */
export const THIRD_PARTY_SOURCES = ["tauri 2.11.5", "tauri-plugin-window-state 2.4.1"];

/**
 * The upstream projects whose public tracker a comment may cite by number IN
 * THE SPACE-SEPARATED FORM.
 *
 * Same discipline, same reason. A tracker number is reachable only through the
 * project that hosts it, so the project name is the citation and the number is
 * the argument. This project's own tracker is deliberately absent: it is not in
 * the tree, so nothing a clone holds can open it.
 *
 * THREE AND NOT FOUR, and the entry that came off is worth a note because
 * keeping it would have misrepresented the assertion beside it. A fourth entry
 * named a project cited only in the ATTACHED form, and the attached form never
 * consults this list: `BARE_TRACKER` refuses to fire when a word character
 * precedes the hash, which is what lets `PKCS#8` through and, with it, any
 * `project#123`. So that entry was never read, while "a sorted set of exactly
 * four entries" read as an assurance that all four were load-bearing. An
 * unexercised allow-list entry is the same shape as an unexercised exemption,
 * and this file refuses those elsewhere for the same reason. If the spaced form
 * of that project ever appears, the fix is one line and a visible diff, which is
 * what a pinned list is for.
 */
export const UPSTREAM_TRACKERS = ["IronRDP", "Kitty", "xterm.js"];

export function runAllowListChecks(): void {
  check(
    "the third-party source allow-list is a sorted set of exactly 2 entries",
    sortedSet(THIRD_PARTY_SOURCES) && THIRD_PARTY_SOURCES.length === 2,
    THIRD_PARTY_SOURCES,
  );
  check(
    "the upstream tracker allow-list is a sorted set of exactly 3 entries",
    sortedSet(UPSTREAM_TRACKERS) && UPSTREAM_TRACKERS.length === 3,
    UPSTREAM_TRACKERS,
  );

  /** Every `name`/`version` pair `Cargo.lock` declares. */
  const lockVersions = new Map<string, string>();
  for (const m of readFileSync(join(ROOT, "src-tauri/Cargo.lock"), "utf8").matchAll(
    /\[\[package\]\]\r?\nname = "([^"]+)"\r?\nversion = "([^"]+)"/g,
  )) {
    lockVersions.set(m[1], m[2]);
  }
  check("Cargo.lock parsed into package versions", lockVersions.size > 100, lockVersions.size);
  for (const entry of THIRD_PARTY_SOURCES) {
    const [crate, version] = entry.split(" ");
    check(`Cargo.lock still pins ${crate} at ${version}`, lockVersions.get(crate) === version, {
      lockfile: lockVersions.get(crate) ?? null,
    });
  }
}
