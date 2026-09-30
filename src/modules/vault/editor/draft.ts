// The entry editor's draft model: everything the dialog holds in component
// state while it is open, plus the pure conversions between that draft, the
// wire `EntryDraft` payload and the stored `EntryDetail`.
//
// No React, no state library and no Tauri bindings, so a node script can load
// this file and check the "unchanged stays unchanged" rules that decide whether
// a save clobbers a secret it never loaded.

import type {
  DetailCustomField,
  DraftCustomField,
  EntryDetail,
  EntryDraft,
  EntryUrl,
} from "@/modules/vault/types";

/**
 * One custom field while the editor is open. `valueLoaded` is false until a
 * value entered this session (a hidden field the detail hands out as `null`,
 * or a row that was just added): the save then omits `value` and Rust keeps
 * the stored one, which is why renaming a never-loaded field would lose the
 * edit and the name input stays read-only until the value is shown.
 */
export type EditorCustomField = {
  name: string;
  value: string;
  hidden: boolean;
  valueLoaded: boolean;
};

export type EditorDraft = {
  id: string | null;
  groupId: string;
  title: string;
  username: string;
  /** The stored password is never loaded, so this starts empty. */
  password: string;
  /** True once the user typed a password or took one from the generator. */
  passwordTouched: boolean;
  urls: EntryUrl[];
  notes: string;
  /** The TOTP URI, loaded on demand through `vault_entry_reveal`. */
  totp: string;
  /** True once the user typed a URI, cleared it, or replaced it. */
  totpTouched: boolean;
  customFields: EditorCustomField[];
  tags: string[];
  icon: string | null;
  color: EntryDraft["color"];
  favorite: boolean;
  /** Epoch ms at local midnight, `null` when the entry never expires. */
  expiresAt: number | null;
};

function customFieldFromDetail(field: DetailCustomField): EditorCustomField {
  // A hidden value arrives as `null` (the detail never carries it), which is
  // exactly "not loaded this session".
  return {
    name: field.name,
    value: field.value ?? "",
    hidden: field.hidden,
    valueLoaded: field.value !== null,
  };
}

/** The draft for an existing entry; `null` detail falls back to a create. */
export function draftFromDetail(detail: EntryDetail | null, groupId: string): EditorDraft {
  if (detail === null) return draftFromCreate(groupId);
  return {
    id: detail.id,
    groupId: detail.groupId,
    title: detail.title,
    username: detail.username,
    password: "",
    passwordTouched: false,
    urls: detail.urls.map((url) => ({ ...url })),
    notes: detail.notes,
    totp: "",
    totpTouched: false,
    customFields: detail.customFields.map(customFieldFromDetail),
    tags: [...detail.tags],
    icon: detail.icon,
    color: detail.color,
    favorite: detail.favorite,
    expiresAt: detail.expiresAt,
  };
}

/** An empty draft for a new entry in `groupId`. */
export function draftFromCreate(groupId: string): EditorDraft {
  return {
    id: null,
    groupId,
    title: "",
    username: "",
    password: "",
    passwordTouched: false,
    urls: [],
    notes: "",
    totp: "",
    totpTouched: false,
    customFields: [],
    tags: [],
    icon: null,
    color: null,
    favorite: false,
    expiresAt: null,
  };
}

/**
 * The wire payload. `undefined` keys are dropped by `JSON.stringify`, which is
 * the "unchanged" shape Rust's `double_option` reads: an omitted password keeps
 * the stored one, an omitted TOTP keeps the stored URI.
 */
export function toDraftPayload(draft: EditorDraft): EntryDraft {
  const payload: EntryDraft = {
    id: draft.id,
    groupId: draft.groupId,
    title: draft.title,
    username: draft.username,
    urls: draft.urls
      .filter((url) => url.url.trim() !== "")
      .map((url) => ({ url: url.url.trim(), match: url.match })),
    notes: draft.notes,
    customFields: draft.customFields.map(toCustomFieldPayload),
    tags: draft.tags,
    icon: draft.icon,
    color: draft.color,
    favorite: draft.favorite,
    expiresAt: draft.expiresAt,
  };
  // A create has no stored value to keep, so it always carries one (Rust would
  // otherwise default it to an empty string). On the update path the stored
  // password is never loaded, so an EMPTY field cannot be told apart from a
  // field the user typed into and then cleared: both mean "leave it alone".
  // Sending `""` there would wipe a password the editor never showed.
  if (draft.id === null) payload.password = draft.password;
  else if (draft.passwordTouched && draft.password !== "") payload.password = draft.password;
  // An untouched TOTP is omitted; a touched-but-empty one is the explicit
  // `null` that clears it; anything else is normalised to an otpauth URI.
  if (draft.totpTouched)
    payload.totp = draft.totp.trim() === "" ? null : totpUriFromInput(draft.totp);
  return payload;
}

function toCustomFieldPayload(field: EditorCustomField): DraftCustomField {
  const payload: DraftCustomField = { name: field.name, hidden: field.hidden };
  if (field.valueLoaded) payload.value = field.value === "" ? null : field.value;
  return payload;
}

/** True when the draft differs from the one the dialog opened with. */
export function isDirty(draft: EditorDraft, initial: EditorDraft): boolean {
  const serialise = (d: EditorDraft): string =>
    JSON.stringify([
      d.id,
      d.groupId,
      d.title,
      d.username,
      d.password,
      d.passwordTouched,
      d.urls,
      d.notes,
      d.totp,
      d.totpTouched,
      d.customFields,
      d.tags,
      d.icon,
      d.color,
      d.favorite,
      d.expiresAt,
    ]);
  return serialise(draft) !== serialise(initial);
}

/**
 * Save is blocked while a custom field has no name, or has no value and is not
 * hidden (a new row must carry one; an existing hidden one is kept by name).
 */
export function customFieldsReady(fields: EditorCustomField[]): boolean {
  return fields.every(
    (field) => field.name.trim() !== "" && (field.hidden || field.value.trim() !== ""),
  );
}

/** `expiresAt` (epoch ms at local midnight) as the `YYYY-MM-DD` a date input shows. */
export function epochToDateInput(at: number | null): string {
  if (at === null) return "";
  const date = new Date(at);
  if (Number.isNaN(date.getTime())) return "";
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

/** A date input's value as local midnight epoch ms; `""` becomes `null`. */
export function dateInputToEpoch(value: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const date = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  return Number.isNaN(date.getTime()) ? null : date.getTime();
}

const BASE32_RE = /^[A-Za-z2-7\s=]+$/;

/**
 * Accepts both forms the TOTP row takes: an `otpauth://` URI is returned as
 * typed (and anything that is neither a URI nor a bare base32 secret too, so
 * the parser reports it), a bare base32 secret is wrapped as
 * `otpauth://totp/Subclave?secret=<secret>`.
 */
export function totpUriFromInput(raw: string): string {
  const trimmed = raw.trim();
  if (trimmed.startsWith("otpauth://") || !BASE32_RE.test(trimmed)) return trimmed;
  const secret = trimmed.toUpperCase().replace(/[\s=]/g, "");
  return `otpauth://totp/Subclave?secret=${secret}`;
}
