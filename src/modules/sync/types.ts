// The vocabulary the sync module speaks, and the mirrors of the Rust command
// payloads.
//
// KEPT IN LOCKSTEP BY HAND with `src-tauri/src/modules/sync/engine/types.rs`, the same
// way `src/lib/ipc.ts` mirrors the filesystem payloads: `tsc` cannot see across
// the IPC boundary, so a field renamed on one side is `undefined` on the other
// with no error anywhere. Each type below names the Rust type it mirrors.

/** Where the sync module's own settings live. Its own file, so a contended
 *  write here can never clobber a preference or a vault key. */
export const SYNC_STORE_PATH = "subclave-sync.json";

/** Keys inside {@link SYNC_STORE_PATH}. Separate keys rather than one blob, so a
 *  status write and a config write never contend for the same value. */
export const SYNC_CONFIG_KEY = "config";
export const SYNC_STATUS_KEY = "status";

/**
 * Settings asking the main window to do something it is the only window allowed
 * to do.
 *
 * WEBVIEW TO WEBVIEW, so it is not in `src/lib/ipc.ts`: that file mirrors the
 * events the RUST process emits. The passphrase is typed in the settings window
 * and the session it opens lives in the Rust process, so the settings window
 * asks the main window to open one rather than opening one of its own.
 */
export const SYNC_REQUEST_EVENT = "subclave:sync-request";
export type SyncRequest = "pull" | "push";

/** Emitted whenever the recorded status changes, so a second window's pill
 *  shows the same figures without polling. */
export const SYNC_STATUS_EVENT = "subclave://sync-status-changed";

/** Emitted whenever the stored configuration changes. A toggle-off writes the
 *  config but raises no request, so this is what reaches the pill. */
export const SYNC_CONFIG_EVENT = "subclave://sync-config-changed";

/**
 * How long an edit waits for its neighbours before a push goes out.
 *
 * Five seconds, because the unit a user produces is a burst: saving one entry is
 * often followed by two or three more. Per-edit pushes would make that three
 * round trips where one would do.
 */
export const PUSH_DEBOUNCE_MS = 5000;

/**
 * The floor between two focus-driven pulls.
 *
 * Without it, alt-tabbing is a poll loop wearing an event's clothes, and each
 * iteration is a LIST against the user's storage.
 */
export const FOCUS_INTERVAL_MS = 60_000;

/** A pass over the remote, as the status bar renders it. */
export type SyncPhase = "idle" | "syncing";

/**
 * The non-secret half of a sync configuration.
 *
 * WHAT IS NOT HERE: the passphrase and whichever provider credential is in use.
 * Those live in the vault payload's device state, sealed with the rest of the
 * vault, because this file sits in the app data directory in plain JSON.
 *
 * `enabled` off means NO NETWORK, checked before anything is invoked rather than
 * inside the Rust commands.
 */
export type SyncConfig = {
  enabled: boolean;
  /** The provider id `build` in `src-tauri/src/modules/sync/provider.rs`
   *  dispatches on. A bare string and not a union: that `build` is the authority
   *  on which ids exist, and a union here would be a second list to keep in step
   *  with it. */
  provider: string;
  endpoint: string;
  /** S3's, and ignored by any provider that has no such notion. */
  region: string;
  /** S3's, and ignored by any provider that has no such notion. */
  bucket: string;
  /** Where in the remote storage this device's inventory lives. May be empty. */
  prefix: string;
  /** Whether the endpoint honours a conditional write. A STORED USER TOGGLE and
   *  never a probe, see `cas` in `src-tauri/src/modules/sync/provider.rs`. Not
   *  sent to a provider that has no conditional write to offer, which is why
   *  `SyncBehaviour` renders no switch for one. */
  cas: boolean;
};

/**
 * The provider-facing subset of {@link SyncConfig}, exactly the fields Rust's
 * `SyncConfigArg` declares.
 *
 * `enabled` is left out because that struct denies unknown fields, so a stray
 * one is a hard error at configure time rather than a key nobody reads. Both the
 * scheduler and the settings form build their configure arguments through
 * {@link configSubset} so neither can drift from the other.
 */
export type SyncConfigArg = {
  provider: string;
  endpoint: string;
  region: string;
  bucket: string;
  prefix: string;
  cas: boolean;
};

/**
 * The providers `build` in `src-tauri/src/modules/sync/provider.rs` dispatches
 * on, with the label each form shows.
 *
 * A list rather than a constant pair because the id is STORED and the label is
 * not, and the two must not drift apart. Declared once because both forms that
 * configure a provider render it: an id added to that match and missed here
 * would be unreachable from the UI.
 */
export const SYNC_PROVIDERS: { id: string; label: string }[] = [
  { id: "s3", label: "S3-compatible" },
  { id: "webdav", label: "WebDAV (Nextcloud, ownCloud)" },
];

