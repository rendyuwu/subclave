/**
 * The tree walk and file selection, and the sweep asserting that the whole
 * hand-written tree carries no forbidden citation in a comment.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

import { commentRangesOf, hasKnownCommentSyntax } from "../comments";
import { check, ROOT, ROOT_DIRS, ROOT_FILES } from "./report";

import { REPO_FILE_SET } from "./resolve";
import { type Violation, violationsIn } from "./violations";

function walk(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    if (["node_modules", "target", "dist", "gen", "test-results"].includes(name)) continue;
    const full = join(dir, name);
    if (statSync(full).isDirectory()) walk(full, out);
    else if (hasKnownCommentSyntax(name)) out.push(full);
  }
  return out;
}

/**
 * How many partial spellings the tree may carry, as a CEILING rather than an
 * exact count.
 *
 * A partial spelling is a real citation of a real file whose name happens to be
 * shared, cited from somewhere too far away for the citing directory to break
 * the tie: a script naming a bare derive-file name when three modules have one.
 * Not one of them is a dead reference, and a reader resolves each from the citing
 * script's own subject, so reddening per site would commission an edit in every
 * one of them for no defect found.
 *
 * A ceiling and not an exact number so that FIXING one never reddens the check,
 * while ADDING one does. It ratchets in the direction the repository wants and
 * stays silent in the other. Lowering it as the number falls is a one-line diff
 * with the new number in the failure message.
 */
const PARTIAL_PATH_CEILING = 38;

/**
 * What gets scanned, as labelled units, so a failure names where it came from.
 *
 * The named files are one unit rather than one each, because two checks apiece
 * over a pair of files buys nothing a single labelled group does not.
 */
const SCAN_UNITS: Array<{ label: string; files: string[] }> = [
  ...ROOT_DIRS.map((dir) => ({ label: `${dir}/`, files: walk(join(ROOT, dir)) })),
  { label: "root-level source", files: ROOT_FILES.map((f) => join(ROOT, f)) },
];

export function runTreeChecks(): void {
  // A named file that has moved would silently scan nothing and read as coverage,
  // which is the one failure a list of literal paths has.
  check(
    "every named root-level file is there to be scanned",
    ROOT_FILES.every((f) => REPO_FILE_SET.has(f)),
    ROOT_FILES.filter((f) => !REPO_FILE_SET.has(f)),
  );
  // And the walk as a whole has to have found the tree. Per-unit counts below
  // catch a single root breaking; this catches the walk itself breaking, which
  // would otherwise turn every assertion in this section green over empty lists.
  check(
    "the scan covers the whole hand-written tree",
    SCAN_UNITS.reduce((n, u) => n + u.files.length, 0) > 60,
    SCAN_UNITS.map((u) => `${u.label} ${u.files.length}`),
  );

  let partialTotal = 0;
  const partialSites: string[] = [];
  const unitComments: string[] = [];
  for (const { label: root, files } of SCAN_UNITS) {
    check(`${root} has files with comment syntax to scan`, files.length > 0, files.length);
    // Listed is not the same as READ. A unit whose files all failed to yield a
    // comment would pass every assertion below over an empty set, and that is the
    // shape a newly added root fails in: named correctly, walked correctly, and
    // silently contributing nothing.
    unitComments.push(
      `${root} ${files.reduce((n, f) => n + commentRangesOf(relative(ROOT, f), readFileSync(f, "utf8")).length, 0)}`,
    );

    const found: Violation[] = [];
    for (const file of files) {
      const rel = relative(ROOT, file);
      const src = readFileSync(file, "utf8");
      found.push(...violationsIn(rel, src));
    }

    for (const kind of ["named", "bare-line", "dead-path", "self-resolved"]) {
      const offenders = found.filter((v) => v.kind === kind);
      check(
        `${root} carries no ${kind} citation in a comment`,
        offenders.length === 0,
        offenders.map((v) => `${v.where}  ${v.cite}`),
      );
    }

    const partial = found.filter((v) => v.kind === "partial-path");
    partialTotal += partial.length;
    partialSites.push(...partial.map((v) => `${v.where}  ${v.cite}`));
  }

  check(
    `the tree carries at most ${PARTIAL_PATH_CEILING} partial file spellings (now ${partialTotal})`,
    partialTotal <= PARTIAL_PATH_CEILING,
    partialSites,
  );

  check(
    "every scanned unit yielded comments, so none is listed but unread",
    unitComments.every((u) => Number(u.split(" ").pop()) > 0),
    unitComments,
  );
}
