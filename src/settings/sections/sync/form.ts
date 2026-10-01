import { describeVaultError } from "@/modules/vault/errors";

/**
 * What this session typed into the secret fields. Never read back out of the
 * vault, and never written into {@link SyncConfig}: it is the argument of one
 * `syncConfigure` call and then dropped.
 *
 * EVERY PROVIDER'S FIELDS IN ONE FLAT SHAPE, not one shape per provider. Only
 * the selected provider's fields are rendered, so the rest stay empty, and
 * every write below is already gated on "the user actually typed something",
 * which makes a per-provider union earn nothing but the narrowing it would then
 * demand at each call site.
 */
export type SecretDraft = {
  passphrase: string;
  accessKeyId: string;
  secretAccessKey: string;
  webdavUsername: string;
  webdavPassword: string;
};

export const EMPTY_SECRETS: SecretDraft = {
  passphrase: "",
  accessKeyId: "",
  secretAccessKey: "",
  webdavUsername: "",
  webdavPassword: "",
};

/** The credentials half of `SyncConfigureArgs`, whichever provider they belong
 *  to. Only filled fields are sent: a blank one would write the empty string,
 *  which every stored-credential check reads as a real secret. */
export type CredentialsArg = {
  accessKeyId?: string;
  secretAccessKey?: string;
  username?: string;
  password?: string;
};

export function credentialsArg(provider: string, draft: SecretDraft): CredentialsArg {
  const out: CredentialsArg = {};
  if (provider === "webdav") {
    if (draft.webdavUsername) out.username = draft.webdavUsername;
    if (draft.webdavPassword) out.password = draft.webdavPassword;
  } else {
    if (draft.accessKeyId) out.accessKeyId = draft.accessKeyId;
    if (draft.secretAccessKey) out.secretAccessKey = draft.secretAccessKey;
  }
  return out;
}

/** What a rejected `invoke` or store call is worth showing. Rust answers with
 *  `"<module>: <sentence>"`, which every other surface in the app reads as a
 *  sentence through `describeVaultError`, so this one does too. */
export function errorText(e: unknown): string {
  const raw = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  return describeVaultError(raw);
}

/** The one placeholder every secret input wears, unconditionally. There is no
 *  presence read to vary it: no command exposes whether the vault holds one. */
export const KEEP_STORED = "Leave blank to keep the stored value.";