/** The five provider fields plus `cas`, without `enabled`. */
export function configSubset(config: SyncConfig): SyncConfigArg {
  return {
    provider: config.provider,
    endpoint: config.endpoint,
    region: config.region,
    bucket: config.bucket,
    prefix: config.prefix,
    cas: config.cas,
  };
}

/**
 * Whether the fields a provider actually needs are filled in.
 *
 * SHARED WITH `scripts/sync-verify.ts`, and that is the point: this is the one
 * boolean in the join form that can make a whole provider unusable while the
 * form still looks complete, so it lives where a check can drive it rather than
 * inside a component.
 *
 * REGION AND BUCKET ARE S3'S ALONE. A WebDAV configuration refuses unknown
 * fields, so demanding them for a WebDAV server would make the form impossible
 * to submit.
 */
export function connectionFieldsReady(config: SyncConfig, credentialsReady: boolean): boolean {
  if (config.endpoint.trim().length === 0) return false;
  if (!credentialsReady) return false;
  if (config.provider === "webdav") return true;
  return config.region.trim().length > 0 && config.bucket.trim().length > 0;
}

/** A device with sync never configured. Off, and naming nothing. */
export const DEFAULT_SYNC_CONFIG: SyncConfig = {
  enabled: false,
  provider: "s3",
  endpoint: "",
  region: "",
  bucket: "",
  prefix: "subclave",
  cas: false,
};

/**
 * What the status bar and the settings window render, written by the main
 * window and read by settings.
 *
 * IN THE STORE RATHER THAN IN A COMMAND, because neither the pending count nor
 * the quarantine list is reachable from Rust alone: the settings window has no
 * session, so it reads what the main window learned.
 */
export type SyncStatus = {
  lastPullAt: number | null;
  lastPushAt: number | null;
  /** Records the remote does not yet hold this device's copy of. */
  pending: number;
  /** Remote objects this device could not read, by their opaque names. */
  quarantine: { name: string; reason: string }[];
  /** Records this device holds that the remote has no object for and that are
   *  older than the tombstone window: reported, never deleted. */
  stale: { kind: string; id: string }[];
  lastError: string | null;
  /** True while the vault is locked, so every trigger is dropped rather than
   *  opening a session against a locked vault. */
  paused: boolean;
};

export const EMPTY_SYNC_STATUS: SyncStatus = {
  lastPullAt: null,
  lastPushAt: null,
  pending: 0,
  quarantine: [],
  stale: [],
  lastError: null,
  paused: false,
};

/**
 * The provider credential pair, whatever the provider calls its halves.
 *
 * Never stored in this module's file: it travels as an argument, and Rust puts
 * it in the vault payload.
 */
export type SyncCredentialsArg = {
  accessKeyId?: string;
  secretAccessKey?: string;
  username?: string;
  password?: string;
};

/** Mirrors Rust `SyncConfigureArgs`. */
export type SyncConfigureArgs = {
  config: SyncConfigArg;
  credentials?: SyncCredentialsArg;
  /** Only non-empty drafts are sent; an absent one keeps the stored value. */
  passphrase?: string;
  create?: boolean;
};

/** Mirrors Rust `SyncConfigureResult`. */
export type SyncConfigureResult = {
  remote: string;
};

/** Mirrors Rust `SyncJoinArgs`. */
export type SyncJoinArgs = {
  masterPassword: string;
  config: SyncConfigArg;
  credentials: SyncCredentialsArg;
  passphrase: string;
};

/** Mirrors Rust `SyncJoinResult`. */
export type SyncJoinResult = {
  remote: string;
  landed: number;
  quarantine: { name: string; reason: string }[];
};

/** Mirrors Rust `SyncPullResult`. */
export type SyncPullResult = {
  /** Records the remote is still missing, as of the reconcile. */
  pending: number;
  landed: number;
  quarantine: { name: string; reason: string }[];
  stale: { kind: string; id: string }[];
};

/** Mirrors Rust `SyncPushResult`. The failure reasons stay in Rust. */
export type SyncPushResult = {
  pushed: number;
  failed: number;
};

/**
 * The Rust commands the SCHEDULER drives, as a port.
 *
 * NAMED METHODS rather than one `invoke(command, args)`: the command-registry
 * scanner reads the command name as a LITERAL at the `invoke` call, so a generic
 * port would make this module a dynamic call site to pin. They also keep the
 * scheduler free of a Tauri import at module scope, so a plain node check can
 * load it.
 */
export type SyncCommands = {
  configure(args: SyncConfigureArgs): Promise<SyncConfigureResult>;
  disable(): Promise<void>;
  pull(): Promise<SyncPullResult>;
  push(): Promise<SyncPushResult>;
  join(args: SyncJoinArgs): Promise<SyncJoinResult>;
};
