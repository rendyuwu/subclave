/**
 * Fixtures and helpers shared across the control suites: the symbol the
 * dependency controls cite, and the two reductions every control compares
 * against.
 *
 * Split out of scripts/citation-format-verify.ts, which is now the runner.
 */
import { violationsIn } from "../violations";

/** The symbol every dependency control cites. Real, and reachable through the lockfile. */
export const SYM = "Builder::invoke_handler";

/** The kinds `violationsIn` reports over a control, sorted, for one comparison. */
export const kindsOf = (rel: string, src: string): string =>
  violationsIn(rel, src)
    .map((v) => v.kind)
    .sort()
    .join(",");

/** The lines `violationsIn` reports over a control, in order, as one string. */
export const linesOf = (rel: string, src: string): string =>
  violationsIn(rel, src)
    .map((v) => v.where.split(":").pop())
    .join(",");
