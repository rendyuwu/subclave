import { invoke } from "@tauri-apps/api/core";
import type {
  ClearResult,
  EntryDetail,
  EntryDraft,
  EntrySummary,
  GeneratorOptions,
  Group,
  GroupDraft,
  RevealField,
  Strength,
  TotpCode,
  VaultStatus,
} from "./types";

// One typed wrapper per vault command, so every command literal lives here and
// nowhere else.
//
// Two scanner rules bind this file. Every `invoke("<name>"` literal must sit on
// the same source line as the call, because `scripts/command-registry-verify.ts`
// reads a call site line by line. No command name may be computed, because the
// same script treats a dynamic name as an unenumerated site.
// Argument keys are camelCase: Tauri maps them to the Rust snake_case
// parameters by default.

export function vaultStatus(): Promise<VaultStatus> {
  return invoke<VaultStatus>("vault_status");
}

export function vaultCreate(masterPassword: string): Promise<void> {
  return invoke("vault_create", { masterPassword });
}

export function vaultUnlock(masterPassword: string): Promise<void> {
  return invoke("vault_unlock", { masterPassword });
}

export function vaultLock(): Promise<void> {
  return invoke("vault_lock");
}

export function vaultTouch(): Promise<void> {
  return invoke("vault_touch");
}

export function vaultChangeMaster(current: string, next: string): Promise<void> {
  return invoke("vault_change_master", { current, next });
}

export function vaultRetrySave(): Promise<void> {
  return invoke("vault_retry_save");
}

export function vaultRestoreSnapshot(): Promise<void> {
  return invoke("vault_restore_snapshot");
}

export function vaultList(): Promise<{ entries: EntrySummary[]; groups: Group[] }> {
  return invoke<{ entries: EntrySummary[]; groups: Group[] }>("vault_list");
}

export function vaultSearch(query: string): Promise<string[]> {
  return invoke<string[]>("vault_search", { query });
}

export function vaultEntryGet(id: string): Promise<EntryDetail> {
  return invoke<EntryDetail>("vault_entry_get", { id });
}

export function vaultEntryReveal(id: string, field: RevealField): Promise<string> {
  return invoke<string>("vault_entry_reveal", { id, field });
}

export function vaultEntryUpsert(draft: EntryDraft): Promise<EntrySummary> {
  return invoke<EntrySummary>("vault_entry_upsert", { draft });
}

export function vaultEntryMove(ids: string[], groupId: string): Promise<void> {
  return invoke("vault_entry_move", { ids, groupId });
}

export function vaultEntryTrash(ids: string[]): Promise<void> {
  return invoke("vault_entry_trash", { ids });
}

export function vaultEntryRestore(ids: string[]): Promise<void> {
  return invoke("vault_entry_restore", { ids });
}

export function vaultEntryDelete(ids: string[]): Promise<void> {
  return invoke("vault_entry_delete", { ids });
}

export function vaultEntryRestoreVersion(id: string, updatedAt: number): Promise<EntrySummary> {
  return invoke<EntrySummary>("vault_entry_restore_version", { id, updatedAt });
}

export function vaultGroupUpsert(group: GroupDraft): Promise<Group> {
  return invoke<Group>("vault_group_upsert", { group });
}

export function vaultGroupDelete(id: string): Promise<void> {
  return invoke("vault_group_delete", { id });
}

export function clipCopyField(id: string, field: string): Promise<ClearResult> {
  return invoke<ClearResult>("clip_copy_field", { id, field });
}

export function totpCode(id: string): Promise<TotpCode> {
  return invoke<TotpCode>("totp_code", { id });
}

export function totpPreview(uri: string): Promise<TotpCode> {
  return invoke<TotpCode>("totp_preview", { uri });
}

export function genPassword(options: GeneratorOptions): Promise<string> {
  return invoke<string>("gen_password", { options });
}

export function genStrength(password: string): Promise<Strength> {
  return invoke<Strength>("gen_strength", { password });
}

export function quitSubclave(): Promise<void> {
  return invoke("quit_subclave");
}
