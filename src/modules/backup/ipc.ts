import { invoke } from "@tauri-apps/api/core";

// One typed wrapper per import, export and backup command, so every command
// literal lives here and nowhere else.
//
// Two scanner rules bind this file. Every `invoke("<name>"` literal must sit on
// the same source line as the call, because `scripts/command-registry-verify.ts`
// reads a call site line by line. No command name may be computed, because the
// same script treats a dynamic name as an unenumerated site.
// Argument keys are camelCase: Tauri maps them to the Rust snake_case
// parameters by default.

export type CsvFormat = "auto" | "keepassxc" | "bitwarden" | "chrome" | "firefox";
export type DetectedCsvFormat = Exclude<CsvFormat, "auto">;

/** One CSV record as the preview shows it. `row` is the selection key. */
export type ImportRow = {
  row: number;
  title: string;
  host: string | null;
  username: string;
  /** Why the row starts unticked, or `null` for a clean row. */
  problem: string | null;
};

export type ImportPreview = { handle: number; format: DetectedCsvFormat; rows: ImportRow[] };
export type ImportApplied = { added: number };
export type BackupPreview = {
  handle: number;
  added: number;
  newer: number;
  older: number;
  same: number;
};
export type BackupApplied = { added: number; updated: number };

export function importCsvPreview(path: string, format: CsvFormat): Promise<ImportPreview> {
  return invoke<ImportPreview>("import_csv_preview", { path, format });
}

export function importApply(handle: number, rows: number[]): Promise<ImportApplied> {
  return invoke<ImportApplied>("import_apply", { handle, rows });
}

export function importDeleteCsv(handle: number): Promise<void> {
  return invoke("import_delete_csv", { handle });
}

export function exportCsv(path: string): Promise<void> {
  return invoke("export_csv", { path });
}

export function backupExport(path: string, passphrase: string): Promise<void> {
  return invoke("backup_export", { path, passphrase });
}

export function backupImportPreview(path: string, passphrase: string): Promise<BackupPreview> {
  return invoke<BackupPreview>("backup_import_preview", { path, passphrase });
}

export function backupImportApply(handle: number): Promise<BackupApplied> {
  return invoke<BackupApplied>("backup_import_apply", { handle });
}
