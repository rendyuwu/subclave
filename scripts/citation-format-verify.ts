/**
 * Self-check for the repository's comment-citation rule.
 * Run: `npx tsx scripts/citation-format-verify.ts`.
 *
 * `SUBCLAVE.md` carries the rule: a comment may cite only
 * what a reader holding nothing but the clone can open. A checked-in file, a
 * symbol, a path in the repo, an upstream project's public tracker named with
 * its project, or a pinned dependency's own source named with its crate. A LINE
 * NUMBER is none of those, and it is the shape that fails worst, because it is
 * correct only until the next commit touches the file it names and it never
 * announces that it stopped being correct.
 *
 * That class had been measured, corrected and re-measured more than once before
 * this file existed, and every correction rotted again from the next commit
 * onward. A structural check is the only version of the property that cannot
 * drift, so this replaces the correcting.
 *
 * Six shapes fail here:
 *
 *   1. A NAMED line citation, a file spelling followed by a colon and a line.
 *      No exemption, not even for a pinned dependency: a line into a
 *      dependency's source is no more openable from a clone than a line into
 *      this repository's own file is durable, and naming the crate makes such a
 *      citation attributable without making it reachable.
 *   2. A BARE line reference, a colon and a line leaning on a file named
 *      earlier in the same paragraph. It names no file at all, so a reader who
 *      starts mid-comment cannot even tell what it is relative to.
 *   3. A BARE tracker reference. This project's own tracker is not in the tree,
 *      so a number alone is not reachable. Naming the project is what makes one
 *      reachable, which is why the allow-list of upstream projects is pinned
 *      too.
 *   4. A MALFORMED dependency citation: a pinned crate opening a parenthesis
 *      that does not go on to name that crate's pinned version and a symbol, on
 *      one line. What a dependency citation buys by naming its crate is the
 *      right to cite a SYMBOL, and only at the version the lockfile pins.
 *   5. An UNCREDITED one: the shape of a dependency citation, a version and a
 *      symbol in parentheses, crediting no crate at all. This is what keeps the
 *      allow-list load-bearing now that shape 1 has no exemption. Without it a
 *      citation could name a version and a symbol while attributing them to
 *      nothing, and no other detector here would see it.
 *   6. A DEAD PATH: a backticked file spelling naming no file in the checkout.
 *      The first five are about a citation's SHAPE and all five pass one that is
 *      beautifully formed and points at nothing.
 *
 * WHY THE SIXTH IS THE ONE NO CARE CAN REPLACE. The first five guard a class
 * that rots when the CITED file changes, which is frequent and shallow. This one
 * guards the class that rots when the cited file is deleted or renamed, which is
 * rare, total, and invisible from the side that causes it. Nothing in the
 * toolchain connects deleting a module to the comments elsewhere that name it,
 * and the citing file may never be opened again: measured on this branch, one
 * commit deleted two modules and left six comments in a third file naming them,
 * and no commit since has touched that file. Authoring-time care cannot catch
 * that, because at authoring time the citation was right. Only a sweep over the
 * whole tree can.
 *
 * A seventh class is MEASURED BUT NOT FAILED: a partial spelling whose name is
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
 * WHY THE ALLOW-LISTS ARE PINNED AS EXACT SETS. An allow-list that can grow
 * without anybody noticing is not an allow-list. Every set here is asserted
 * sorted, deduplicated and at an exact length, so adding an entry is a visible
 * diff in this file, and every pinned crate version is read back out of
 * `src-tauri/Cargo.lock` so the allow-list cannot outlive the pin it describes.
 *
 * EVERY EXEMPTION IS ALSO ASSERTED STILL NECESSARY, which is the half such a
 * list usually lacks. An entry whose spelling somebody has since checked in
 * stops being an exemption and becomes a blindfold over a citation that has
 * become perfectly good, and nothing about the list's own shape would show it.
 * So each entry is re-resolved on every run and has to still fail.
 *
 * WHAT IT DOES NOT SEE, SAID OUT LOUD. The standing rule forbids more than
 * these six shapes, and the rest of it is not mechanically checkable from a
 * comment's text. A row id from one of this project's planning documents is a
 * violation of exactly the same rule and of exactly the same kind, since a
 * reader holding the clone cannot open it either, but it is prose: nothing
 * distinguishes such an id from an ordinary capitalised phrase. Also unseen: a
 * date, a commit hash, and a hand-test label. Green here therefore means
 * "carries none of the shapes below", never "obeys the citation rule", and a
 * reviewer still has to read.
 *
 * FOUR SHAPES THAT WOULD ESCAPE EVERY DETECTOR HERE, none of them in the tree,
 * listed so the paragraph is not read as exhaustive when it is merely complete
 * about what was found. A citation wrapped exactly at its extension's dot, so
 * that the stem ends one comment line and the extension opens the next, splits
 * the token across two ranges and matches neither the named nor the bare
 * pattern. A source-host line anchor, hash-L followed by a number, carries no
 * colon and no bare hash-digit. The same claim written as prose, "line 484 of
 * the session module", is a line citation with no punctuation to key on. And an
 * extension-less stem with a colon, a component name followed by a range, has
 * neither an extension for the named pattern nor a delimiter before the colon
 * for the bare one. The first two are cheap to add if one ever appears; the
 * third is prose and unreachable; the fourth would need the extension
 * requirement relaxed, which is what keeps host-and-port out.
 *
 * A BACKTICK IS WHAT MARKS A CITATION, so the resolvability half sees only
 * backticked spellings and a bare unquoted path is invisible to it. An attached
 * tracker reference is the same kind of gap: `word#123` passes for any word,
 * which is what lets `PKCS#8` through, so a bare number can be laundered by
 * attaching anything to it. That is this
 * repository's own convention rather than a shortcut, and it is why prose in
 * this file that mentions a file name as an EXAMPLE leaves the backticks off.
 * It does mean the sixth shape can be evaded by dropping them.
 *
 * A DELIBERATELY DEAD NAME IS LEGITIMATE, AND NOT DETECTABLE. A comment may
 * name a file precisely in order to say it is gone, and then the dead name is
 * the point: the sentence is false if the file still exists, and deleting the
 * name leaves the paragraph with no subject. The same dead spelling cited in the
 * present tense, as a live mechanism, is simply wrong. Both were in this tree at
 * once, and the difference between them is TENSE AND GRAMMATICAL SUBJECT, which
 * is a property of meaning and not of text. No detector reading a comment can
 * tell them apart, and a heuristic over surrounding words would only encode a
 * guess about English. So `DELIBERATELY_DEAD` is a human judgement recorded as
 * a pinned entry rather than a rule, keyed on the citing file so it cannot
 * travel to a file that cites the same dead name as though it were alive. It is
 * the only place in this check where that is true, and if it ever grows past a
 * couple of entries the honest reading is that the class needs a convention in
 * the prose, not a longer list here.
 *
 * A SYMBOL IS NOT CHECKED, and this was measured rather than assumed. The only
 * test available without a type checker is whether the token appears somewhere
 * in the tree, which is too weak to establish that a symbol exists (a name
 * surviving in a string literal passes) and too strong to be usable: 131 of the
 * 2,342 distinct backticked identifiers in this tree's comments appear nowhere
 * in its comment-stripped code, and almost every one is legitimate. They name
 * Win32 entry points, a dependency's internals, TypeScript compiler node kinds,
 * DOM events, and external tools. A resolvability rule would redden all 131 of
 * them, almost every one for being right, and a symbol can legitimately live in
 * a dependency in any case. Out of scope, deliberately.
 *
 * NO INSTANCE OF THAT CLASS IS NAMED HERE, and that is the rule rather than a
 * gap. This paragraph used to name `secrets_list` as its decisive case - a
 * command the secrets IPC surface deliberately did not expose, cited in 26
 * comments precisely because it did not exist - and two checks at the end of
 * this file asserted both halves of that arithmetic, which is what made naming
 * it safe. That command has since been implemented, those two checks were
 * retired with it, and the paragraph lost its example. It did not lose its
 * point: a prose claim about the state of the tree is exactly as durable as the
 * assertion standing behind it, and with no assertion it is a line number by
 * another name. So the class is described and no instance is named, which is the
 * same treatment the row-id class above gets, for the same reason.
 *
 * An earlier draft of that row-id paragraph did name one such row id and the
 * file holding it, which was true when written and false within the hour,
 * because the id was deleted. A comment asserting a state of the tree rots on the
 * next commit for the same reason a line number does, and the shape of the
 * mistake does not change just because the subject is a defect rather than a
 * location. Name a fact only when something announces its change; describe the
 * class when nothing does.
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

import { runAllowListChecks } from "./lib/citation/allowlist";
import { runExemptionControls } from "./lib/citation/controls/exemptions";
import { runLineControls } from "./lib/citation/controls/lines";
import { runReferentControls } from "./lib/citation/controls/referent";
import { runShapeControls } from "./lib/citation/controls/shapes";
import { check, failed } from "./lib/citation/report";
import { runResolvabilityChecks } from "./lib/citation/resolve";
import { runTreeChecks } from "./lib/citation/tree";

console.log("[allow-list] pinned, sorted, and still true of the lockfile");
runAllowListChecks();

console.log("\n[resolvability] the file universe, and every exemption still necessary");
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

console.log("\n[tree] no comment cites a line number, a bare tracker number or a dead path");
runTreeChecks();

console.log(failed === 0 ? "\nAll citation-format checks passed." : `\n${failed} check(s) FAILED.`);
process.exit(failed === 0 ? 0 : 1);
