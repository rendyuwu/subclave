/**
 * The two line-citation detectors: a named `file:line`, and a bare line
 * reference leaning on a file named earlier in the same comment.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */

/**
 * A file spelling followed by a colon and one or more line numbers.
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
