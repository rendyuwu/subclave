// The wire contract, mirrored from `src-tauri/src/modules/browser/protocol.rs`
// and `.../frame.rs`. These literals exist in exactly two places; the Rust side
// and this file. `scripts/browser-verify.ts` compares them.

export const NATIVE_HOST = "dev.rendy.subclave";
export const PROTOCOL_VERSION = 1;

/** extension -> app (the proxy's request cap). */
export const MAX_REQUEST_FRAME = 64 * 1024;
/** app -> extension (the proxy's response cap). */
export const MAX_RESPONSE_FRAME = 1024 * 1024;
/** Chrome's own limit on a native message to the host. */
export const MAX_STREAM_FRAME = 64 * 1024 * 1024;

/** How long an idle connection is kept open between sends. */
export const IDLE_TIMEOUT_MS = 30_000;

export type NmAction =
  | "status"
  | "focus-app"
  | "associate"
  | "hello"
  | "auth"
  | "get-logins"
  | "check-login"
  | "get-credential"
  | "save-login"
  | "generate-password";

export type NmErrorCode =
  | "app-not-running"
  | "vault-locked"
  | "not-associated"
  | "auth-failed"
  | "pairing-denied"
  | "busy"
  | "no-match"
  | "bad-request"
  | "too-large"
  | "version";

export type NmRequest = {
  v: number;
  id: string;
  action: NmAction;
  params?: unknown;
};

export type NmError = { code: NmErrorCode; message: string };

export type NmResponse =
  | { v: number; id: string; ok: true; result: unknown }
  | { v: number; id: string; ok: false; error: NmError };

export type StatusResult = { locked: boolean; appVersion: string; protocol: number };

export type LoginSummary = {
  id: string;
  title: string;
  username: string;
  group: string;
  lastUsedAt: number | null;
};

export type GetLoginsResult = { entries: LoginSummary[]; otherMatches: number; domain: string };
export type CredentialResult = { username: string; password: string };
export type SaveLoginResult = { id: string; created: boolean };
export type SaveCandidate = { id: string; title: string; username: string };
export type CheckLoginResult = {
  state: "new" | "changed" | "unchanged" | "other-site";
  entries: SaveCandidate[];
};
export type GenerateResult = { password: string };
export type PairResult = { clientId: string; secret: string };
export type HelloResult = { appNonce: string; appProof: string };
