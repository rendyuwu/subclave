//! Encrypted backups: the `.subclave-backup` export, its preview and its apply.
//!
//! A backup is a [`VaultFile`] with `format: "subclave-backup"` whose
//! ciphertext is a [`BackupPayload`] (entries and groups, nothing else) under
//! AES-256-GCM with a key derived from the backup passphrase by Argon2id. The
//! header is the associated data, as in the vault file. A preview stages the
//! decoded records in `Unlocked.staged`; the apply merges each one through
//! `merge_into_payload`, the path a sync pull lands records through.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::path::Path;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use zeroize::Zeroizing;

use crate::modules::aesgcm;
use crate::modules::fs::atomic;
use crate::modules::strength::strength_of;
use crate::modules::sync::engine::payload::{
    entry_envelope, group_envelope, local_envelope, merge_into_payload,
};
use crate::modules::sync::model::{ENTRY_KIND, GROUP_KIND};
use crate::modules::vault::events::{emit_changed, run_blocking};
use crate::modules::vault::file::{header_aad, VaultFile};
use crate::modules::vault::kdf::{derive_key, fresh_params, Argon2Params};
use crate::modules::vault::model::{Entry, Group, VaultPayload};
use crate::modules::vault::state::{commit, now_ms, VaultState, LOCKED_ERR};
use crate::modules::vault::{next_stage_handle, replace_staged, vault_dir, Staged, StagedSource};

const FORMAT: &str = "subclave-backup";
const FORMAT_VERSION: u32 = 1;
const NOT_A_BACKUP: &str = "backup: not a Subclave backup file";
/// Wrong passphrase, tampered bytes and truncation share one message, as the
/// vault file's do: telling them apart tells an attacker which guess was
/// closer.
const CORRUPT: &str = "backup: wrong passphrase, or the file is corrupt";
const GONE: &str = "backup: this preview is gone; open the file again";

/// What the ciphertext holds: entries and groups only, never `DeviceState`
/// (sync credentials, paired browsers) and never tombstones, so a restore
/// cannot delete anything. `Cow` lets the seal borrow the snapshot's records
/// instead of copying the plaintext a second time.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupPayload<'a> {
    entries: Cow<'a, [Entry]>,
    groups: Cow<'a, [Group]>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BackupPreview {
    handle: u32,
    added: usize,
    newer: usize,
    older: usize,
    same: usize,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BackupApplied {
    added: usize,
    updated: usize,
}

fn seal_backup(
    entries: &[Entry],
    groups: &[Group],
    passphrase: &str,
    kdf: &Argon2Params,
) -> Result<VaultFile, String> {
    let key = derive_key(passphrase, kdf)?;
    let json = Zeroizing::new(
        serde_json::to_vec(&BackupPayload {
            entries: Cow::Borrowed(entries),
            groups: Cow::Borrowed(groups),
        })
        .map_err(|e| format!("backup: {e}"))?,
    );
    let (nonce, ciphertext) =
        aesgcm::seal_with_key(&key, &header_aad(FORMAT, FORMAT_VERSION, kdf), &json)?;
    Ok(VaultFile {
        format: FORMAT.to_string(),
        v: FORMAT_VERSION,
        kdf: kdf.clone(),
        nonce: B64.encode(nonce),
        ciphertext: B64.encode(ciphertext),
    })
}

/// Checks in order: format, version, KDF header, then the key and the GCM
/// tag, then the payload shape.
fn open_backup(file: &VaultFile, passphrase: &str) -> Result<(Vec<Entry>, Vec<Group>), String> {
    if file.format != FORMAT {
        return Err(NOT_A_BACKUP.to_string());
    }
    if file.v > FORMAT_VERSION {
        return Err("backup: this backup was written by a newer Subclave".to_string());
    }
    // `derive_key` runs `check_params` first; a bad salt or argon2's own
    // refusal is a header fault all the same.
    let key = derive_key(passphrase, &file.kdf)
        .map_err(|_| "backup: the backup's key settings are not accepted".to_string())?;
    let nonce: [u8; 12] = B64
        .decode(&file.nonce)
        .ok()
        .and_then(|n| n.try_into().ok())
        .ok_or_else(|| CORRUPT.to_string())?;
    let ciphertext = B64
        .decode(&file.ciphertext)
        .map_err(|_| CORRUPT.to_string())?;
    let plain = aesgcm::open_with_key(
        &key,
        &header_aad(FORMAT, file.v, &file.kdf),
        &nonce,
        ciphertext,
        "backup",
    )?;
    let payload: BackupPayload = serde_json::from_slice(&plain)
        .map_err(|_| "backup: the backup file is corrupt".to_string())?;
    Ok((payload.entries.into_owned(), payload.groups.into_owned()))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    atomic::atomic_write_private(path, bytes)
        .map_err(|e| format!("backup: could not write the file: {e}"))
}

