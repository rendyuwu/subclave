/**
 * Controls for line attribution: a finding names the citation's line, not the
 * comment's opening delimiter.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { check } from "../report";

import { kindsOf, linesOf } from "./support";

/**
 * A `/* *\/ block whose citations sit on its third and fourth lines.
 *
 * Every fixture here opens its comment on file line 2 and puts the citation
 * further down, so a report taken from the comment's opening delimiter answers
 * 2 and a report taken from the match answers 4. Nothing above this section
 * could tell the two apart, because a single-line probe puts them at the same
 * number.
 *
 * This one carries TWO citations, on two different lines of ONE comment. That
 * is the strongest form of the assertion available: no single number can
 * satisfy it, so an implementation reporting anything per-range rather than
 * per-match fails it whichever line it picks.
 */
const C_BLOCK_TWO_LINES = [
  "const a = 1;",
  "/*",
  " * nothing citable on this line",
  " * mirrors the guard at foo.ts:12",
  " * and the other constructor is at `:1183`",
  " */",
  "",
].join("\n");

/** The same in a `/** *\/ docblock, which is the form the three real findings were in. */
const C_DOCBLOCK_LINES = [
  "const a = 1;",
  "/**",
  " * nothing citable on this line",
  " * mirrors the guard at foo.ts:12",
  " */",
  "",
].join("\n");

/** The same in a Rust block comment, which the hand-written scanner reads. */
const C_RUST_BLOCK_LINES = [
  "fn f() {}",
  "/*",
  " * nothing citable on this line",
  " * see the tail at session.rs:484",
  " */",
  "",
].join("\n");

/** And in a stylesheet, the fourth comment syntax and the fourth code path. */
const C_CSS_BLOCK_LINES = [
  ".a { color: red; }",
  "/*",
  " * nothing citable on this line",
  " * mirrors the guard at foo.ts:12",
  " */",
  "",
].join("\n");

/**
 * Two citations OF THE SAME KIND, on two lines of one comment, one fixture per
 * detector. This is the property the four fixtures above do not have.
 *
 * The pair at the top of this section mixes two kinds, so it proves a finding is
 * placed per MATCH rather than per comment. It cannot prove a COUNT: an
 * implementation reporting one finding per kind per comment satisfies it
 * whichever line it picks. The defect this section exists for had both halves at
 * once, one detector naming a single line for two occurrences, and a control
 * that sees only one half is half a control.
 *
 * COVERED EXHAUSTIVELY AND NOT BY SAMPLE, because sampling is what left the gap.
 * When this section held four fixtures they cited three spellings between them
 * and exercised two detectors; the other must-be-zero kinds, and the bounded
 * one, had no line control at all. The two that did were the two those
 * three spellings happen to exercise, which is not a reason.
 *
 * Every fixture keeps the section's convention: the comment opens on file line 2
 * and both citations sit on lines 4 and 5, so the assertion is a pair that no
 * single number satisfies and that nothing but a per-match report produces.
 *
 * THE REPORT THAT PROMPTED THESE HAS SINCE BEEN RE-RUN, and it does not
 * reproduce. A project name ending in a scanned extension was once reported by
 * the dead-path detector as one finding on a line holding neither occurrence;
 * that spelling is now refused before resolution, so the symptom left before the
 * cause was established. Lifting the refusal and scanning this file again
 * returns both occurrences, each on its own line, so what fixed it was taking
 * the offset from the match rather than from the comment, and the refusal masks
 * nothing. The controls below are what makes that answer hold tomorrow.
 */

/** Two named citations, on two lines of one block comment. MUST report 4 and 5. */
const C_NAMED_TWICE = [
  "const a = 1;",
  "/*",
  " * nothing citable on this line",
  " * mirrors the guard at foo.ts:12",
  " * and the fallback at foo.ts:98",
  " */",
  "",
].join("\n");

