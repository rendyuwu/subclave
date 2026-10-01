/**
 * The five shape detectors: a named line citation, a bare line reference, a
 * bare tracker number, a malformed dependency citation and an uncredited one,
 * plus the allow-list patterns they are built from.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { THIRD_PARTY_SOURCES, UPSTREAM_TRACKERS } from "./allowlist";

const escapeRe = (s: string): string => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * A file spelling followed by a colon and one or more line numbers.
 *
 * NO EXEMPTION, not even for a pinned dependency. A line into a dependency's
 * source is unverifiable whether or not the crate is named: no clone checks that
 * source out and nothing in CI can open it, so naming the crate makes the
 * citation attributable without making it reachable. Naming a crate buys the
 * right to cite its SYMBOL, which is what the allow-list below is for; it does
 * not buy a line. An earlier draft of this file exempted a crate-qualified line
 * and that was the one hole through which the shape this check exists to remove
 * could come back green.
 *
 * The extension list is closed rather than open (`\w+` after the dot) because
 * an open one matches a sentence's `word.Another:2` and, worse, matches an
 * enum-ish `Foo.Bar:1`. Closed to the extensions this repository actually has,
 * plus the two lockfile spellings, since those get cited too.
 *
 * The optional quote before the colon is not cosmetic. A citation written as
 * a backticked path followed by an unbacked range puts a backtick between the
 * extension and the colon, and a pattern demanding adjacency reads that as a
 * bare reference and reports it without the file name. Measured: zero comments
 * in the three roots put a quote-then-colon-then-digit anywhere else, so
 * allowing it costs no false positive and buys a citation the generated
 * inventory could not see at all.
 *
 * The trailing `[-,]` group is what makes a multi-line and a multi-target
 * citation one match rather than several: a range and a comma-separated list are
 * both a single rotted reference, and counting them apart would inflate a
 * failure report without adding a fact.
 */