#[tauri::command]
pub async fn backup_export(app: AppHandle, path: String, passphrase: String) -> Result<(), String> {
    run_blocking(&app, move |state| {
        backup_export_inner(state, Path::new(&path), &passphrase)
    })
    .await
}

/// The passphrase rule is the sync passphrase's: refused below zxcvbn score
/// 3. The key derivation runs after the vault mutex is released, on a
/// snapshot that is wiped on every path.
pub(crate) fn backup_export_inner(
    state: &VaultState,
    path: &Path,
    passphrase: &str,
) -> Result<(), String> {
    if passphrase.is_empty() {
        return Err("backup: a passphrase is required".to_string());
    }
    let strength = strength_of(passphrase);
    if strength.score < 3 {
        return Err(match strength.warning {
            Some(warning) => format!("backup: the passphrase is too weak: {warning}"),
            None => "backup: the passphrase is too weak".to_string(),
        });
    }
    let mut snapshot = {
        let guard = state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        let mut entries = unlocked.payload.entries.clone();
        for entry in &mut entries {
            // Device-local: when this machine last filled the entry.
            entry.last_used_at = None;
        }
        VaultPayload {
            entries,
            groups: unlocked.payload.groups.clone(),
            ..Default::default()
        }
    };
    let result = fresh_params()
        .and_then(|kdf| seal_backup(&snapshot.entries, &snapshot.groups, passphrase, &kdf))
        .and_then(|file| serde_json::to_vec(&file).map_err(|e| format!("backup: {e}")))
        .and_then(|bytes| write_file(path, &bytes));
    snapshot.wipe();
    result
}

#[tauri::command]
pub async fn backup_import_preview(
    app: AppHandle,
    path: String,
    passphrase: String,
) -> Result<BackupPreview, String> {
    run_blocking(&app, move |state| {
        backup_import_preview_inner(state, Path::new(&path), &passphrase)
    })
    .await
}

/// Classify each backup entry against the vault on stamps alone: history is
/// not compared, so an entry whose history grew without a new stamp reads as
/// same. Groups are merged at apply but not counted.
pub(crate) fn backup_import_preview_inner(
    state: &VaultState,
    path: &Path,
    passphrase: &str,
) -> Result<BackupPreview, String> {
    if state.access()?.is_none() {
        return Err(LOCKED_ERR.to_string());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("backup: could not read the file: {e}"))?;
    let file: VaultFile = serde_json::from_slice(&bytes).map_err(|_| NOT_A_BACKUP.to_string())?;
    let (entries, groups) = open_backup(&file, passphrase)?;
    let mut records = VaultPayload {
        entries,
        groups,
        ..Default::default()
    };

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
    let now = now_ms();
    let mut preview = BackupPreview {
        handle: next_stage_handle(),
        added: 0,
        newer: 0,
        older: 0,
        same: 0,
    };
    for entry in &records.entries {
        match local_envelope(&unlocked.payload, ENTRY_KIND, &entry.id, now) {
            None => preview.added += 1,
            // A tie goes to the delete on apply, as in the merge.
            Some(local) if local.deleted => {
                if entry.updated_at > local.updated_at.unwrap_or(0) {
                    preview.added += 1;
                } else {
                    preview.older += 1;
                }
            }
            Some(local) => match entry.updated_at.cmp(&local.updated_at.unwrap_or(0)) {
                Ordering::Greater => preview.newer += 1,
                Ordering::Less => preview.older += 1,
                Ordering::Equal => preview.same += 1,
            },
        }
    }
    replace_staged(
        unlocked,
        Staged {
            handle: preview.handle,
            source: StagedSource::Backup,
            records: Some(records),
        },
    );
    Ok(preview)
}

#[tauri::command]
pub async fn backup_import_apply(app: AppHandle, handle: u32) -> Result<BackupApplied, String> {
    let dir = vault_dir(&app)?;
    let result = run_blocking(&app, move |state| {
        backup_import_apply_inner(state, &dir, handle)
    })
    .await;
    if result.is_ok() {
        emit_changed(&app, &[], "import");
    }
    result
}

