/**
 * Controls for the referent half of the rule: a cited file has to be a file
 * that exists, and a spelling resolves against the citing file's own directory.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { check } from "../report";

import { kindsOf } from "./support";

/** A citation of a file that is not in the checkout. MUST be flagged. */
const C_DEAD_PATH = "const a = 1; // see `modules/ai/lib/httpProxy.ts` for the same reason\n";
/** The same spelling in a string literal. MUST NOT be flagged. */
const C_DEAD_PATH_IN_STRING = 'const a = "see modules/ai/lib/httpProxy.ts";\n';
/** A citation of a file that IS in the checkout. MUST NOT be flagged. */
const C_LIVE_PATH = "const a = 1; // `src/lib/storeRecovery.ts` classifies the failure\n";
/**
 * A bare file name resolved by the citing file's own directory. MUST NOT be flagged.
 *
 * Two files share that name in this tree, so a resolver ignoring where the
 * citation sits would call this ambiguous. Cited from the settings window's own
 * directory it means the settings one, which is how every reader takes it.
 */
const C_NEAREST_PATH = "const a = 1; // the root is created in `main.tsx` first\n";
/**
 * The identical spelling with nothing nearby to disambiguate it. MUST be
 * flagged as partial.
 *
 * The SAME text as the control above, from a citing path in another directory.
 * That is what makes the pair a test of the resolution rule rather than of the
 * spelling: one string, two answers, and only a resolver that reads the citing
 * file's directory can produce both.
 */
export const C_PARTIAL_PATH = C_NEAREST_PATH;
/** A relative spelling, which resolves against the citing file's directory. MUST NOT be flagged. */
const C_RELATIVE_PATH = "const a = 1; // mirrors `./preferences.ts` exactly\n";
/** A relative spelling that walks off the tree. MUST be flagged. */
const C_RELATIVE_DEAD = "const a = 1; // mirrors `./nothingHere.ts` exactly\n";
/** An import spelling quoted as an example of a class, not as a citation. MUST NOT be flagged. */
const C_IMPORT_EXAMPLE =
  "const a = 1; // no `../store`, no `../../vault/store`, no `@/modules/vault/store`\n";
/** This suite naming its own members with a glob. MUST NOT be flagged. */
const C_GLOB = "const a = 1; // every `scripts/*-verify.ts` runs in one pass\n";
/** A bare extension in prose, and a Windows path used as example input. MUST NOT be flagged. */
const C_NOT_A_FILE = "const a = 1; // a `.tsx` file, or `C:\\a\\b.md` on Windows\n";

export function runReferentControls(): void {
  check(
    "a citation of a file not in the checkout is flagged",
    kindsOf("src/x.ts", C_DEAD_PATH) === "dead-path",
  );
  check(
    "the identical spelling in a string literal is NOT flagged",
    kindsOf("src/x.ts", C_DEAD_PATH_IN_STRING) === "",
  );
  check(
    "a citation of a file that IS in the checkout is NOT flagged",
    kindsOf("src/x.ts", C_LIVE_PATH) === "",
  );
  // The pair that tests the RULE and not the spelling: identical text, two
  // answers, decided only by where the citation sits.
  check(
    "a bare `main.tsx` cited from the settings directory is NOT flagged",
    kindsOf("src/settings/SettingsApp.tsx", C_NEAREST_PATH) === "",
  );
  check(
    "the same spelling cited from a script, with nothing nearby, is flagged as partial",
    kindsOf("scripts/some-verify.ts", C_PARTIAL_PATH) === "partial-path",
  );
  check(
    "a relative spelling resolves against the citing file's own directory",
    kindsOf("src/modules/settings/customTheme.ts", C_RELATIVE_PATH) === "",
  );
  check(
    "a relative spelling that names nothing there is flagged",
    kindsOf("src/modules/settings/customTheme.ts", C_RELATIVE_DEAD) === "dead-path",
  );
  check(
    "import spellings quoted as examples of a class are NOT flagged",
    kindsOf("scripts/some-verify.ts", C_IMPORT_EXAMPLE) === "",
  );
  check(
    "a glob naming this suite's members is NOT flagged",
    kindsOf("scripts/s.ts", C_GLOB) === "",
  );
  check(
    "a bare extension and a Windows example path are NOT flagged",
    kindsOf("src/x.ts", C_NOT_A_FILE) === "",
  );

  // The exemptions, each proved to fire AND proved not to travel. A pair per
  // exemption, because "it is green" alone is satisfied by an exemption that
}
