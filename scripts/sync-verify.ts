/**
 * Self-check for the sync scheduler and the sync module's static shape.
 * Run: `pnpm run verify sync`.
 *
 * WHY IT EXISTS. The scheduler decides WHEN sync runs, and it is the only file
 * that decides it: the push debounce, the rate-limited focus pull, the pause on
 * a locked vault and the session memo are all here and nowhere else. The Rust
 * engine decides what a pass MEANS, so the triggers have no other test. The
 * scheduler imports no Tauri surface at module scope, so a plain node process
 * loads it with the commands, the store, the clock and the timers injected.
 *
 * THE SECOND HALF reads the tree and asserts the port landed under this fork's
 * own names: the two subkey labels, the keyfile format, the conditional-write
 * verb, and the absence of the upstream vocabulary later edits are most likely
 * to copy back in.
 */
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import type { Scheduler, SchedulerIo } from "@/modules/sync/scheduler";
import { createScheduler } from "@/modules/sync/scheduler";
import type { SyncSettingsStore } from "@/modules/sync/store";
import type {
  SyncCommands,
  SyncConfig,
  SyncConfigureResult,
  SyncPhase,
  SyncPullResult,
  SyncPushResult,
  SyncStatus,
} from "@/modules/sync/types";
import {
  connectionFieldsReady,
  DEFAULT_SYNC_CONFIG,
  EMPTY_SYNC_STATUS,
} from "@/modules/sync/types";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

let failed = 0;
function check(name: string, ok: boolean, detail?: unknown): void {
  if (ok) {
    console.log(`  ok: ${name}`);
    return;
  }
  console.error(`  FAIL: ${name}`, detail === undefined ? "" : JSON.stringify(detail));
  failed++;
}

/** Let every pending microtask settle. The scheduler uses no real timers of its
 *  own, so one turn of the event loop drains a whole injected pass. */
const flush = (): Promise<void> => new Promise((resolve) => setImmediate(resolve));

// ---------------------------------------------------------------------------
// The harness: the real scheduler over a fake clock and a fake timer table.
// ---------------------------------------------------------------------------

type Harness = {
  scheduler: Scheduler;
  io: SchedulerIo;
  /** Command names in call order. */
  calls: string[];
  /** Every phase the scheduler reported, in order. */
  phases: SyncPhase[];
  /** Swap in a different answer or a rejection per command. */
  respond: {
    configure: () => Promise<SyncConfigureResult>;
    pull: () => Promise<SyncPullResult>;
    push: () => Promise<SyncPushResult>;
  };
  /** Move the fake clock and fire every timer that comes due. */
  advance(ms: number): void;
  setConfig(next: Partial<SyncConfig>): void;
  status(): SyncStatus;
  /** Timers still pending. */
  timers(): number;
};

function makeHarness(config: Partial<SyncConfig> = {}): Harness {
  let clock = 0;
  let nextHandle = 1;
  const timers = new Map<number, { at: number; fn: () => void }>();

  const calls: string[] = [];
  const phases: SyncPhase[] = [];
  let current: SyncStatus = { ...EMPTY_SYNC_STATUS };
  let storedConfig: SyncConfig = { ...DEFAULT_SYNC_CONFIG, ...config };

  const respond = {
    configure: async (): Promise<SyncConfigureResult> => ({ remote: "existing" }),
    pull: async (): Promise<SyncPullResult> => ({
      pending: 0,
      landed: 0,
      quarantine: [],
      stale: [],
    }),
    push: async (): Promise<SyncPushResult> => ({ pushed: 0, failed: 0 }),
  };

  const settings: SyncSettingsStore = {
    readConfig: async () => ({ ...storedConfig }),
    writeConfig: async (next) => {
      storedConfig = next;
    },
    readStatus: async () => ({ ...EMPTY_SYNC_STATUS }),
    writeStatus: async () => {},
  };

  const commands: SyncCommands = {
    configure: async () => {
      calls.push("configure");
      return respond.configure();
    },
    disable: async () => {
      calls.push("disable");
    },
    pull: async () => {
      calls.push("pull");
      return respond.pull();
    },
    push: async () => {
      calls.push("push");
      return respond.push();
    },
    join: async () => ({ remote: "existing", landed: 0, quarantine: [] }),
  };

  const io: SchedulerIo = {
    label: "main",
    commands,
    settings,
    onStatus: (status) => {
      current = { ...status };
    },
    onPhase: (phase) => {
      phases.push(phase);
    },
    now: () => clock,
    setTimer: (fn, ms) => {
      const handle = nextHandle++;
      timers.set(handle, { at: clock + ms, fn });
      return handle;
    },
    clearTimer: (handle) => {
      timers.delete(handle as number);
    },
  };

  const scheduler = createScheduler(io);

  function advance(ms: number): void {
    const target = clock + ms;
    for (;;) {
      const due = [...timers.entries()]
        .filter(([, timer]) => timer.at <= target)
        .sort((a, b) => a[1].at - b[1].at);
      if (due.length === 0) break;
      const [handle, timer] = due[0];
      timers.delete(handle);
      clock = timer.at;
      timer.fn();
    }
    clock = target;
  }

  return {
    scheduler,
    io,
    calls,
    phases,
    respond,
    advance,
    setConfig: (next) => {
      storedConfig = { ...storedConfig, ...next };
    },
    status: () => current,
    timers: () => timers.size,
  };
}