/// Merge every staged record into the vault, groups first so entries land
/// under them. The merge keeps the newer copy current and puts the other into
/// history as `conflict`; a local tombstone newer than the backup copy wins.
/// `updated` counts live entries whose stored form changed, `added` the rest.
pub(crate) fn backup_import_apply_inner(
    state: &VaultState,
    dir: &Path,
    handle: u32,
) -> Result<BackupApplied, String> {
    state.ensure_writable()?;
    let mut guard = state.access()?;
    let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
    let mut records = match unlocked.staged.take() {
        Some(Staged {
            handle: staged,
            source: StagedSource::Backup,
            records: Some(records),
        }) if staged == handle => records,
        other => {
            unlocked.staged = other;
            return Err(GONE.to_string());
        }
    };
    let payload = &mut unlocked.payload;
    let now = now_ms();
    let mut applied = BackupApplied {
        added: 0,
        updated: 0,
    };
    // `Err` names a record this build refuses to store; unreachable for
    // records that deserialized as `Entry` and `Group`, so it is skipped.
    for group in &records.groups {
        if matches!(
            merge_into_payload(payload, &group_envelope(group), now),
            Ok(true)
        ) {
            payload.device.sync.mark_dirty(GROUP_KIND, &group.id);
        }
    }
    for entry in &records.entries {
        let was_live =
            local_envelope(payload, ENTRY_KIND, &entry.id, now).is_some_and(|l| !l.deleted);
        if matches!(
            merge_into_payload(payload, &entry_envelope(entry), now),
            Ok(true)
        ) {
            payload.device.sync.mark_dirty(ENTRY_KIND, &entry.id);
            if was_live {
                applied.updated += 1;
            } else {
                applied.added += 1;
            }
        }
    }
    records.wipe();
    drop(guard);
    commit(state, dir)?;
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::lockext::lock_or_recover;
    use crate::modules::vault::entry_commands::vault_entry_upsert_inner;
    use crate::modules::vault::model::{Tombstone, TombstoneKind, VersionReason};
    use crate::modules::vault::session::vault_create_inner;
    use crate::modules::vault::test_util::{draft, TempDir};

    const PASS: &str = "correct horse battery staple 42";

    /// Cheap parameters (8 MiB, one pass) so a test does not pay the default
    /// cost, as `seal_cheap` in the vault file tests.
    fn cheap_kdf() -> Argon2Params {
        let mut kdf = fresh_params().unwrap();
        kdf.memory_kib = 8192;
        kdf.iterations = 1;
        kdf
    }

    fn write_backup(
        dir: &TempDir,
        name: &str,
        entries: &[Entry],
        groups: &[Group],
    ) -> std::path::PathBuf {
        let file = seal_backup(entries, groups, PASS, &cheap_kdf()).unwrap();
        let path = dir.0.join(name);
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        path
    }

    fn unlocked_vault(tag: &str) -> (TempDir, VaultState) {
        let dir = TempDir::new(tag);
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        (dir, state)
    }

    fn with_payload<T>(state: &VaultState, f: impl FnOnce(&mut VaultPayload) -> T) -> T {
        f(&mut lock_or_recover(&state.inner).as_mut().unwrap().payload)
    }

    fn entry_by_id(state: &VaultState, id: &str) -> Option<Entry> {
        with_payload(state, |p| p.entries.iter().find(|e| e.id == id).cloned())
    }

    #[test]
    fn export_writes_a_sealed_file_without_device_state() {
        let (dir, state) = unlocked_vault("bkexport");
        let id = vault_entry_upsert_inner(&state, &dir.0, draft(None, "One"))
            .unwrap()
            .id;
        with_payload(&state, |p| p.entries[0].last_used_at = Some(42));

        let first = dir.0.join("a.subclave-backup");
        let second = dir.0.join("b.subclave-backup");
        backup_export_inner(&state, &first, PASS).unwrap();
        backup_export_inner(&state, &second, PASS).unwrap();
        let a: VaultFile = serde_json::from_slice(&std::fs::read(&first).unwrap()).unwrap();
        let b: VaultFile = serde_json::from_slice(&std::fs::read(&second).unwrap()).unwrap();

        assert_eq!(a.format, "subclave-backup");
        assert_eq!(a.v, 1);
        assert_eq!(
            (
                a.kdf.name.as_str(),
                a.kdf.memory_kib,
                a.kdf.iterations,
                a.kdf.parallelism
            ),
            ("argon2id", 65536, 3, 4)
        );
        assert_ne!(a.kdf.salt, b.kdf.salt);
        assert_ne!(a.nonce, b.nonce);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&first).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // Decrypt by hand to see the payload's exact shape.
        let key = derive_key(PASS, &a.kdf).unwrap();
        let nonce: [u8; 12] = B64.decode(&a.nonce).unwrap().try_into().unwrap();
        let plain = aesgcm::open_with_key(
            &key,
            &header_aad(FORMAT, a.v, &a.kdf),
            &nonce,
            B64.decode(&a.ciphertext).unwrap(),
            "test",
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        let mut keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["entries", "groups"]);
        let entries = json["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["id"], id.as_str());
        assert!(entries
            .iter()
            .all(|e| e.get("lastUsedAt").is_none_or(|v| v.is_null())));
        assert!(!json["groups"].as_array().unwrap().is_empty());
    }

    #[test]
    fn export_refuses_a_missing_or_weak_passphrase() {
        let (dir, state) = unlocked_vault("bkweak");
        let path = dir.0.join("weak.subclave-backup");
        assert_eq!(
            backup_export_inner(&state, &path, "").unwrap_err(),
            "backup: a passphrase is required"
        );
        let err = backup_export_inner(&state, &path, "abc").unwrap_err();
        assert!(
            err.starts_with("backup: the passphrase is too weak"),
            "{err}"
        );
        assert!(!path.exists());

        let locked = VaultState::default();
        assert_eq!(
            backup_import_preview_inner(&locked, &path, PASS).unwrap_err(),
            LOCKED_ERR
        );
    }

    #[test]
    fn open_checks_the_header_and_the_passphrase() {
        let file = seal_backup(&[], &[], PASS, &cheap_kdf()).unwrap();
        assert!(open_backup(&file, PASS).is_ok());
        assert_eq!(
            open_backup(&file, "not the passphrase").unwrap_err(),
            CORRUPT
        );

        // v = 0 passes the version check and derives the same key, so only
        // the associated data differs: the header is bound.
        let mut flipped = file.clone();
        flipped.v = 0;
        assert_eq!(open_backup(&flipped, PASS).unwrap_err(), CORRUPT);

        let mut other = file.clone();
        other.format = "subclave-vault".into();
        assert_eq!(open_backup(&other, PASS).unwrap_err(), NOT_A_BACKUP);
        let mut newer = file.clone();
        newer.v = 2;
        assert_eq!(
            open_backup(&newer, PASS).unwrap_err(),
            "backup: this backup was written by a newer Subclave"
        );
        let mut greedy = file.clone();
        greedy.kdf.memory_kib = 1 << 30;
        assert_eq!(
            open_backup(&greedy, PASS).unwrap_err(),
            "backup: the backup's key settings are not accepted"
        );
    }

    /// One vault holding every class the preview names, then the apply over
    /// it: an older tombstone wins, a newer backup copy revives the entry.
    #[test]
    fn preview_classifies_on_stamps_and_apply_merges() {
        let (dir, state) = unlocked_vault("bkclass");
        for title in ["Same", "Newer", "Newer too", "Older"] {
            vault_entry_upsert_inner(&state, &dir.0, draft(None, title)).unwrap();
        }
        let local = with_payload(&state, |p| p.entries.clone());
        let now = now_ms();
        let mut backup = local.clone();
        backup[1].updated_at += 10;
        backup[1].title = "Newer from backup".into();
        backup[2].updated_at += 10;
        backup[3].updated_at -= 10;
        backup[3].title = "Older from backup".into();
        let mut fresh = local[0].clone();
        fresh.id = "fresh".into();
        let mut buried = local[0].clone();
        buried.id = "buried".into();
        buried.updated_at = now - 2_000;
        let mut revived = local[0].clone();
        revived.id = "revived".into();
        revived.updated_at = now;
        backup.extend([fresh, buried, revived]);
        with_payload(&state, |p| {
            for id in ["buried", "revived"] {
                p.tombstones.push(Tombstone {
                    id: id.into(),
                    kind: TombstoneKind::Entry,
                    deleted_at: now - 1_000,
                });
            }
            p.device.sync.dirty.clear();
        });
        let path = write_backup(&dir, "class.subclave-backup", &backup, &[]);

        let preview = backup_import_preview_inner(&state, &path, PASS).unwrap();
        assert_eq!(
            (preview.added, preview.newer, preview.older, preview.same),
            (2, 2, 2, 1)
        );
        assert_eq!(
            backup_import_apply_inner(&state, &dir.0, preview.handle + 1).unwrap_err(),
            GONE
        );

        let applied = backup_import_apply_inner(&state, &dir.0, preview.handle).unwrap();
        // The older live copy changed the stored form too: it went into
        // history.
        assert_eq!((applied.added, applied.updated), (2, 3));
        assert!(
            entry_by_id(&state, "buried").is_none(),
            "an older copy undid a delete"
        );
        assert!(with_payload(&state, |p| p
            .tombstones
            .iter()
            .any(|t| t.id == "buried")));
        assert!(entry_by_id(&state, "revived").is_some());
        assert!(entry_by_id(&state, "fresh").is_some());
        let dirty = with_payload(&state, |p| p.device.sync.dirty.clone());
        assert!(!dirty.contains(&format!("entry:{}", local[0].id)));
        assert!(dirty.contains("entry:fresh"));
        assert_eq!(
            backup_import_apply_inner(&state, &dir.0, preview.handle).unwrap_err(),
            GONE
        );
    }

    #[test]
    fn a_divergent_copy_lands_in_history_as_conflict() {
        let (dir, state) = unlocked_vault("bkdiverge");
        let older_id = vault_entry_upsert_inner(&state, &dir.0, draft(None, "Kept"))
            .unwrap()
            .id;
        let newer_id = vault_entry_upsert_inner(&state, &dir.0, draft(None, "Replaced"))
            .unwrap()
            .id;
        let local_older = entry_by_id(&state, &older_id).unwrap();
        let local_newer = entry_by_id(&state, &newer_id).unwrap();
        with_payload(&state, |p| p.device.sync.dirty.clear());

        let mut stale = local_older.clone();
        stale.title = "Stale".into();
        stale.updated_at -= 5;
        stale.history.clear();
        let mut ahead = local_newer.clone();
        ahead.title = "Ahead".into();
        ahead.updated_at += 5;
        ahead.history.clear();
        let path = write_backup(
            &dir,
            "diverge.subclave-backup",
            &[stale.clone(), ahead.clone()],
            &[],
        );

        let preview = backup_import_preview_inner(&state, &path, PASS).unwrap();
        assert_eq!((preview.newer, preview.older), (1, 1));
        let applied = backup_import_apply_inner(&state, &dir.0, preview.handle).unwrap();
        assert_eq!((applied.added, applied.updated), (0, 2));

        let kept = entry_by_id(&state, &older_id).unwrap();
        assert_eq!(kept.title, "Kept");
        assert_eq!(kept.updated_at, local_older.updated_at);
        assert!(kept.history.iter().any(|v| v.title == "Stale"
            && v.updated_at == stale.updated_at
            && v.reason == VersionReason::Conflict));

        let replaced = entry_by_id(&state, &newer_id).unwrap();
        assert_eq!(replaced.title, "Ahead");
        assert_eq!(replaced.updated_at, ahead.updated_at);
        assert!(replaced.history.iter().any(|v| v.title == "Replaced"
            && v.updated_at == local_newer.updated_at
            && v.reason == VersionReason::Conflict));

        let dirty = with_payload(&state, |p| p.device.sync.dirty.clone());
        assert!(dirty.contains(&format!("entry:{older_id}")));
        assert!(dirty.contains(&format!("entry:{newer_id}")));
    }

    /// Export, edit, import: the backup copy is the version the edit already
    /// pushed into history, so nothing changes.
    #[test]
    fn restoring_a_backup_older_than_an_edit_changes_nothing() {
        let (dir, state) = unlocked_vault("bknatural");
        let id = vault_entry_upsert_inner(&state, &dir.0, draft(None, "Before"))
            .unwrap()
            .id;
        let path = dir.0.join("natural.subclave-backup");
        backup_export_inner(&state, &path, PASS).unwrap();
        vault_entry_upsert_inner(&state, &dir.0, draft(Some(id.clone()), "After")).unwrap();
        let edited = entry_by_id(&state, &id).unwrap();
        assert_eq!(edited.history.len(), 1);

        let preview = backup_import_preview_inner(&state, &path, PASS).unwrap();
        assert_eq!(
            (preview.added, preview.newer, preview.older, preview.same),
            (0, 0, 1, 0)
        );
        let applied = backup_import_apply_inner(&state, &dir.0, preview.handle).unwrap();
        assert_eq!((applied.added, applied.updated), (0, 0));
        assert_eq!(entry_by_id(&state, &id).unwrap(), edited);
    }
}
