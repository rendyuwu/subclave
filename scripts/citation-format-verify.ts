/**
 * Self-check for the repository's comment-citation rule.
 * Run: `npx tsx scripts/citation-format-verify.ts`.
 *
 * `SUBCLAVE.md` carries the rule: a comment may cite only what a reader holding
 * nothing but the clone can open. A checked-in file, a symbol, this repository's
 * own source. A LINE NUMBER is none of those, and it is the shape that fails
 * worst, because it is correct only until the next commit touches the file it
 * names and it never announces that it stopped being correct.
 *
 * That class had been measured, corrected and re-measured more than once before
 * this file existed, and every correction rotted again from the next commit
 * onward. A structural check is the only version of the property that cannot
 * drift, so this replaces the correcting.
 *
 * Two shapes fail here:
 *
 *   1. A NAMED line citation, a file spelling followed by a colon and a line.
 *   2. A BARE line reference, a colon and a line leaning on a file named
 *      earlier in the same paragraph. It names no file at all, so a reader who
 *      starts mid-comment cannot even tell what it is relative to.
 *
 * And a third: a DEAD PATH, a backticked file spelling naming no file in the
 * checkout. The two above are about a citation's SHAPE and both pass one that is
 * beautifully formed and points at nothing.
 *
 * WHY THE DEAD PATH IS THE ONE NO CARE CAN REPLACE. The two shape checks guard a
 * class that rots when the CITED file changes, which is frequent and shallow.
 * This one guards the class that rots when the cited file is deleted or renamed,
 * which is rare, total, and invisible from the side that causes it. Nothing in
 * the toolchain connects deleting a module to the comments elsewhere that name
 * it, and the citing file may never be opened again: measured on this branch,
 * one commit deleted two modules and left six comments in a third file naming
 * them, and no commit since has touched that file. Authoring-time care cannot
 * catch that, because at authoring time the citation was right. Only a sweep over
 * the whole tree can.
 *
 * A further class is MEASURED BUT NOT FAILED: a partial spelling whose name is
 * shared by several files and whose citing directory does not break the tie. It
 * is capped rather than forbidden. See `PARTIAL_PATH_CEILING`.
 *
 * WHY IT READS COMMENTS AND NOT SOURCE TEXT. Half this suite legitimately
 * carries a file spelling and a line inside a STRING literal: the fixtures quote
 * store payloads, error strings and JSON, and several scripts assert over text
 * that contains exactly the shape above. A scan over raw source would redden
 * those, which is not a check anybody could keep. `lib/comments.ts` extracts the
 * comments out of the parse instead, and its own two-direction self-test runs
 * first below, because an extractor that silently returns nothing would turn
 * every check here green.
 *
 * WHAT IT DOES NOT SEE, SAID OUT LOUD. The standing rule forbids more than
 * these three shapes, and the rest of it is not mechanically checkable from a
 * comment's text. A row id from one of this project's planning documents is a
 * violation of exactly the same kind, since a reader holding the clone cannot
 * open it either, but it is prose: nothing distinguishes such an id from an
 * ordinary capitalised phrase. Also unseen: a date, a commit hash, a tracker
 * number, and a hand-test label. Green here therefore means "carries none of the
 * shapes below", never "obeys the citation rule", and a reviewer still has to
 * read.
 *
 * A BACKTICK IS WHAT MARKS A CITATION, so the resolvability half sees only
 * backticked spellings and a bare unquoted path is invisible to it. That is this
 * repository's own convention rather than a shortcut, and it is why prose in
 * this file that mentions a file name as an EXAMPLE leaves the backticks off.
 *
 * A DELIBERATELY DEAD NAME IS LEGITIMATE, AND NOT DETECTABLE. A comment may
 * name a file precisely in order to say it is gone, and then the dead name is
 * the point: the sentence is false if the file still exists, and deleting the
 * name leaves the paragraph with no subject. The same dead spelling cited in the
 * present tense, as a live mechanism, is simply wrong. The difference between
 * them is TENSE AND GRAMMATICAL SUBJECT, which is a property of meaning and not
 * of text. No detector reading a comment can tell them apart, and a heuristic
 * over surrounding words would only encode a guess about English. So
 * `DELIBERATELY_DEAD` is a human judgement recorded as a pinned entry rather
 * than a rule, keyed on the citing file so it cannot travel to a file that cites
 * the same dead name as though it were alive.
 *
 * A SYMBOL IS NOT CHECKED, and this was measured rather than assumed. The only
 * test available without a type checker is whether the token appears somewhere
 * in the tree, which is too weak to establish that a symbol exists (a name
 * surviving in a string literal passes) and too strong to be usable: 131 of the
 * 2,342 distinct backticked identifiers in this tree's comments appear nowhere
 * in its comment-stripped code, and almost every one is legitimate. Out of
 * scope, deliberately.
 *
 * WHY THE CONTROLS ARE HERE AND NOT IN A NOTE SOMEWHERE. Both halves of this
 * check can fail silently: a detector that matches nothing passes the tree, and
 * a detector that matches a string literal reddens a file nobody can fix. The
 * `[controls]` section runs every shape against a fixture that must be flagged
 * AND a fixture that must not, so neither direction can rot unobserved. The
 * fixtures live in string literals under `scripts/lib/citation/`, which keeps
 * them inside the tree the scan below covers, so it reads them too and must not
 * flag a single one of them.
 */

import { commentScannerSelfTest } from "./lib/comments";

import { runExemptionControls } from "./lib/citation/controls/exemptions";
import { runLineControls } from "./lib/citation/controls/lines";
import { runReferentControls } from "./lib/citation/controls/referent";
import { runShapeControls } from "./lib/citation/controls/shapes";
import { check, failed } from "./lib/citation/report";
import { runResolvabilityChecks } from "./lib/citation/resolve";
import { runTreeChecks } from "./lib/citation/tree";

console.log("[resolvability] the file universe, and every exemption still necessary");
runResolvabilityChecks();

console.log("\n[extractor] comments come out of the parse, in both directions");
for (const { label, ok } of commentScannerSelfTest()) check(label, ok);

console.log("\n[controls] each detector fires, and each declines to");
runShapeControls();

console.log("\n[referent] a cited file has to be a file that exists");
runReferentControls();

console.log("\n[exemptions] each one fires, and none of them travels");
runExemptionControls();

console.log("\n[lines] a finding names the citation's line, not the comment's");
runLineControls();

console.log("\n[tree] no comment cites a line number or a dead path");
runTreeChecks();

console.log(failed === 0 ? "\nAll citation-format checks passed." : `\n${failed} check(s) FAILED.`);
process.exit(failed === 0 ? 0 : 1);
