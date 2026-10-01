/**
 * Resolvability: does a cited file exist? The file universe walked from the
 * checkout, the spellings that are not file spellings at all, the exemptions,
 * and the resolver that answers what a spelling could mean.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { readdirSync, statSync } from "node:fs";
import { dirname, join, normalize, relative } from "node:path";

import { UPSTREAM_TRACKERS } from "./allowlist";
import { check, ROOT, sortedSet } from "./report";

/**
 * Every file a clone would have, by walking the checkout.
 *
 * Not `git ls-files`, for two reasons. Nothing else in this suite spawns a
 * child process, and a check that needs git present answers differently in a
 * shallow or exported checkout than in a working one. More importantly a walk
 * SEES A FILE ADDED BUT NOT YET COMMITTED, which is the state every branch is
 * in while it is being written: resolving against the index alone would redden
 * a citation of a file added in the same commit, which is the commonest
 * legitimate new citation there is.
 *
 * The skipped directories are NAMED rather than matched by a leading dot, and
 * the difference is load-bearing. A blanket dot rule looks tidy and drops
 * `.github/` with its workflows and shell scripts, and every checked-in dotfile
 * with them, so a citation of any of those reads as dead. Measured while
 * building this: that rule alone produced six findings against a checked-in
 * formatter config. The two dotted directories skipped here are skipped because
 * they hold no source the rule governs and a citation into either is forbidden
 * by the rule anyway.
 */
const SKIP_DIRS = new Set([
  ".claude",
  ".git",
  ".omc",
  "build",
  "coverage",
  "dist",
  "node_modules",
  "target",
]);
function everyFile(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) {
      if (!SKIP_DIRS.has(name)) everyFile(full, out);
      continue;
    }
    out.push(relative(ROOT, full).split("\\").join("/"));
  }
  return out;
}
const REPO_FILES = everyFile(ROOT);
export const REPO_FILE_SET = new Set(REPO_FILES);

/** The extensions a backticked spelling must end in to be read as a file at all. */
const FILE_EXT = /\.(?:tsx?|mts|cts|jsx?|mjs|cjs|rs|css|html|json|toml|md|lock|ya?ml|sh|ps1)$/;

/**
 * Why `s` is not a spelling of a file in this repository, or `null`.
 *
 * EVERY ONE OF THESE WAS MEASURED, not guessed, and the counts are what justify
 * them. Over the three roots there are about 12,900 backticked tokens in
 * comments that carry no file extension and 1,144 that do, so the extension is
 * doing almost all of the work and it has to be right.
 *
 *   `no-extension`   about 12,900 tokens, and the reason this detector requires
 *                    one. Five verify scripts quote relative import spellings
 *                    as EXAMPLES of a class rather than as citations, because
 *                    the property they check is that a module does not reach a
 *                    dependency by ANY spelling: `../store`, `../../vault/store`
 *                    and the alias form all appear side by side as members of an
 *                    unclosable set. Nothing in the text tells those apart from
 *                    a citation, so a detector that reddened them would be
 *                    deleted rather than fixed. Requiring an extension excludes
 *                    every one of them, and it excludes package specifiers and
 *                    bare directory prefixes with them.
 *   `bare-extension` 46 tokens: a comment saying "a `.tsx` file" or naming
 *                    `.d.ts`. An extension alone names no file.
 *   `glob`           14 tokens, almost all this suite naming its own members as
 *                    `*-verify.ts`. A pattern is not a path.
 *   `elided`         4 tokens, a path abbreviated with an ellipsis mid-way.
 *   `foreign-path`   6 tokens: Windows drive letters and backslashed paths, used
 *                    as examples of input the path helpers must handle.
 *   `absolute`       2 tokens, a leading slash, which names a filesystem root
 *                    rather than a repository path.
 *   `not-a-path`     51 tokens carrying a space, a bracket or a path-qualified
 *                    symbol, which no file spelling in this tree does.
 */
