/**
 * Controls for the exemptions: each proved to fire, and each proved not to
 * travel beyond the spelling it excuses.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { check } from "../report";
import { notAFileSpelling } from "../resolve";

import { C_PARTIAL_PATH } from "./referent";
import { kindsOf } from "./support";

export function runExemptionControls(): void {
  check(
    "a runtime store-file name is exempt",
    kindsOf("src/x.ts", "const a = 1; // written to `subclave-settings.json`\n") === "",
  );
  check(
    "the same stem at another extension is NOT exempt, so the pattern is not a prefix",
    kindsOf("src/x.ts", "const a = 1; // written to `subclave-settings.ts`\n") === "dead-path",
  );
  check(
    "an external tool's config name is exempt",
    kindsOf("src/x.ts", "const a = 1; // reads `prettier.config.js` first\n") === "",
  );
  check(
    "this repository's own checked-in config resolves normally rather than by exemption",
    kindsOf("src/x.ts", "const a = 1; // reads `.prettierrc.json` first\n") === "",
  );
  // An exemption must never be able to silence an AMBIGUITY, which is a finding
  // about a file that does exist. Proved by ordering: the runtime pattern is
  // consulted only after resolution misses.
  check(
    "an exemption cannot suppress a partial spelling",
    kindsOf("scripts/some-verify.ts", C_PARTIAL_PATH) === "partial-path",
  );
  // The discriminator itself, asserted by REASON and not by outcome. An
  // extension-less import spelling must be refused BEFORE resolution, or the
  // resolution step could quietly land it on a plausible file.
  check(
    "an extension-less import spelling is refused for having no extension",
    notAFileSpelling("../store") === "no-extension" &&
      notAFileSpelling("@/modules/vault/store") === "no-extension",
    { relative: notAFileSpelling("../store"), alias: notAFileSpelling("@/modules/vault/store") },
  );
  check(
    "a real relative citation, which carries one, is NOT refused",
    notAFileSpelling("./store.ts") === null,
    notAFileSpelling("./store.ts"),
  );
  check(
    "an elided dependency path is refused for being elided, so no npm rule is needed",
    notAFileSpelling("react-remove-scroll/dist/.../SideEffect.js") === "elided",
    notAFileSpelling("react-remove-scroll/dist/.../SideEffect.js"),
  );
  // A project name that happens to end in an extension, against a file spelling
  // that does not. Same shape, two answers, decided by the tracker register.
  check(
    "a backticked upstream project name is NOT read as a file",
    kindsOf("src/x.ts", "const a = 1; // a known bug in `xterm.js` dims the glyphs\n") === "",
  );
  check(
    "a lookalike that is not a pinned project IS read as a file, and reddens",
    kindsOf("src/x.ts", "const a = 1; // a known bug in `xtermm.js` dims the glyphs\n") ===
      "dead-path",
  );
  // The silently-wrong unique resolution, using the shape that actually occurred.
  // The pair is decided by whether the author wrote the path out: a bare name lets
  // nearness prefer the citing file automatically, a full path does not.
  check(
    "a bare name that resolves to the citing file, while others share it, IS flagged",
    kindsOf("src/settings/main.tsx", "// the entry point is `main.tsx`, not this one\n") ===
      "self-resolved",
  );
  check(
    "the same claim with the path written out is NOT flagged",
    kindsOf("src/settings/main.tsx", "// the entry point is `src/main.tsx`, not this one\n") === "",
  );
  // And a bare name that resolves to the citing file when NOTHING else shares it
  // is a redundant self-reference rather than a misdirection, so it passes.
  check(
    "a bare self-reference with no other file of that name is NOT flagged",
    kindsOf("src/lib/storeRecovery.ts", "// as `storeRecovery.ts` does above\n") === "",
  );
}
