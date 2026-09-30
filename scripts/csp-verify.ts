/**
 * Self-check for the release CSP: `app.security.csp` in
 * `src-tauri/tauri.conf.json`.
 * Run: `npx tsx scripts/csp-verify.ts`.
 *
 * The policy gates the webview only in a release build (`tauri dev` loads the
 * dev server and Tauri injects no CSP there), and the config is JSON that no
 * compiler reads, so widening it - a remote `connect-src` for "just one call",
 * an `'unsafe-inline'` script for "just one inline handler" - is invisible to
 * every other gate while it silently hands the webview network access the app
 * must not have. The app talks to its own backend over the Tauri IPC channel and
 * nothing else, so `connect-src ipc: http://ipc.localhost` is the whole of what
 * it needs.
 *
 * `cspProblems` is pure; the three fixtures below prove each rule fires, and the
 * real config proves the shipped policy is clean.
 */
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** The only sources `connect-src` may name: the Tauri IPC channel. */
const ALLOWED_CONNECT_SRC = new Set(["ipc:", "http://ipc.localhost"]);

/** The only source `script-src` may name. Any other token - a remote origin, `*`,
 *  `data:`, `'unsafe-inline'` or `'unsafe-eval'` - defeats the directive. */
const ALLOWED_SCRIPT_SRC = "'self'";

/**
 * The sources of one directive, or null when the policy does not name it.
 *
 * Split on `;` rather than pattern-matched: a directive's own tokens can look
 * like another directive's name (`http://ipc.localhost`), so only the first token
 * of a semicolon-separated part is the name being asked for.
 */
function sourcesOf(csp: string, name: string): string[] | null {
  for (const part of csp.split(";")) {
    const tokens = part.trim().split(/\s+/).filter(Boolean);
    if (tokens[0]?.toLowerCase() === name) return tokens.slice(1);
  }
  return null;
}

/** The `app.security.csp` value `conf` carries, or undefined if the shape differs. */
function cspOf(conf: unknown): unknown {
  if (!conf || typeof conf !== "object" || !("app" in conf)) return undefined;
  const app = conf.app;
  if (!app || typeof app !== "object" || !("security" in app)) return undefined;
  const security = app.security;
  if (!security || typeof security !== "object" || !("csp" in security)) return undefined;
  return security.csp;
}

/**
 * What is wrong with `conf`'s CSP, or nothing. One string per problem, so a
 * failing run says which rule broke rather than only that one did.
 */
function cspProblems(conf: unknown): string[] {
  const out: string[] = [];
  const csp = cspOf(conf);
  if (typeof csp !== "string" || csp.trim() === "") {
    // Nothing below can be read out of a policy that is not there, and `null` is
    // how the config spells "no CSP at all": Tauri then injects none and the
    // webview runs unrestricted.
    return ["app.security.csp is missing or not a string"];
  }

  const connect = sourcesOf(csp, "connect-src");
  if (connect === null) {
    out.push("connect-src is missing, so the fallback allows whatever default-src does");
  } else {
    const outside = connect.filter((s) => !ALLOWED_CONNECT_SRC.has(s));
    if (outside.length > 0) {
      out.push(`connect-src names a source outside the IPC channel: ${outside.join(", ")}`);
    }
  }

  const script = sourcesOf(csp, "script-src") ?? [];
  const scriptOutside = script.filter((s) => s !== ALLOWED_SCRIPT_SRC);
  if (scriptOutside.length > 0) {
    out.push(`script-src names a source other than 'self': ${scriptOutside.join(", ")}`);
  }

  if (sourcesOf(csp, "default-src")?.join(" ") !== "'self'") {
    out.push("default-src is not 'self'");
  }
  if (sourcesOf(csp, "object-src")?.join(" ") !== "'none'") {
    out.push("object-src is not 'none'");
  }
  return out;
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const realConf: unknown = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
const realCspValue = cspOf(realConf);
const realCsp = typeof realCspValue === "string" ? realCspValue : "";

/** The real config with only `csp` replaced, so a fixture cannot drift from it. */
function confWithCsp(csp: unknown): unknown {
  if (!realConf || typeof realConf !== "object" || !("app" in realConf)) {
    throw new Error("csp-verify: tauri.conf.json has no app object to mutate");
  }
  const app = realConf.app;
  if (!app || typeof app !== "object" || !("security" in app)) {
    throw new Error("csp-verify: tauri.conf.json has no app.security object to mutate");
  }
  const security = app.security;
  if (!security || typeof security !== "object") {
    throw new Error("csp-verify: tauri.conf.json has no app.security object to mutate");
  }
  return { ...realConf, app: { ...app, security: { ...security, csp } } };
}

let failed = 0;
function check(label: string, ok: boolean, detail?: unknown): void {
  if (ok) {
    console.log(`  ok: ${label}`);
    return;
  }
  console.error(`  FAIL: ${label}`, detail === undefined ? "" : JSON.stringify(detail));
  failed++;
}

console.log("[real] the policy that ships");
{
  const problems = cspProblems(realConf);
  check("src-tauri/tauri.conf.json has no CSP problem", problems.length === 0, problems);
}

console.log("\n[fixtures] a broken policy is refused");
const FIXTURES: Array<[string, unknown]> = [
  ["csp: null", confWithCsp(null)],
  [
    "connect-src widened with a remote origin",
    confWithCsp(
      realCsp.replace(
        "connect-src ipc: http://ipc.localhost",
        "connect-src ipc: http://ipc.localhost https://evil.example",
      ),
    ),
  ],
  [
    "script-src 'unsafe-inline'",
    confWithCsp(realCsp.replace("script-src 'self'", "script-src 'self' 'unsafe-inline'")),
  ],
  [
    "script-src widened with a remote origin",
    confWithCsp(realCsp.replace("script-src 'self'", "script-src 'self' https://cdn.example")),
  ],
];
for (const [label, conf] of FIXTURES) {
  const problems = cspProblems(conf);
  check(`${label} yields at least one problem`, problems.length > 0, problems);
}

if (failed > 0) process.exit(1);
console.log("\ncsp-verify: all checks passed");