export function notAFileSpelling(s: string): string | null {
  // A PROJECT NAME THAT ENDS IN AN EXTENSION is not a file spelling, and
  // `xterm.js` is one. Found by writing this file's own prose: backticking the
  // project name produced a `dead-path` finding against text that is plainly
  // correct, and a comment legitimately naming that project in backticks would
  // have hit the same thing. Read off `UPSTREAM_TRACKERS`, which is already this
  // check's register of upstream project names, so the two cannot disagree and
  // adding a project does not also require remembering this.
  if (UPSTREAM_TRACKERS.includes(s)) return "project-name";
  if (!FILE_EXT.test(s)) return "no-extension";
  if (/^\.[A-Za-z]{1,2}\.[A-Za-z]+$/.test(s)) return "bare-extension";
  if (!/[A-Za-z0-9_)\]]\.[A-Za-z0-9]+$/.test(s)) return "bare-extension";
  if (s.includes("*")) return "glob";
  if (s.includes("...")) return "elided";
  if (s.includes("\\") || /^[A-Za-z]:/.test(s)) return "foreign-path";
  if (s.startsWith("/")) return "absolute";
  if (/[\s()<>]/.test(s) || s.includes("::")) return "not-a-path";
  return null;
}

/**
 * The store files the app itself writes into the user's data directory.
 *
 * A PATTERN rather than a list, which is the one place here where deriving beats
 * enumerating: the app's data files all share this naming convention, a new one
 * lands whenever a new store does, and a pinned list would need a diff each
 * time for no judgement. Asserted below to match NO file in the checkout, which
 * is what stops it hiding a real citation: this convention names things the app
 * creates at runtime and nothing this repository ships.
 *
 * DERIVING FROM STRING LITERALS WAS TRIED AND REFUSED, because the measurement
 * killed it. "Exempt any spelling that appears as a string literal somewhere in
 * the tree" reads as elegantly self-maintaining and exempts 1,043 file
 * spellings, since the language registry and the file-icon registry each
 * enumerate hundreds of filenames as data. That is not an allow-list, it is an
 * opening. Narrowing it to "the initialiser of an exported const" is safe but
 * covers only 3 of the 8 store files, so a pinned list would still be needed
 * beside it: two mechanisms for less coverage than one pattern.
 */
const RUNTIME_STORE_FILE = /^subclave-[a-z0-9-]+\.json$/;

/**
 * The config spellings a formatter accepts in a USER'S opened project.
 *
 * Pinned, sorted and length-asserted: this is a closed set of what one external
 * tool reads, so adding one is a real decision about another tool's behaviour
 * rather than bookkeeping. Note that this repository's own `.prettierrc.json` is
 * deliberately absent: it is checked in, so it resolves normally, and that
 * contrast is the whole point of the list.
 */
const EXTERNAL_CONFIG_NAMES = [".prettierrc.js", ".prettierrc.yaml", "prettier.config.js"];

/**
 * A file name used as an EXAMPLE rather than as a pointer, pinned per citing
 * file so the exemption cannot travel.
 *
 * Keyed on the citing file and the spelling, and deliberately NOT on a line, so
 * the pin survives every edit that moves the comment. A line-keyed exemption
 * list inside a check about rotting line numbers would be its own joke.
 *
 * EMPTY IN THIS TREE, and that is a measurement rather than an oversight: no
 * comment currently quotes a hypothetical file name as prose. There is no
 * mechanical difference between such a name and a citation, which is why the
 * class is enumerated rather than pattern-matched, and an entry here is a
 * deliberate diff in this file rather than a side effect of somebody's comment.
 */
const EXAMPLE_SPELLINGS: string[] = [];

/**
 * A file named in order to say that it is GONE, where the deletion is the
 * sentence's subject.
 *
 * THE ONE PLACE THIS CHECK RECORDS A HUMAN JUDGEMENT INSTEAD OF APPLYING A RULE,
 * and the docblock's WHAT IT DOES NOT SEE section carries the argument. In
 * short: a comment may explain that a module was deleted, so a purge that skips
 * it strands a secret. The sentence is FALSE if the file still exists, so the
 * dead name is load-bearing and removing it would leave the paragraph with no
 * subject.
 *
 * The same dead spelling cited in the present tense, as a live mechanism, is
 * simply wrong. Same text, opposite verdicts, and the difference is tense and
 * grammatical subject: a semantic property no detector over a comment's text can
 * read. Hence an entry rather than a rule. Keyed on the citing file, so the
 * exemption cannot travel to another file that cites the same dead name as
 * though it were alive.
 *
 * EMPTY IN THIS TREE, for the same reason as the list above.
 */
