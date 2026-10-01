/**
 * Every citation in a source string's COMMENTS that the rule refuses.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { commentRangesOf, lineNumbersFor } from "../comments";

import {
  ANY_PINNED_CRATE,
  BARE_LINE,
  BARE_TRACKER,
  DEP_OPENER,
  DEP_SHAPE,
  DEP_WELL_FORMED,
  NAMED_CITATION,
  TRACKER_PREFIX,
} from "./detectors";
import {
  BACKTICKED,
  exemptDeadPath,
  notAFileSpelling,
  resolveSpelling,
  selfPreferred,
} from "./resolve";

export type Violation = { readonly where: string; readonly kind: string; readonly cite: string };

/**
 * Every citation in `src`'s COMMENTS that the rule refuses.
 *
 * `rel` decides the comment syntax and is what the report names, so it must be
 * the repository-relative spelling rather than an absolute path.
 *
 * EVERY LINE NUMBER HERE COMES FROM THE MATCH'S OWN OFFSET, never from the
 * comment range's start. This file reported the range's start once, and for a
 * block comment that is the line holding the opening delimiter: the three
 * findings it produced pointed at a bare `/**` and a reader following one
 * learned nothing. That is this check's own subject matter, a number that looks
 * checkable and lands on comment filler, reproduced inside the instrument built
 * to remove it. It survived fourteen controls because every one of them was a
 * single-line probe, where the two numbers coincide. Firing and pointing
 * somewhere are different properties and only the first had been tested; the
 * `[lines]` controls test the second.
 */
export function violationsIn(rel: string, src: string): Violation[] {
  const out: Violation[] = [];
  const lineOf = lineNumbersFor(src);
  for (const comment of commentRangesOf(rel, src)) {
    const flag = (kind: string, at: number, cite: string): void => {
      out.push({ where: `${rel}:${lineOf(comment.pos + at)}`, kind, cite });
    };

    // Named first, and its spans remembered: a citation whose colon sits outside
    // a backtick satisfies the bare pattern too, and reporting it twice would
    // claim two defects where the file name is right there in the first one.
    const namedSpans: Array<{ from: number; to: number }> = [];
    for (const m of comment.text.matchAll(NAMED_CITATION)) {
      const at = m.index ?? 0;
      namedSpans.push({ from: at, to: at + m[0].length });
      flag("named", at, m[0]);
    }
    for (const m of comment.text.matchAll(BARE_LINE)) {
      const at = m.index ?? 0;
      if (namedSpans.some((s) => at >= s.from && at < s.to)) continue;
      flag("bare-line", at, m[0].trim());
    }

    for (const m of comment.text.matchAll(BARE_TRACKER)) {
      const at = m.index ?? 0;
      const before = comment.text.slice(0, at);
      if (!TRACKER_PREFIX.test(before)) flag("bare-tracker", at, m[0]);
    }

    // A pinned crate in citation position must complete its triple, on one line,
    // at the version the lockfile pins.
    for (const m of comment.text.matchAll(DEP_OPENER)) {
      const at = m.index ?? 0;
      const rest = comment.text.slice(at);
      if (!DEP_WELL_FORMED.some((re) => re.test(rest))) {
        flag("dep-form", at, rest.slice(0, 72).split("\n")[0]);
      }
    }
    // And a citation shaped like a triple must credit one. Skipped when a pinned
    // crate IS named, because then the more specific `dep-form` owns the finding.
    for (const m of comment.text.matchAll(DEP_SHAPE)) {
      if (!ANY_PINNED_CRATE.test(m[0])) flag("dep-uncredited", m.index ?? 0, m[0]);
    }

    // Finally the REFERENT rather than the shape: every file this comment names
    // has to be a file that exists. The five detectors above all pass a citation
    // that is beautifully formed and points at nothing.
    for (const m of comment.text.matchAll(BACKTICKED)) {
      const spelling = m[1].trim();
      if (notAFileSpelling(spelling) !== null) continue;
      const found = resolveSpelling(rel, spelling);
      // The exemptions are consulted ONLY on a miss, never before resolving.
      // Consulting them first would let an exemption silence an AMBIGUITY, which
      // is a different finding about a file that does exist, and would hide it
      // under a list whose every entry claims the opposite.
      if (found.length === 0) {
        if (exemptDeadPath(rel, spelling) === null) flag("dead-path", m.index ?? 0, spelling);
      } else if (found.length > 1) {
        flag("partial-path", m.index ?? 0, `${spelling} (${found.length} candidates)`);
      } else if (selfPreferred(rel, spelling, found[0])) {
        flag("self-resolved", m.index ?? 0, `${spelling} resolves to the citing file`);
      }
    }
  }
  return out;
}
