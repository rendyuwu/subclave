//! CSV import with a preview and an apply, the delete of the imported file,
//! and plaintext CSV export.
//!
//! A preview parses the file and stages its entries in `Unlocked.staged`
//! (wiped on every lock); the webview gets titles, hosts and usernames only,
//! and the apply lands the ticked rows in one new `Imported <date>` group.

mod export;
mod formats;

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use serde::Serialize;
use tauri::AppHandle;
use zeroize::Zeroizing;

pub use formats::CsvFormat;

use crate::modules::vault::events::{emit_changed, run_blocking};
use crate::modules::vault::model::{host_of, Entry, Group, VaultPayload, ROOT_ID};
use crate::modules::vault::query::in_trash;
use crate::modules::vault::state::{now_ms, VaultState, LOCKED_ERR};
use crate::modules::vault::{
    commit, next_stage_handle, replace_staged, vault_dir, Staged, StagedSource,
};

const GONE_ERR: &str = "import: this preview is gone; pick the file again";

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRow {
    /// The 0-based record index, the selection key for the apply.
    row: usize,
    title: String,
    host: Option<String>,
    username: String,
    problem: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    handle: u32,
    format: CsvFormat,
    rows: Vec<PreviewRow>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportApplied {
    added: usize,
}

fn first_host(entry: &Entry) -> Option<String> {
    entry.urls.first().and_then(|u| host_of(&u.url))
}

#[tauri::command]
pub async fn import_csv_preview(
    app: AppHandle,
    path: String,
    format: CsvFormat,
) -> Result<ImportPreview, String> {
    run_blocking(&app, move |state| {
        import_csv_preview_inner(state, Path::new(&path), format)
    })
    .await
}

pub(crate) fn import_csv_preview_inner(
    state: &VaultState,
    path: &Path,
    format: CsvFormat,
) -> Result<ImportPreview, String> {
    if state.access()?.is_none() {
        return Err(LOCKED_ERR.to_string());
    }
    // Read and parse outside the vault mutex.
    let bytes = Zeroizing::new(
        std::fs::read(path).map_err(|e| format!("import: could not read the file: {e}"))?,
    );
    let (format, parsed) = formats::parse_csv(&bytes, format)?;
    let recycled: Vec<bool> = parsed.iter().map(|p| p.recycled).collect();
    let mut records = VaultPayload {
        entries: parsed.into_iter().map(|p| p.entry).collect(),
        ..Default::default()
    };

    // The vault can lock while the file is parsed; the rows are plaintext,
    // so they are wiped on that path too, as every lock wipes a staged one.
    let mut guard = match state.access() {
        Ok(guard) => guard,
        Err(e) => {
            records.wipe();
            return Err(e);
        }
    };
    let Some(unlocked) = guard.as_mut() else {
        records.wipe();
        return Err(LOCKED_ERR.to_string());
    };
    let payload = &unlocked.payload;
    let existing: HashSet<(Option<String>, &str, &str)> = payload
        .entries
        .iter()
        .filter(|e| !in_trash(payload, &e.group_id))
        .map(|e| (first_host(e), e.username.as_str(), e.password.as_str()))
        .collect();
    let rows = records
        .entries
        .iter()
        .zip(&recycled)
        .enumerate()
        .map(|(row, (entry, &recycled))| {
            let host = first_host(entry);
            let problem = if entry.title.is_empty() && entry.urls.is_empty() {
                Some("No title or URL.")
            } else if recycled {
                Some("In the KeePassXC Recycle Bin.")
            } else if existing.contains(&(
                host.clone(),
                entry.username.as_str(),
                entry.password.as_str(),
            )) {
                Some("Already in the vault.")
            } else {
                None
            };
            PreviewRow {
                row,
                title: entry.title.clone(),
                host,
                username: entry.username.clone(),
                problem: problem.map(str::to_string),
            }
        })
        .collect();

    let handle = next_stage_handle();
    replace_staged(
        unlocked,
        Staged {
            handle,
            source: StagedSource::Csv(path.to_path_buf()),
            records: Some(records),
        },
    );
    Ok(ImportPreview {
        handle,
        format,
        rows,
    })
}

#[tauri::command]
pub async fn import_apply(
    app: AppHandle,
    handle: u32,
    rows: Vec<usize>,
) -> Result<ImportApplied, String> {
    let dir = vault_dir(&app)?;
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let result = run_blocking(&app, move |state| {
        import_apply_inner(state, &dir, handle, &rows, &date)
    })
    .await;
    if result.is_ok() {
        emit_changed(&app, &[], "import");
    }
    result
}

/// Land the selected rows of the staged preview in a new root group named
/// `Imported <date>` (suffixed ` (2)`, ` (3)`, ... on a clash), then wipe the
/// rest. The CSV path stays staged for [`import_delete_csv_inner`].
pub(crate) fn import_apply_inner(
    state: &VaultState,
    dir: &Path,
    handle: u32,
    rows: &[usize],
    date: &str,
) -> Result<ImportApplied, String> {
    state.ensure_writable()?;
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let staged = match unlocked.staged.as_mut() {
        Some(s) if s.handle == handle && matches!(s.source, StagedSource::Csv(_)) => s,
        _ => return Err(GONE_ERR.to_string()),
    };
    let records = staged
        .records
        .as_mut()
        .ok_or_else(|| "import: these rows were already imported".to_string())?;
    if rows.is_empty() {
        return Err("import: no rows selected".to_string());
    }
    let selected: BTreeSet<usize> = rows.iter().copied().collect();
    if selected.last().is_some_and(|&i| i >= records.entries.len()) {
        return Err("import: no such row".to_string());
    }

    let payload = &mut unlocked.payload;
    let now = now_ms();
    let base = format!("Imported {date}");
    let mut name = base.clone();
    let mut n = 2;
    while payload.groups.iter().any(|g| {
        g.parent_id.as_deref().unwrap_or(ROOT_ID) == ROOT_ID && g.name.eq_ignore_ascii_case(&name)
    }) {
        name = format!("{base} ({n})");
        n += 1;
    }
    let group_id = uuid::Uuid::new_v4().to_string();
    payload.groups.push(Group {
        id: group_id.clone(),
        parent_id: Some(ROOT_ID.to_string()),
        name,
        icon: None,
        color: None,
        created_at: now,
        updated_at: now,
    });
    payload.device.sync.mark_dirty("group", &group_id);

    let mut rest = VaultPayload::default();
    for (i, mut entry) in std::mem::take(&mut records.entries).into_iter().enumerate() {
        if !selected.contains(&i) {
            rest.entries.push(entry);
            continue;
        }
        entry.group_id = group_id.clone();
        entry.created_at = now;
        entry.updated_at = now;
        payload.device.sync.mark_dirty("entry", &entry.id);
        payload.entries.push(entry);
    }
    rest.wipe();
    staged.records = None;
    drop(guard);
    commit(state, dir)?;
    Ok(ImportApplied {
        added: selected.len(),
    })
}

#[tauri::command]
pub async fn import_delete_csv(app: AppHandle, handle: u32) -> Result<(), String> {
    run_blocking(&app, move |state| import_delete_csv_inner(state, handle)).await
}

/// Delete the file behind an applied preview. Only a file that previewed as
/// a supported CSV and was imported can be deleted this way.
pub(crate) fn import_delete_csv_inner(state: &VaultState, handle: u32) -> Result<(), String> {
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let path = match &unlocked.staged {
        Some(Staged {
            handle: h,
            source: StagedSource::Csv(path),
            records,
        }) if *h == handle => {
            if records.is_some() {
                return Err("import: import the rows before deleting the file".to_string());
            }
            path
        }
        _ => return Err(GONE_ERR.to_string()),
    };
    std::fs::remove_file(path).map_err(|e| format!("import: could not delete the file: {e}"))?;
    unlocked.staged = None;
    Ok(())
}

#[tauri::command]
pub async fn export_csv(app: AppHandle, path: String) -> Result<(), String> {
    run_blocking(&app, move |state| export_csv_inner(state, Path::new(&path))).await
}

pub(crate) fn export_csv_inner(state: &VaultState, path: &Path) -> Result<(), String> {
    let bytes = {
        let guard = state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        export::csv_bytes(&unlocked.payload)?
    };
    crate::modules::fs::atomic::atomic_write_private(path, &bytes)
        .map_err(|e| format!("import: could not write the file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::modules::vault::entry_commands::{
        vault_entry_trash_inner, vault_entry_upsert_inner,
    };
    use crate::modules::vault::group_commands::vault_group_upsert_inner;
    use crate::modules::vault::model::{CustomField, EntryUrl, MatchMode};
    use crate::modules::vault::session::{vault_create_inner, vault_unlock_inner};
    use crate::modules::vault::test_util::{draft, group_draft, TempDir};

    const DATE: &str = "2026-10-02";

    /// Row 0 a normal login, row 1 no title and no URL, row 2 from the Recycle
    /// Bin, row 3 the vault's `user` / `pw-1` login on `dup.example.com`.
    const KEEPASSXC: &str =
        "\"Group\",\"Title\",\"Username\",\"Password\",\"URL\",\"Notes\",\"TOTP\"\n\
\"Root\",\"Mail\",\"alice\",\"pw-a\",\"https://mail.example.com\",\"\",\"\"\n\
\"Root\",\"\",\"bob\",\"pw-b\",\"\",\"\",\"\"\n\
\"Root/Recycle Bin\",\"Old\",\"carol\",\"pw-c\",\"https://old.example.com\",\"\",\"\"\n\
\"Root\",\"Dup\",\"user\",\"pw-1\",\"https://dup.example.com/login\",\"\",\"\"\n";

    fn vault(tag: &str) -> (TempDir, VaultState) {
        let dir = TempDir::new(tag);
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        (dir, state)
    }

    fn csv_file(dir: &TempDir, text: &str) -> std::path::PathBuf {
        let path = dir.0.join("import.csv");
        std::fs::write(&path, text).unwrap();
        path
    }

    fn with_url(title: &str, url: &str) -> crate::modules::vault::model::EntryDraft {
        let mut d = draft(None, title);
        d.urls = vec![EntryUrl {
            url: url.into(),
            match_mode: MatchMode::Domain,
        }];
        d
    }

    fn problems(preview: &ImportPreview) -> Vec<Option<&str>> {
        preview.rows.iter().map(|r| r.problem.as_deref()).collect()
    }

    fn payload_of<T>(state: &VaultState, f: impl FnOnce(&VaultPayload) -> T) -> T {
        f(&state.access().unwrap().as_ref().unwrap().payload)
    }

    #[test]
    fn preview_flags_problem_rows_and_ignores_duplicates_in_trash() {
        let (dir, state) = vault("imp-preview");
        vault_entry_upsert_inner(&state, &dir.0, with_url("Dup", "https://DUP.example.com"))
            .unwrap();
        // The same login sitting in Trash does not count as a duplicate.
        let trashed =
            vault_entry_upsert_inner(&state, &dir.0, with_url("Old", "https://old.example.com"))
                .unwrap();
        vault_entry_trash_inner(&state, &dir.0, vec![trashed.id]).unwrap();
        let path = csv_file(&dir, KEEPASSXC);
        let preview = import_csv_preview_inner(&state, &path, CsvFormat::Auto).unwrap();
        assert_eq!(preview.format, CsvFormat::Keepassxc);
        assert_eq!(
            problems(&preview),
            [
                None,
                Some("No title or URL."),
                Some("In the KeePassXC Recycle Bin."),
                Some("Already in the vault."),
            ]
        );
        assert_eq!(preview.rows[0].host.as_deref(), Some("mail.example.com"));

        // A Trash duplicate alone flags nothing.
        let alone = "\"Title\",\"Username\",\"Password\",\"URL\"\n\"Old\",\"user\",\"pw-1\",\"https://old.example.com\"\n";
        let preview =
            import_csv_preview_inner(&state, &csv_file(&dir, alone), CsvFormat::Auto).unwrap();
        assert_eq!(problems(&preview), [None]);
    }

    #[test]
    fn apply_lands_selected_rows_in_a_dated_group_and_marks_them_dirty() {
        let (dir, state) = vault("imp-apply");
        let path = csv_file(&dir, KEEPASSXC);
        let preview = import_csv_preview_inner(&state, &path, CsvFormat::Auto).unwrap();
        let applied = import_apply_inner(&state, &dir.0, preview.handle, &[3, 0, 0], DATE).unwrap();
        assert_eq!(applied.added, 2);
        payload_of(&state, |p| {
            let group = p
                .groups
                .iter()
                .find(|g| g.name == "Imported 2026-10-02")
                .unwrap();
            assert_eq!(group.parent_id.as_deref(), Some(ROOT_ID));
            let imported: Vec<&Entry> = p
                .entries
                .iter()
                .filter(|e| e.group_id == group.id)
                .collect();
            assert_eq!(
                imported
                    .iter()
                    .map(|e| e.title.as_str())
                    .collect::<Vec<_>>(),
                ["Mail", "Dup"]
            );
            assert!(imported
                .iter()
                .all(|e| e.created_at > 0 && e.updated_at > 0));
            assert!(p.device.sync.dirty.contains(&format!("group:{}", group.id)));
            for e in &imported {
                assert!(p.device.sync.dirty.contains(&format!("entry:{}", e.id)));
            }
        });

        // A second apply of the same preview is refused.
        assert_eq!(
            import_apply_inner(&state, &dir.0, preview.handle, &[1], DATE).unwrap_err(),
            "import: these rows were already imported"
        );

        // The same date again gets a suffix.
        let again = import_csv_preview_inner(&state, &path, CsvFormat::Auto).unwrap();
        import_apply_inner(&state, &dir.0, again.handle, &[0], DATE).unwrap();
        payload_of(&state, |p| {
            assert!(p.groups.iter().any(|g| g.name == "Imported 2026-10-02 (2)"));
        });
    }

    #[test]
    fn apply_refusals() {
        let (dir, state) = vault("imp-refuse");
        let path = csv_file(&dir, KEEPASSXC);
        let first = import_csv_preview_inner(&state, &path, CsvFormat::Auto).unwrap();
        let second = import_csv_preview_inner(&state, &path, CsvFormat::Auto).unwrap();
        // The first preview was replaced.
        assert_eq!(
            import_apply_inner(&state, &dir.0, first.handle, &[0], DATE).unwrap_err(),
            GONE_ERR
        );
        assert_eq!(
            import_apply_inner(&state, &dir.0, second.handle, &[], DATE).unwrap_err(),
            "import: no rows selected"
        );
        assert_eq!(
            import_apply_inner(&state, &dir.0, second.handle, &[0, 4], DATE).unwrap_err(),
            "import: no such row"
        );

        // A lock drops the preview; a fresh unlock does not bring it back.
        state.lock_inner();
        vault_unlock_inner(&state, &dir.0, "master-pw").unwrap();
        assert_eq!(
            import_apply_inner(&state, &dir.0, second.handle, &[0], DATE).unwrap_err(),
            GONE_ERR
        );
    }

    #[test]
    fn delete_needs_an_applied_preview() {
        let (dir, state) = vault("imp-delete");
        let path = csv_file(&dir, KEEPASSXC);
        let preview = import_csv_preview_inner(&state, &path, CsvFormat::Auto).unwrap();
        assert_eq!(
            import_delete_csv_inner(&state, preview.handle).unwrap_err(),
            "import: import the rows before deleting the file"
        );
        assert!(path.exists());
        import_apply_inner(&state, &dir.0, preview.handle, &[0], DATE).unwrap();
        import_delete_csv_inner(&state, preview.handle).unwrap();
        assert!(!path.exists());
        assert_eq!(
            import_delete_csv_inner(&state, preview.handle).unwrap_err(),
            GONE_ERR
        );
    }

    #[test]
    fn csv_export_writes_folders_skips_trash_and_reimports_as_bitwarden() {
        let (dir, state) = vault("imp-export");
        let work =
            vault_group_upsert_inner(&state, &dir.0, group_draft(None, None, "Work")).unwrap();
        let email = vault_group_upsert_inner(
            &state,
            &dir.0,
            group_draft(None, Some(work.id.clone()), "Email"),
        )
        .unwrap();
        let mut d = with_url("Mail", "https://mail.example.com");
        d.urls.push(EntryUrl {
            url: "https://webmail.example.com".into(),
            match_mode: MatchMode::Domain,
        });
        d.group_id = email.id.clone();
        d.notes = "line one\nline two".into();
        d.favorite = true;
        d.totp = Some(Some(
            "otpauth://totp/Subclave?secret=JBSWY3DPEHPK3PXP".into(),
        ));
        d.custom_fields = vec![crate::modules::vault::model::DraftCustomField {
            name: "PIN".into(),
            value: Some(Some("1234".into())),
            hidden: false,
        }];
        vault_entry_upsert_inner(&state, &dir.0, d).unwrap();
        let gone = vault_entry_upsert_inner(&state, &dir.0, draft(None, "Gone")).unwrap();
        vault_entry_trash_inner(&state, &dir.0, vec![gone.id]).unwrap();

        let path = dir.0.join("export.csv");
        export_csv_inner(&state, &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "a plaintext export is private");
        }
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some("folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp")
        );
        assert!(lines
            .next()
            .unwrap()
            .starts_with("Work/Email,1,login,Mail,"));
        assert!(!text.contains("Gone"));

        let preview = import_csv_preview_inner(&state, &path, CsvFormat::Auto).unwrap();
        assert_eq!(preview.format, CsvFormat::Bitwarden);
        assert_eq!(preview.rows.len(), 1);
        let guard = state.access().unwrap();
        let staged = guard.as_ref().unwrap().staged.as_ref().unwrap();
        let e = &staged.records.as_ref().unwrap().entries[0];
        assert_eq!(
            (e.title.as_str(), e.username.as_str(), e.password.as_str()),
            ("Mail", "user", "pw-1")
        );
        assert_eq!(
            e.urls.iter().map(|u| u.url.as_str()).collect::<Vec<_>>(),
            ["https://mail.example.com", "https://webmail.example.com"]
        );
        assert_eq!(e.notes, "line one\nline two");
        assert_eq!(
            e.totp.as_deref(),
            Some("otpauth://totp/Subclave?secret=JBSWY3DPEHPK3PXP")
        );
        assert!(e.favorite);
        assert_eq!(
            e.custom_fields,
            [CustomField {
                name: "PIN".into(),
                value: "1234".into(),
                hidden: true
            }]
        );
    }
}