// ---------------------------------------------------------------------------
// [debounce] a burst of edits publishes once, five seconds after the first.
// ---------------------------------------------------------------------------

console.log("\n[debounce] one push per burst, and the window does not slide");
{
  const h = makeHarness({ enabled: true });
  h.scheduler.markDirty();
  h.advance(1000);
  h.scheduler.markDirty();
  check("the second mark is folded into the open window", h.timers() === 1, h.timers());
  h.advance(3999);
  check("no push before the five second window closes", h.calls.length === 0, h.calls);
  h.advance(1);
  await flush();
  check(
    "the burst fired exactly one push",
    h.calls.filter((name) => name === "push").length === 1,
    h.calls,
  );
  check("the push opened the session first", h.calls.join(",") === "configure,push", h.calls);
}

// ---------------------------------------------------------------------------
// [focus] a rate-limited pull, and a failed pull still costs the window.
// ---------------------------------------------------------------------------

console.log("\n[focus] the sixty second floor holds on success and on failure");
{
  const h = makeHarness({ enabled: true });
  h.scheduler.onUnlocked();
  await flush();
  check(
    "unlocking configures the session, then reconciles",
    h.calls.join(",") === "configure,pull,push",
    h.calls,
  );

  h.calls.length = 0;
  h.advance(10_000);
  h.scheduler.onFocus();
  await flush();
  check("a focus inside the floor does nothing", h.calls.length === 0, h.calls);

  h.advance(51_000);
  h.scheduler.onFocus();
  await flush();
  check("a focus past the floor pulls", h.calls.includes("pull"), h.calls);
}

{
  const h = makeHarness({ enabled: true });
  h.respond.pull = async () => {
    throw new Error("network down");
  };
  h.scheduler.onUnlocked();
  await flush();
  check("a rejected pull is reported", h.status().lastError === "network down", h.status());
  check("a failed pull stamps no lastPullAt", h.status().lastPullAt === null, h.status());

  const before = h.calls.length;
  h.advance(10_000);
  h.scheduler.onFocus();
  await flush();
  check("the rejected pull still consumed the focus window", h.calls.length === before, h.calls);
  h.advance(51_000);
  h.scheduler.onFocus();
  await flush();
  check("the next focus past the floor tries again", h.calls.length > before, h.calls);
}

// ---------------------------------------------------------------------------
// [pull tail] a pull writes its figures, then flushes the push body itself.
// ---------------------------------------------------------------------------

console.log("\n[pull tail] the internal push runs inside the same pass, not queued behind it");
{
  const h = makeHarness({ enabled: true });
  h.scheduler.pullNow();
  await flush();
  check(
    "a successful pull writes lastPullAt and clears the error",
    h.status().lastPullAt === 0 && h.status().lastError === null,
    h.status(),
  );
  check(
    "the pull flushed the push body after itself",
    h.calls.includes("push") && h.calls.indexOf("pull") < h.calls.indexOf("push"),
    h.calls,
  );
  check(
    "the tail push did not re-enter the serialized chain",
    h.phases.join(",") === "syncing,idle",
    h.phases,
  );
}

