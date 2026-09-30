/**
 * Self-check for the entry editor's draft model and the error copy.
 * Run: `npx tsx scripts/vault-draft-verify.ts`.
 *
 * `draft.ts` decides whether a save CLOBBERS a secret it never loaded: an
 * omitted `password` keeps the stored one, an omitted `totp` keeps the stored
 * URI, and a custom field whose value never entered this session keeps what
 * Rust holds. Every one of those failures is silent in the UI (the save
 * reports success and the stored secret is gone), which is why the rules are
 * pinned here rather than left to the dialog's own behaviour.
 *
 * `describeVaultError` is checked for the same reason: it is the single
 * translation between Rust's `"<module>: <sentence>"` errors and the words on
 * screen, including the two sentences whose wording is fixed.
 */
import {
  customFieldsReady,
  dateInputToEpoch,
  draftFromCreate,
  draftFromDetail,
  epochToDateInput,
  isDirty,
  toDraftPayload,
  totpUriFromInput,
  type EditorDraft,
} from "../src/modules/vault/editor/draft";
import { describeVaultError } from "../src/modules/vault/errors";
import type { EntryDetail } from "../src/modules/vault/types";

let failed = 0;
function check(label: string, ok: boolean, detail?: unknown): void {
  if (ok) {
    console.log(`  ok: ${label}`);
    return;
  }
  console.error(`  FAIL: ${label}`, detail === undefined ? "" : JSON.stringify(detail));
  failed++;
}

/** The wire shape the check compares against: what `JSON.stringify` sends. */
function wire(draft: EditorDraft): Record<string, unknown> {
  return JSON.parse(JSON.stringify(toDraftPayload(draft))) as Record<string, unknown>;
}

const URI = "otpauth://totp/T?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

function detail(overrides: Partial<EntryDetail> = {}): EntryDetail {
  return {
    id: "e1",
    groupId: "root",
    title: "Site",
    username: "user",
    primaryHost: "example.com",
    tags: ["web"],
    icon: null,
    color: null,
    favorite: false,
    hasPassword: true,
    hasTotp: true,
    expiresAt: null,
    updatedAt: 1,
    lastUsedAt: null,
    urls: [{ url: "https://example.com", match: "domain" }],
    notes: "notes",
    customFields: [
      { name: "Shown", hidden: false, value: "visible" },
      { name: "Secret", hidden: true, value: null },
    ],
    history: [],
    createdAt: 1,
    ...overrides,
  };
}

console.log("[draft] the password is never loaded and stays omitted");
{
  const d = draftFromDetail(detail(), "root");
  check("the draft starts with an empty password", d.password === "");
  check("and reports it untouched", d.passwordTouched === false);
  check("so the payload omits it", !("password" in wire(d)), wire(d));
  // The stored password is never loaded, so an empty field cannot be told
  // apart from one the user typed into and then cleared: both must leave the
  // stored value alone rather than saving "".
  check(
    "typing and then clearing is still an omission",
    !("password" in wire({ ...d, password: "", passwordTouched: true })),
    wire({ ...d, password: "", passwordTouched: true }),
  );
  const typed = { ...d, password: "new-pw", passwordTouched: true };
  check("typing sends it", wire(typed).password === "new-pw");
  const generated = { ...d, password: "generated-pw", passwordTouched: true };
  check("the generator's Use sends it", wire(generated).password === "generated-pw");
}

console.log("[draft] a create always sends a password");
{
  const p = wire(draftFromCreate("root"));
  check("the key is present even with nothing typed", "password" in p);
  check("and it is the empty string Rust would otherwise default to", p.password === "");
}

console.log("[draft] TOTP: unchanged, cleared, set");
{
  const d = draftFromDetail(detail(), "root");
  check("untouched omits the key entirely", !("totp" in wire(d)));
  check("an explicit clear sends null", wire({ ...d, totpTouched: true }).totp === null);
  const set = { ...d, totp: URI, totpTouched: true };
  check("a URI is sent through", wire(set).totp === URI);
  const bare = { ...d, totp: "gezd gnbv gy3t qojq", totpTouched: true };
  check(
    "a bare base32 secret is wrapped",
    wire(bare).totp === "otpauth://totp/Subclave?secret=GEZDGNBVGY3TQOJQ",
    wire(bare).totp,
  );
  // Two real forms, and everything else passed through so the parser reports.
  check("an otpauth URI is returned unchanged", totpUriFromInput(URI) === URI);
  check("surrounding whitespace is trimmed", totpUriFromInput(`  ${URI} `) === URI);
  check(
    "padding and lower case are normalised into the bare form",
    totpUriFromInput("  gezdgnbvgy3tqojqgezdgnbvgy3tqojq== ") ===
      "otpauth://totp/Subclave?secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ",
  );
  check("junk is passed through for the parser", totpUriFromInput("nope!") === "nope!");
}