const DELIBERATELY_DEAD: string[] = [];

/** Is a spelling that resolved to nothing nevertheless permitted, and why? */
export function exemptDeadPath(citing: string, spelling: string): string | null {
  if (RUNTIME_STORE_FILE.test(spelling)) return "runtime store file";
  if (EXTERNAL_CONFIG_NAMES.includes(spelling)) return "external tool's config name";
  if (EXAMPLE_SPELLINGS.includes(`${citing} ${spelling}`)) return "example, not a pointer";
  if (DELIBERATELY_DEAD.includes(`${citing} ${spelling}`)) return "deliberately dead";
  return null;
}

/** How many leading directory segments `a` and `b` agree on. */
function sharedDepth(a: string, b: string): number {
  const x = a.split("/");
  const y = b.split("/");
  let n = 0;
  while (n < x.length - 1 && n < y.length - 1 && x[n] === y[n]) n++;
  return n;
}

/**
 * The files `spelling`, cited from `citing`, could mean.
 *
 * Three forms, in order of how much they say:
 *
 *   - a relative spelling resolves against the CITING FILE'S directory, which
 *     is the only reading that makes a dot-slash spelling mean anything, and it
 *     hits one file or none;
 *   - the alias form maps to the source root, per the compiler's own path
 *     mapping;
 *   - anything else is matched as a path suffix on a segment boundary, so
 *     `settings/preferences.ts` finds the one file ending that way.
 *
 * A suffix tie is then broken by NEARNESS to the citing file, which is how a
 * human reads it: a bare `store.ts` in a comment inside the settings module
 * means the settings one, and every reader knows that without being told. This
 * is a ranking and it can pick the wrong file among several that exist, so it is
 * used ONLY to answer "does this name something", never to report which file
 * was meant. Measured: nearness resolves 29 of the 67 spellings that are
 * ambiguous by suffix alone, and every one it resolves sits in the same module
 * as its citation.
 */
export function resolveSpelling(citing: string, spelling: string): string[] {
  if (spelling.startsWith("./") || spelling.startsWith("../")) {
    const p = normalize(join(dirname(citing), spelling))
      .split("\\")
      .join("/");
    return REPO_FILE_SET.has(p) ? [p] : [];
  }
  const bare = spelling.startsWith("@/") ? "src/" + spelling.slice(2) : spelling;
  if (REPO_FILE_SET.has(bare)) return [bare];
  const all = REPO_FILES.filter((f) => f.endsWith("/" + bare));
  if (all.length <= 1) return all;
  const best = Math.max(...all.map((f) => sharedDepth(f, citing)));
  return best === 0 ? all : all.filter((f) => sharedDepth(f, citing) === best);
}

/**
 * Did a bare spelling resolve to the CITING FILE, while other files share the
 * name? The one silently-wrong resolution that is mechanically detectable.
 *
 * `resolveSpelling`'s nearness tie-break is a ranking, and its docblock says it
 * can pick the wrong file among several that exist. That limitation had one live
 * instance and it was the worst-shaped citation in the repository: a launcher
 * crate's own file said a setting was made in its entry point, two files of
 * that name are tracked, and nearness preferred the citing file by maximum
 * shared depth. It
 * resolved UNIQUELY and went green while delivering a reader to the file already
 * open in front of them, where the setting is not made. Green, unique, wrong.
 *
 * WHY THIS SHAPE AND NOT THE GENERAL LIMITATION. A wrong pick among two OTHER
 * files needs the citation's meaning to detect, which nothing here can read. But
 * preferring SELF is different: the citing file always wins nearness against any
 * candidate, automatically and regardless of what the sentence means, so the
 * ranking is doing no work and its answer carries no information. That is
 * checkable, and it is the sub-case the live defect fell into.
 *
 * Relative spellings are excluded because the author wrote the path out, so
 * nothing was inferred on their behalf.
 *
 * ADDED WITH ZERO INSTANCES IN THE TREE, which this file refuses elsewhere: the
 * npm-dependency exemption was declined for being unexercised. The difference is
 * the demonstrated defect. That exemption guarded a class that had never gone
 * wrong; this guards one that went wrong last week in the one directory nothing
 * was scanning, and it is exercised by the controls either way.
 */