// ---------------------------------------------------------------------------
// [pause] a locked vault sends nothing.
// ---------------------------------------------------------------------------

console.log("\n[pause] a locked vault drops every trigger");
{
  const h = makeHarness({ enabled: true });
  h.scheduler.onUnlocked();
  await flush();
  h.scheduler.onLocked();
  await flush();
  check("locking pauses the scheduler", h.status().paused === true, h.status());

  const before = h.calls.length;
  h.scheduler.pullNow();
  await flush();
  h.scheduler.pushNow();
  await flush();
  check("a locked scheduler attempts no command", h.calls.length === before, h.calls);
}

// ---------------------------------------------------------------------------
// [unlock] the enable switch decides between a pull and a disable.
// ---------------------------------------------------------------------------

console.log("\n[unlock] enabled pulls; disabled asks Rust to clear any residue");
{
  const h = makeHarness({ enabled: false });
  h.scheduler.onUnlocked();
  await flush();
  check("unlocking with sync off disables the session", h.calls.join(",") === "disable", h.calls);
}

// ---------------------------------------------------------------------------
// [dispose] the pending debounce is dropped.
// ---------------------------------------------------------------------------

console.log("\n[dispose] a disposed scheduler leaves no timer behind");
{
  const h = makeHarness({ enabled: true });
  h.scheduler.markDirty();
  check("a pending push timer is scheduled", h.timers() === 1, h.timers());
  h.scheduler.dispose();
  h.advance(10_000);
  await flush();
  check(
    "dispose dropped the pending timer and ran nothing",
    h.timers() === 0 && h.calls.length === 0,
    { timers: h.timers(), calls: h.calls },
  );
}

// ---------------------------------------------------------------------------
// [phase] the pill's spinner has a source, and it always returns to idle.
// ---------------------------------------------------------------------------

console.log("\n[phase] syncing while a pass runs, idle whenever it settles");
{
  const h = makeHarness({ enabled: true });
  let release: (() => void) | undefined;
  h.respond.pull = (): Promise<SyncPullResult> =>
    new Promise((resolve) => {
      release = () => resolve({ pending: 0, landed: 0, quarantine: [], stale: [] });
    });
  h.scheduler.pullNow();
  await flush();
  check("a running pass reports syncing", h.phases.join(",") === "syncing", h.phases);
  if (!release) throw new Error("the pull never started");
  release();
  await flush();
  check("a settled pass reports idle", h.phases.join(",") === "syncing,idle", h.phases);
}

{
  const h = makeHarness({ enabled: true });
  h.respond.pull = async () => {
    throw new Error("boom");
  };
  h.scheduler.pullNow();
  await flush();
  check(
    "a pass whose command rejected also reports idle",
    h.phases.join(",") === "syncing,idle",
    h.phases,
  );
}

{
  // A rejection out of the pass itself, not out of a command: the status port
  // throwing is caught and logged by the serializer, and idle still lands.
  const h = makeHarness({ enabled: true });
  h.io.onStatus = () => {
    throw new Error("status port down");
  };
  const logged: unknown[][] = [];
  const realError = console.error;
  console.error = (...args: unknown[]) => {
    logged.push(args);
  };
  try {
    h.scheduler.pullNow();
    await flush();
  } finally {
    console.error = realError;
  }
  check("a rejected pass still reports idle", h.phases.join(",") === "syncing,idle", h.phases);
  check("the rejected pass is logged, not thrown", logged.length === 1, logged.length);
}

// ---------------------------------------------------------------------------
// [session] the fresh answer is not memoized; an off config calls nothing.
// ---------------------------------------------------------------------------

console.log("\n[session] a fresh remote is re-checked, and off means no request at all");
{
  const h = makeHarness({ enabled: true });
  h.respond.configure = async () => ({ remote: "fresh" });
  h.scheduler.pullNow();
  await flush();
  h.scheduler.pullNow();
  await flush();
  check(
    "a fresh remote is reported",
    h.status().lastError === "No Subclave data at this location",
    h.status(),
  );
  check(
    "a fresh answer is not memoized, so the next trigger configures again",
    h.calls.filter((name) => name === "configure").length === 2,
    h.calls,
  );
  check("a fresh remote pulls nothing", !h.calls.includes("pull"), h.calls);
}

