/**
 * Controls for the five shape detectors: every shape proved to fire, and every
 * near-miss proved not to.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { check } from "../report";

import { kindsOf, SYM } from "./support";

/** A named citation in a comment. MUST be flagged. */
const C_NAMED = "const a = 1; // mirrors the guard at foo.ts:12\n";
/** The identical text in a string literal. MUST NOT be flagged. */
const C_NAMED_IN_STRING = 'const a = "mirrors the guard at foo.ts:12";\n';
/**
 * A citation whose colon sits OUTSIDE the closing backtick. MUST be flagged, and
 * as `named` rather than as a bare reference, because the file is right there.
 *
 * The generated inventory that sized this work could not see this shape at all,
 * which is the reason it is a control: a second instrument that agrees with the
 * first by construction proves nothing about what the first one missed.
 */
const C_NAMED_OUTSIDE_BACKTICK =
  "const a = 1; // awaited before `src/lib/storeRecovery.ts`:12 runs\n";
/** A bare span in parentheses, the other shape the inventory missed. MUST be flagged. */
const C_BARE_SPAN = "const a = 1; // the direction is explained at (:112-114)\n";
/**
 * A dependency citation that still carries a line. MUST be flagged.
 *
 * The crate and the pinned version are both correct and it is still refused: a
 * line into a dependency's source is not reachable from a clone, so crediting it
 * makes it attributable without making it openable.
 */
const C_DEP_WITH_LINE = "const a = 1; // (`tauri` 2.11.5, `src/ipc/channel.rs:39`)\n";
/** The form the conversion lands on: crate, pinned version, symbol, no line. MUST NOT be flagged. */
const C_DEP_SYMBOL = `const a = 1; // (\`tauri\` 2.11.5, \`${SYM}\`)\n`;
/** The same with the crate name removed, and no line to redden it instead. MUST be flagged. */
const C_DEP_NO_CRATE = `const a = 1; // (2.11.5, \`${SYM}\`)\n`;
/** The same with the crate named but no version. MUST be flagged. */
const C_DEP_NO_VERSION = `const a = 1; // (\`tauri\`, \`${SYM}\`)\n`;
/**
 * The same at a version this repository does not pin. MUST be flagged.
 *
 * 2.4.1 is a real pinned version in this repository, of the OTHER crate on the
 * allow-list, so this also proves the version is checked against its own crate
 * rather than against the set of versions in use.
 */
const C_DEP_BAD_VERSION = `const a = 1; // (\`tauri\` 2.4.1, \`${SYM}\`)\n`;
/** A triple wrapped across two comment lines. MUST be flagged as malformed. */
const C_DEP_WRAPPED = `fn f() {} // (\`tauri\` 2.11.5,\n// \`${SYM}\`)\n`;
/** A dependency named in prose, citing no symbol. MUST NOT be flagged. */
const C_DEP_IN_PROSE =
  "const a = 1; // re-apply the floor after `tauri-plugin-window-state` has restored\n";
/** A bare line reference in a comment. MUST be flagged. */
const C_BARE = "const a = 1; // the other constructor is at `:1183`\n";
/** The identical text in a string literal. MUST NOT be flagged. */
const C_BARE_IN_STRING = 'const a = "the other constructor is at `:1183`";\n';
/** An address and a JSON key, neither of which is a citation. MUST NOT be flagged. */
const C_NOT_A_CITATION = 'const a = 1; // binds 127.0.0.1:5432 and logs `{"count":0}`\n';
/**
 * A forwarding route, which is illustrative prose. MUST NOT be flagged.
 *
 * The text is the one a page component carries to size its widest row. Three
 * host-and-port pairs, one of them a bare name and two of them addresses, in a
 * repository whose whole subject is forwarding ports between them.
 */
const C_HOST_PORT =
  "const a = 1; // the longest row is `localhost:18084 → bastion → 10.0.0.9:5432`\n";
/** A bare tracker number. MUST be flagged. */
const C_TRACKER = "const a = 1; // pinned above other apps (#33)\n";
/** A tracker number naming its project. MUST NOT be flagged. */
const C_TRACKER_NAMED = "const a = 1; // a known renderer bug (xterm.js #4054)\n";
/** A colour and a format name. MUST NOT be flagged. */
const C_TRACKER_LOOKALIKE = "const a = 1; // base0 (#839496) under an unencrypted PKCS#8\n";
/** A Rust comment behind a lifetime, which must not swallow the line. MUST be flagged. */
const C_RUST = "fn f<'a>(x: &'a str) -> &'a str { x } // see the tail at session.rs:484\n";
/** The identical text in a Rust string. MUST NOT be flagged. */
const C_RUST_IN_STRING = 'fn f() { let s = "see the tail at session.rs:484"; }\n';

export function runShapeControls(): void {
  check("a named citation in a comment is flagged", kindsOf("c.ts", C_NAMED) === "named");
  check(
    "the identical text in a string literal is NOT flagged",
    kindsOf("c.ts", C_NAMED_IN_STRING) === "",
  );
  check(
    "a citation whose colon sits outside the backtick is flagged, as named",
    kindsOf("c.ts", C_NAMED_OUTSIDE_BACKTICK) === "named",
  );
  check("a bare span in parentheses is flagged", kindsOf("c.ts", C_BARE_SPAN) === "bare-line");
  check(
    "a dependency citation still carrying a line IS flagged, crate and version notwithstanding",
    kindsOf("c.ts", C_DEP_WITH_LINE) === "named",
  );
  check(
    "the crate, version and symbol form the conversion lands on is NOT flagged",
    kindsOf("c.ts", C_DEP_SYMBOL) === "",
  );
  check(
    "the same form with the crate name removed IS flagged",
    kindsOf("c.ts", C_DEP_NO_CRATE) === "dep-uncredited",
  );
  check(
    "the same form with no version IS flagged",
    kindsOf("c.ts", C_DEP_NO_VERSION) === "dep-form",
  );
  check(
    "the same form at a version this repository does not pin IS flagged",
    kindsOf("c.ts", C_DEP_BAD_VERSION) === "dep-form",
  );
  check(
    "a triple wrapped across two comment lines IS flagged as malformed",
    kindsOf("c.rs", C_DEP_WRAPPED) === "dep-form",
  );
  check("a dependency named in prose is NOT flagged", kindsOf("c.ts", C_DEP_IN_PROSE) === "");
  check("a bare line reference is flagged", kindsOf("c.ts", C_BARE) === "bare-line");
  check(
    "the identical bare reference in a string literal is NOT flagged",
    kindsOf("c.ts", C_BARE_IN_STRING) === "",
  );
  check(
    "an address and a JSON key in a comment are NOT flagged",
    kindsOf("c.ts", C_NOT_A_CITATION) === "",
  );
  check("a host:port forwarding route is NOT flagged", kindsOf("c.ts", C_HOST_PORT) === "");
  check("a bare tracker number is flagged", kindsOf("c.ts", C_TRACKER) === "bare-tracker");
  check(
    "a tracker number naming its project is NOT flagged",
    kindsOf("c.ts", C_TRACKER_NAMED) === "",
  );
  check(
    "a six-digit colour and an attached format name are NOT flagged",
    kindsOf("c.ts", C_TRACKER_LOOKALIKE) === "",
  );
  check("a Rust comment sitting behind a lifetime is flagged", kindsOf("c.rs", C_RUST) === "named");
  check(
    "the identical text in a Rust string literal is NOT flagged",
    kindsOf("c.rs", C_RUST_IN_STRING) === "",
  );
}
