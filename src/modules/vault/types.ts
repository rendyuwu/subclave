// Canonical TypeScript mirrors of the vault projections and drafts in
// `src-tauri/src/modules/vault/model.rs`. Field for field, camelCase, enums
// lowercase; a rename on one side is a compile error on the other, which is
// the point.

export type EntryColor = "red" | "yellow" | "green" | "cyan" | "blue" | "magenta";
export type MatchMode = "domain" | "host" | "exact";
export type VersionReason = "edit" | "restore" | "conflict";

/**
 * The field `vault_entry_reveal` and `clip_copy_field` accept. `custom:<name>`
 * reaches one custom field's value; `history:<updatedAt>:password` a history
 * version's password.
 */
export type RevealField =
  "password" | "totp" | "username" | `custom:${string}` | `history:${number}:password`;

/** Mirrors `vault::VaultStatus`. */
export type VaultStatus = {
  exists: boolean;
  locked: boolean;
  savePending: boolean;
  /** True while the payload came from the `.bak` and every save is refused. */
  saveBlocked: boolean;
  /** Epoch ms mtime of the `.bak`, `null` when there is none. */
  backupAt: number | null;
  /** Ms until the idle deadline; `null` while locked or when auto-lock is off. */
  locksInMs: number | null;
};

/** The wire field is literally `match` (a Rust keyword there). */
export type EntryUrl = { url: string; match: MatchMode };

export type EntrySummary = {
  id: string;
  groupId: string;
  title: string;
  username: string;
  primaryHost: string | null;
  tags: string[];
  icon: string | null;
  color: EntryColor | null;
  favorite: boolean;
  hasPassword: boolean;
  hasTotp: boolean;
  expiresAt: number | null;
  updatedAt: number;
  lastUsedAt: number | null;
};

export type DetailCustomField = { name: string; hidden: boolean; value: string | null };
export type DetailVersion = { updatedAt: number; reason: VersionReason; changed: string[] };

export type EntryDetail = EntrySummary & {
  urls: EntryUrl[];
  notes: string;
  /** Hidden values are masked to `null`: the detail never carries them. */
  customFields: DetailCustomField[];
  history: DetailVersion[];
  createdAt: number;
};

export type DraftCustomField = { name: string; hidden: boolean; value?: string | null };

export type EntryDraft = {
  id: string | null;
  groupId: string;
  title: string;
  username: string;
  /** Omitted keeps the stored password; the create arm needs a value. */
  password?: string;
  urls: EntryUrl[];
  notes: string;
  /** `undefined` = unchanged, `null` = clear, a URI = set. */
  totp?: string | null;
  customFields: DraftCustomField[];
  tags: string[];
  icon: string | null;
  color: EntryColor | null;
  favorite: boolean;
  expiresAt: number | null;
};

export type Group = {
  id: string;
  parentId: string | null;
  name: string;
  icon: string | null;
  color: EntryColor | null;
  createdAt: number;
  updatedAt: number;
};

export type GroupDraft = {
  id: string | null;
  parentId: string | null;
  name: string;
  icon: string | null;
  color: EntryColor | null;
};

export type Strength = { score: number; warning: string | null };

export type GeneratorOptions = {
  length: number;
  lower: boolean;
  upper: boolean;
  digits: boolean;
  symbols: boolean;
  excludeAmbiguous: boolean;
};

export type TotpCode = { code: string; period: number; remaining: number };

/** Mirrors `clipboard::ClearResult`. */
export type ClearResult = { clearsAt: number | null };
