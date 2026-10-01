/**
 * Controls for the two line detectors: every shape proved to fire, and every
 * near-miss proved not to.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { check } from "../report";

import { kindsOf } from "./support";

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
  check("a Rust comment sitting behind a lifetime is flagged", kindsOf("c.rs", C_RUST) === "named");
  check(
    "the identical text in a Rust string literal is NOT flagged",
    kindsOf("c.rs", C_RUST_IN_STRING) === "",
  );
}