/** Two bare line references, on two lines of one block comment. MUST report 4 and 5. */
const C_BARE_LINE_TWICE = [
  "const a = 1;",
  "/*",
  " * nothing citable on this line",
  " * the other constructor is at `:1183`",
  " * and its only caller is at `:204`",
  " */",
  "",
].join("\n");

/** Two spellings naming no file in the checkout. MUST report 4 and 5. */
const C_DEAD_PATH_TWICE = [
  "const a = 1;",
  "/*",
  " * nothing citable on this line",
  " * see `modules/ai/lib/httpProxy.ts` for the same reason",
  " * and `modules/ai/lib/httpStream.ts` for the other one",
  " */",
  "",
].join("\n");

/**
 * Two bare names each resolving to the citing file while another shares it. MUST
 * report 4 and 5.
 *
 * A Rust block comment, because that puts the offset arithmetic in the
 * hand-written scanner rather than in the parse, which is a second code path.
 */
const C_SELF_RESOLVED_TWICE = [
  "fn f() {}",
  "/*",
  " * nothing citable on this line",
  " * the child module is declared in `mod.rs`, not this one",
  " * and the sibling is declared in `mod.rs` too",
  " */",
  "",
].join("\n");

/**
 * Two bare names each shared by several files, cited from too far away to break
 * the tie. MUST report 4 and 5.
 *
 * The one bounded class, controlled for line attribution all the same. A ceiling
 * is not a licence to misreport: every site it counts is printed for a reader to
 * lower the bound by, and a printed site pointing at comment filler is worth
 * less than no site at all.
 */
const C_PARTIAL_PATH_TWICE = [
  "const a = 1;",
  "/*",
  " * nothing citable on this line",
  " * the root is created in `main.tsx` first",
  " * and the providers are mounted in `main.tsx` too",
  " */",
  "",
].join("\n");

export function runLineControls(): void {
  check(
    "two citations on two lines of one block comment report 4 and 5, not the opener's 2",
    linesOf("c.ts", C_BLOCK_TWO_LINES) === "4,5",
    linesOf("c.ts", C_BLOCK_TWO_LINES),
  );
  check(
    "a citation on a docblock's third line reports 4, not the opener's 2",
    linesOf("c.ts", C_DOCBLOCK_LINES) === "4",
    linesOf("c.ts", C_DOCBLOCK_LINES),
  );
  check(
    "a citation in a Rust block comment reports 4, not the opener's 2",
    linesOf("c.rs", C_RUST_BLOCK_LINES) === "4",
    linesOf("c.rs", C_RUST_BLOCK_LINES),
  );
  check(
    "a citation in a stylesheet's block comment reports 4, not the opener's 2",
    linesOf("c.css", C_CSS_BLOCK_LINES) === "4",
    linesOf("c.css", C_CSS_BLOCK_LINES),
  );

  // One per detector, each two occurrences of ONE kind on two lines of ONE
  // comment. The kind is asserted beside the lines, because a fixture that fired
  // the wrong detector would still answer 4 and 5 and the label would be a claim
  // nothing behind it holds.
  const twoOnTwoLines = (label: string, rel: string, src: string, kind: string): void =>
    check(
      `two ${label} on two lines of one comment report 4 and 5`,
      linesOf(rel, src) === "4,5" && kindsOf(rel, src) === `${kind},${kind}`,
      { lines: linesOf(rel, src), kinds: kindsOf(rel, src) },
    );

  twoOnTwoLines("named citations", "c.ts", C_NAMED_TWICE, "named");
  twoOnTwoLines("bare line references", "c.ts", C_BARE_LINE_TWICE, "bare-line");
  twoOnTwoLines("dead paths", "src/x.ts", C_DEAD_PATH_TWICE, "dead-path");
  twoOnTwoLines(
    "self-resolving bare names",
    "src-tauri/src/modules/fs/mod.rs",
    C_SELF_RESOLVED_TWICE,
    "self-resolved",
  );
  twoOnTwoLines(
    "partial spellings",
    "scripts/some-verify.ts",
    C_PARTIAL_PATH_TWICE,
    "partial-path",
  );
}