{
  const h = makeHarness({ enabled: false });
  h.scheduler.markDirty();
  h.advance(10_000);
  await flush();
  h.scheduler.pullNow();
  await flush();
  h.scheduler.pushNow();
  await flush();
  check("a disabled config makes no command call at all", h.calls.length === 0, h.calls);
}

// ---------------------------------------------------------------------------
// [static] this fork's vocabulary, and none of the upstream leftovers.
// ---------------------------------------------------------------------------

console.log("\n[static] the ported tree carries this fork's own names");

const readFile = (rel: string): string => readFileSync(join(ROOT, rel), "utf8");

function filesUnder(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(join(ROOT, dir), { withFileTypes: true })) {
    const rel = `${dir}/${entry.name}`;
    if (entry.isDirectory()) out.push(...filesUnder(rel));
    else out.push(rel);
  }
  return out;
}

const SYNC_RUST = "src-tauri/src/modules/sync";
const rustFiles = filesUnder(SYNC_RUST).sort();
check("the sync module's Rust files were found to scan", rustFiles.length >= 8, rustFiles.length);

for (const term of [
  "tervia",
  "pbkdf2",
  "carrysecrets",
  "purge_secrets",
  "hasprivatekey",
  "key_kind",
]) {
  const hits = rustFiles.filter((file) => readFile(file).toLowerCase().includes(term));
  check(`no "${term}" remains under ${SYNC_RUST}`, hits.length === 0, hits);
}

const crypto = readFile(`${SYNC_RUST}/crypto.rs`);
for (const label of ["subclave-sync-record-key", "subclave-sync-object-name-key"]) {
  check(`crypto.rs names its subkey label "${label}"`, crypto.includes(label));
}
check('crypto.rs pins the keyfile format "subclave-sync"', crypto.includes('"subclave-sync"'));

const provider = readFile(`${SYNC_RUST}/provider.rs`);
check("provider.rs declares put_if_absent on the trait", /fn\s+put_if_absent\b/.test(provider));

const engine = readFile(`${SYNC_RUST}/engine.rs`);
check("engine.rs defines no sync_purge_secrets", !engine.includes("sync_purge_secrets"));

const syncTypes = readFile("src/modules/sync/types.ts");
check(
  'types.ts pins SYNC_STORE_PATH to "subclave-sync.json"',
  syncTypes.includes('SYNC_STORE_PATH = "subclave-sync.json"'),
);

const syncStore = readFile("src/modules/sync/store.ts");
check(
  "store.ts holds no readEtags, so the etags stay in the vault",
  !syncStore.includes("readEtags"),
);
check(
  "store.ts holds no readDirty, so the dirty set stays in the vault",
  !syncStore.includes("readDirty"),
);

// The join form's gate. Its whole value is that it is a checked predicate
// rather than a condition inside a component: a flipped one makes a provider
// permanently unsubmittable while the form still looks complete.
{
  const base: SyncConfig = {
    enabled: true,
    provider: "s3",
    endpoint: "https://storage.example",
    region: "us-east-1",
    bucket: "subclave",
    prefix: "subclave",
    cas: true,
  };
  const webdav: SyncConfig = { ...base, provider: "webdav", region: "", bucket: "" };
  check("a webdav connection needs no region and no bucket", connectionFieldsReady(webdav, true));
  check(
    "a webdav connection still needs its own credential pair",
    !connectionFieldsReady(webdav, false),
  );
  check("an s3 connection needs a region and a bucket", connectionFieldsReady(base, true));
  check(
    "an s3 connection without a bucket is refused",
    !connectionFieldsReady({ ...base, bucket: "" }, true),
  );
  check(
    "a connection with no endpoint is refused",
    !connectionFieldsReady({ ...webdav, endpoint: "  " }, true),
  );
}

console.log(failed === 0 ? "\nAll sync checks passed." : `\n${failed} check(s) FAILED.`);
process.exit(failed === 0 ? 0 : 1);