export const NAMED_CITATION =
  /[A-Za-z0-9_@./-]*[A-Za-z0-9_-]\.(?:tsx?|mts|cts|jsx?|mjs|cjs|rs|css|html|json|toml|md|lock|ya?ml)[`'"]?\s*:\s*\d+(?:\s*[-,]\s*\d+)*/g;

/**
 * A colon and a line number with nothing in front of it.
 *
 * The lookbehind is the whole design. Requiring a backtick, an opening bracket
 * or whitespace before the colon is what separates a citation leaning on a file
 * named a paragraph earlier from the two shapes that are not citations at all:
 * an authority in a URL or an address, and a JSON key in a quoted fixture, both
 * of which put a word character or a quote immediately before the colon. This
 * is a remote-desktop and SSH client, so a comment illustrating a forward with
 * a host and a port is ordinary prose here and appears across the forwards and
 * terminal modules; a scan without the lookbehind is unusable rather than
 * merely noisy. `C_HOST_PORT` and `C_NOT_A_CITATION` hold that line.
 */
export const BARE_LINE = /(?<=[`([\s])(?::\s*\d+(?:\s*[-,]\s*\d+)*)(?![\d.\w])/g;

/**
 * A tracker number, permitted only when a pinned project is named beside it.
 *
 * Two exclusions, both measured against what this tree's comments actually
 * carry rather than guessed:
 *
 *   - `(?<![\w#])` lets a number ATTACHED to a word through, which covers both
 *     `plugins-workspace#3085` (a project naming its own tracker, permitted)
 *     and the several `PKCS#8` / `PKCS#1` mentions, which are format names and
 *     not citations at all.
 *   - a leading `0` and a trailing hex digit are both refused, which is what
 *     keeps the theme presets and the stylesheet out of it: a six-digit colour
 *     cannot survive either test, and no tracker numbers its issues from zero.
 *     A six-digit issue number would be missed, which fails towards silence on
 *     a shape this repository does not contain.
 */
export const BARE_TRACKER = /(?<![\w#])#[1-9]\d{0,4}(?![\dA-Fa-f])/g;

/**
 * A backticked pinned crate opening a parenthesis, which is citation position.
 *
 * Scoped to `(` deliberately. Naming a dependency in prose is not citing it:
 * a comment legitimately reads "after `tauri-plugin-window-state` has ..." with
 * no symbol in sight, and demanding a version there would redden ordinary
 * English. The one dependency citation in the tree opens a parenthesis, so the
 * parenthesis is what distinguishes the two.
 */
export const DEP_OPENER = new RegExp(
  `\\(\`(?:${THIRD_PARTY_SOURCES.map((e) => escapeRe(e.split(" ")[0])).join("|")})\``,
  "g",
);

/**
 * The one well-formed shape, per pinned crate: crate, its exact pinned version,
 * then a backticked symbol, all inside one set of parentheses.
 *
 * `[ \t]` and never `\s` in the gaps, which is how the ONE-LINE requirement is
 * enforced rather than merely hoped for. A triple split across two comment lines
 * is malformed and reddens as `dep-form`. Two mechanisms, and the requirement
 * BINDS BOTH LANGUAGES rather than only Rust, which an earlier draft of this
 * paragraph got wrong: every line-comment extractor here emits one range per
 * line, the TypeScript one as much as the Rust one, so the half after the break
 * is not in the same string this pattern sees whichever language it is written
 * in. Inside a block comment the halves ARE in one string, and there the newline
 * plus continuation marker fails the gap. The one triple in the tree is in a
 * `.rs` file, so the TypeScript half has no instance, and a reader who took the
 * old wording for a Rust quirk would have written a wrapped triple in a `.ts`
 * comment and been just as invisible. Neither silently accepted nor silently
 * rejected, which was the choice to make: it is reported, with the crate named,
 * so the fix is obvious.
 *
 * The symbol is `[^`\n]+` rather than an identifier pattern because the real
 * ones include qualified and generic spellings that no identifier pattern
 * survives, for instance a trait-qualified method on a generic type.
 */
export const DEP_WELL_FORMED = THIRD_PARTY_SOURCES.map((entry) => {
  const [crate, version] = entry.split(" ");
  return new RegExp(
    `^\\(\`${escapeRe(crate)}\`[ \\t]+${escapeRe(version)},[ \\t]+\`[^\`\\n]+\`\\)`,
  );
});

/**
 * A parenthesised group carrying a version and a backticked symbol: the shape of
 * a dependency citation, whoever it credits.
 *
 * This is what keeps the allow-list load-bearing after the exemption above was
 * removed. Without it, a citation that drops the crate name has nothing left for
 * any detector to catch: it holds no line number, so the file-and-line patterns
 * are silent, and it names no pinned crate, so `DEP_OPENER` is silent too. The
 * form would then be free to name a version and a symbol while crediting
 * nothing, which is precisely the unattributable citation the allow-list exists
 * to forbid. Measured over the tree: this matches the one real triple and
 * nothing else, and that triple credits a pinned crate.
 */
export const DEP_SHAPE = /\([^()\n]{0,80}?\d+\.\d+\.\d+[ \t]*,[ \t]*`[^`\n]+`[^()\n]{0,12}\)/g;

/**
 * How far back a tracker number may look for the project that qualifies it:
 * ANYWHERE EARLIER IN THE SAME COMMENT, and not a character count.
 *
 * This was a 90-character window and that was a latent false positive with no
 * good fix. A docblock names `xterm.js` at the end of one line and carries
 * `#4054` at the start of the next; an edit that pushed the project name
 * past the count would have reddened a correct citation, and the only remedy
 * available to whoever hit it is widening the number, which weakens the
 * detector for every real case. A check that reddens on correct code is worse
 * than no check, because the first contributor to hit one weakens it.
 *
 * The comment is the non-arbitrary bound, and it is the reader's bound too: a
 * reader meeting a bare number scans back for the nearest project name and
 * finds it if it is in the same comment, so that is precisely the span over
 * which the number is attributable. It costs one thing, which is worth stating
 * rather than hiding: a project named in a long docblock's first paragraph
 * qualifies a bare number in its last. That fails towards accepting a citation
 * a reader could still resolve, where a character count fails towards refusing
 * one that is already correct.
 *
 * For a line comment each `//` line is its own range, so a project name on the
 * previous line is out of reach whatever the bound. No citation in the tree is
 * written that way, and the fix if one appears is to join a run of adjacent line
 * comments rather than to reintroduce a count.
 */

/** A pinned upstream project named beside a tracker number. */
export const TRACKER_PREFIX = new RegExp(UPSTREAM_TRACKERS.map(escapeRe).join("|"));

/** Any backticked pinned crate name, for telling `dep-form` from `dep-uncredited`. */
export const ANY_PINNED_CRATE = new RegExp(
  `\`(?:${THIRD_PARTY_SOURCES.map((e) => escapeRe(e.split(" ")[0])).join("|")})\``,
);