console.log("[draft] custom fields keep a value this session never loaded");
{
  const d = draftFromDetail(detail(), "root");
  check("a visible field keeps its loaded value", d.customFields[0].valueLoaded);
  check("a hidden one is not loaded", !d.customFields[1].valueLoaded);
  const hidden = wire(d).customFields as { name: string; value?: unknown }[];
  check("so its value key is omitted", !("value" in hidden[1]), hidden[1]);
  check("while the visible one is sent", hidden[0].value === "visible");
  // Once revealed, an emptied value sends the explicit null. Rust treats a
  // null on an EXISTING same-name field as "keep the stored value", so this is
  // a no-op there rather than a clear: clearing a hidden value is not
  // expressible through a draft, which is why the editor must not offer it.
  const revealed = {
    ...d,
    customFields: [{ ...d.customFields[1], value: "", valueLoaded: true }],
  };
  check(
    "a revealed-then-emptied hidden field sends the explicit null",
    (wire(revealed).customFields as { value?: unknown }[])[0].value === null,
  );
  check(
    "and an unloaded one is omitted rather than sent as null",
    !("value" in (wire(d).customFields as { value?: unknown }[])[1]),
  );
  const added = {
    ...d,
    customFields: [
      ...d.customFields,
      { name: "New", value: "v", hidden: false, valueLoaded: true },
    ],
  };
  check("a newly added field carries its value", customFieldsReady(added.customFields));
  check(
    "and blocks Save while it is unnamed or empty",
    !customFieldsReady([
      ...d.customFields,
      { name: "", value: "v", hidden: false, valueLoaded: false },
    ]) &&
      !customFieldsReady([{ name: "New", value: "", hidden: false, valueLoaded: true }]) &&
      customFieldsReady([{ name: "Kept", value: "", hidden: true, valueLoaded: false }]),
  );
}

console.log("[draft] the expiry date round-trips through the local day");
{
  check('"" is no expiry', dateInputToEpoch("") === null);
  const at = dateInputToEpoch("2031-03-04");
  check("a date becomes local midnight", at !== null && new Date(at).getHours() === 0);
  check("and converts back to the same input value", epochToDateInput(at) === "2031-03-04");
  check("no expiry renders as an empty input", epochToDateInput(null) === "");
  check("junk is refused rather than guessed", dateInputToEpoch("04/03/2031") === null);
}

console.log("[draft] isDirty separates a real edit from a reopen");
{
  const initial = draftFromDetail(detail(), "root");
  check("the same draft is clean", !isDirty(draftFromDetail(detail(), "root"), initial));
  check("a title edit is dirty", isDirty({ ...initial, title: "Site 2" }, initial));
  check("a favourite flip is dirty", isDirty({ ...initial, favorite: true }, initial));
  check("typing a password is dirty", isDirty({ ...initial, passwordTouched: true }, initial));
  check(
    "and clearing it back to empty is still dirty, so the dialog asks",
    isDirty({ ...initial, passwordTouched: true, password: "" }, initial),
  );
  check("moving group is dirty", isDirty({ ...initial, groupId: "other" }, initial));
  check(
    "a create against its own empty draft is clean",
    !isDirty(draftFromCreate("root"), draftFromCreate("root")),
  );
}

console.log("[errors] Rust's module prefix is stripped, the sentence is finished");
{
  check(
    "a lowercase sentence is capitalised and stopped",
    describeVaultError("vault: a sibling group already has that name") ===
      "A sibling group already has that name.",
  );
  check(
    "an existing full stop is not doubled",
    describeVaultError("vault: group name is required.") === "Group name is required.",
  );
  check(
    "every module prefix is stripped",
    describeVaultError("totp: bad base32 secret") === "Bad base32 secret." &&
      describeVaultError("generator: pick at least one character set") ===
        "Pick at least one character set." &&
      describeVaultError("clipboard: no such entry") === "No such entry.",
  );
  check(
    "a message with no prefix at all still reads as a sentence",
    describeVaultError("something went wrong") === "Something went wrong.",
  );
}

console.log("[errors] the two sentences that are fixed verbatim");
{
  check(
    "the newer-vault refusal tells the reader what to do",
    describeVaultError("vault: this vault was written by a newer Subclave") ===
      "This vault was written by a newer Subclave. Update to open it.",
  );
  check(
    "the wrong-password refusal keeps its exact wording",
    describeVaultError("vault: wrong master password, or the vault file is corrupt") ===
      "Wrong master password, or the vault file is corrupt.",
  );
}

if (failed > 0) throw new Error(`vault-draft-verify: ${failed} check(s) failed`);
console.log("\nvault-draft-verify: all checks passed");