export function selfPreferred(citing: string, spelling: string, resolved: string): boolean {
  if (spelling.startsWith("./") || spelling.startsWith("../")) return false;
  if (resolved !== citing) return false;
  return REPO_FILES.filter((f) => f.endsWith("/" + spelling)).length > 1;
}

/** Every backticked run in a comment, which is where a citation is spelled. */
export const BACKTICKED = /`([^`\n]+)`/g;

export function runResolvabilityChecks(): void {
  // A walk that found nothing, or only a handful, would make every resolution
  // fail and every citation look dead. Asserted before anything is resolved.
  check(`the checkout walk found files to resolve against`, REPO_FILES.length > 110, {
    files: REPO_FILES.length,
  });
  check(
    "a file added but not yet committed is resolvable",
    resolveSpelling("scripts/x.ts", "lib/comments.ts").length === 1,
  );
  check(
    "the external-config name list is a sorted set of exactly 3 entries",
    sortedSet(EXTERNAL_CONFIG_NAMES) && EXTERNAL_CONFIG_NAMES.length === 3,
    EXTERNAL_CONFIG_NAMES,
  );
  check(
    "the example-spelling list is a sorted set of exactly 0 entries",
    sortedSet(EXAMPLE_SPELLINGS) && EXAMPLE_SPELLINGS.length === 0,
    EXAMPLE_SPELLINGS,
  );
  check(
    "the deliberately-dead list is a sorted set of exactly 0 entries",
    sortedSet(DELIBERATELY_DEAD) && DELIBERATELY_DEAD.length === 0,
    DELIBERATELY_DEAD,
  );
  // An exemption for a spelling somebody has since checked in is an exemption
  // hiding a resolvable citation, so every list has to stay TRUE and not merely
  // short. This is the half an allow-list normally lacks.
  check(
    "the runtime store-file pattern matches no file in the checkout",
    REPO_FILES.every((f) => !RUNTIME_STORE_FILE.test(f.split("/").pop() ?? "")),
    REPO_FILES.filter((f) => RUNTIME_STORE_FILE.test(f.split("/").pop() ?? "")),
  );
  for (const spelling of EXTERNAL_CONFIG_NAMES) {
    check(
      `the exemption for ${spelling} is still needed, because no such file exists`,
      resolveSpelling("src/x.ts", spelling).length === 0,
    );
  }
  // Two assertions per entry and not one, so a failure says WHICH half broke.
  // Bundled, the label "still needed and still lands" is true of both a citing
  // file that has been deleted and a spelling that has come back, and those want
  // opposite fixes: retire the entry, or delete it and let the citation resolve.
  for (const entry of [...EXAMPLE_SPELLINGS, ...DELIBERATELY_DEAD]) {
    const [citing, spelling] = entry.split(" ");
    // The entry has to still LAND. A citing file that has moved leaves the
    // exemption pinned to nothing, exempting a spelling nobody writes any more.
    check(
      `the exemption for ${spelling} still lands, because ${citing} is there`,
      REPO_FILE_SET.has(citing),
    );
    // And it has to still be NEEDED. This is the inverted half, and it is the
    // failure mode an exemption actually has: the day a deleted file comes back,
    // or somebody checks in a file by that name, the entry stops excusing an
    // unresolvable spelling and starts blindfolding the check to a citation that
    // has become perfectly good. Nothing about the list's own shape would show it,
    // so it is asserted rather than trusted.
    check(
      `the exemption for ${spelling} is still needed, because no such file exists yet`,
      resolveSpelling(citing, spelling).length === 0,
      resolveSpelling(citing, spelling),
    );
  }
}
