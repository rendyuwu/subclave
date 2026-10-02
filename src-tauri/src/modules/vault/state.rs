//! Vault state: the unlocked payload, the derived key and KDF parameters, the
//! parked seal after a failed save, the idle-lock deadline and the save
//! machinery that keeps the disk and the parked seal in agreement.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::modules::lockext::lock_or_recover;
use zeroize::Zeroizing;

use crate::modules::vault::events::LockReason;
use crate::modules::vault::file::{save_vault, seal_payload, VaultFile};
use crate::modules::vault::kdf::Argon2Params;
use crate::modules::vault::lock;
use crate::modules::vault::lock::{deadline_after, deadline_passed};
use crate::modules::vault::model::VaultPayload;

pub const LOCKED_ERR: &str = "vault: locked";

/// Where a staged preview came from. A CSV source keeps its path for the
/// delete that follows an import.
pub(crate) enum StagedSource {
    Csv(PathBuf),
    Backup,
}

/// Records a preview decoded and holds for its apply (a CSV file's entries,
/// or a backup's entries and groups). Plaintext, so it lives inside
/// [`Unlocked`] and is wiped with the payload on every lock. `records` is
/// `None` once applied.
pub(crate) struct Staged {
    pub(crate) handle: u32,
    pub(crate) source: StagedSource,
    pub(crate) records: Option<VaultPayload>,
}

static NEXT_STAGE_HANDLE: AtomicU32 = AtomicU32::new(1);

/// A handle no earlier preview in this process used, so an apply against a
/// preview that was replaced, or dropped by a lock, is refused.
pub(crate) fn next_stage_handle() -> u32 {
    NEXT_STAGE_HANDLE.fetch_add(1, Ordering::Relaxed)
}

/// Everything held while the vault is unlocked.
pub struct Unlocked {
    pub(crate) payload: VaultPayload,
    pub(crate) key: Zeroizing<[u8; 32]>,
    pub(crate) kdf: Argon2Params,
    /// The one staged import or backup preview, if any.
    pub(crate) staged: Option<Staged>,
}

impl Unlocked {
    /// Scrub the payload and any staged preview. Every lock path calls this
    /// before it drops the session.
    pub(crate) fn wipe(&mut self) {
        self.payload.wipe();
        if let Some(records) = self.staged.as_mut().and_then(|s| s.records.as_mut()) {
            records.wipe();
        }
    }
}

/// Store a new preview, wiping the one it replaces: one staged preview at a
/// time, of either kind.
pub(crate) fn replace_staged(unlocked: &mut Unlocked, staged: Staged) {
    if let Some(records) = unlocked.staged.as_mut().and_then(|s| s.records.as_mut()) {
        records.wipe();
    }
    unlocked.staged = Some(staged);
}

/// Managed state. See the field docs for the invariants the save machinery
/// relies on.
pub struct VaultState {
    /// Payload, key and KDF while unlocked. Wiped, not merely dropped, on
    /// every lock path.
    pub(crate) inner: Mutex<Option<Unlocked>>,
    /// Sealed bytes parked after a failed write. Ciphertext only, so the
    /// retry works while locked.
    pub(crate) pending: Mutex<Option<VaultFile>>,
    /// Serializes seal, write and pending updates, so a stale pending write can
    /// never land after a newer one: the newest `perform_save_locked` always
    /// overwrites `pending`, and an older write cannot land after a newer one
    /// because the whole seal-then-write pair is one critical section.
    pub(crate) save_lock: Mutex<()>,
    /// Idle deadline in boot-clock ms; `u64::MAX` = never.
    pub(crate) deadline_ms: AtomicU64,
    /// Set when the payload was OPENED from the `.bak` (the primary was
    /// unreadable, or failed to open): the primary is broken and must never be
    /// silently overwritten. A stale disk copy after a failed write is not
    /// this flag; the next commit simply lands. Clearing this flag is the
    /// restore flow's job.
    pub(crate) save_blocked: AtomicBool,
    /// Set by an automatic lock (the idle tick, or an expired deadline hit
    /// inside [`VaultState::access`]); drained by the next command shell to
    /// emit the event.
    auto_lock_event: Mutex<Option<LockReason>>,
    /// The outcome of the last save attempt, recorded by
    /// [`perform_save_locked`] and drained by the shells into
    /// `subclave:vault-save-failed`. Both the
    /// failure and the success are events: the banner shows the reason and
    /// clears on the next good write.
    save_event: Mutex<Option<SaveOutcome>>,
}

/// The outcome of one save attempt, for the `subclave:vault-save-failed`
/// event (`{ reason } | null`).
#[derive(Clone, Debug)]
pub(crate) enum SaveOutcome {
    Failed(String),
    Succeeded,
}

impl Default for VaultState {
    fn default() -> Self {
        Self {
            inner: Mutex::new(None),
            pending: Mutex::new(None),
            save_lock: Mutex::new(()),
            deadline_ms: AtomicU64::new(u64::MAX),
            save_blocked: AtomicBool::new(false),
            auto_lock_event: Mutex::new(None),
            save_event: Mutex::new(None),
        }
    }
}

impl VaultState {
    /// The only way an inner function touches `inner`. Checks the idle
    /// deadline first: a machine waking past the deadline locks before the
    /// first command, and `vault_touch` cannot resurrect an expired session.
    /// A deadline hit here is the one place the auto-lock flag is recorded;
    /// the tick thread reuses this path, so the event fires exactly once.
    pub fn access(&self) -> Result<std::sync::MutexGuard<'_, Option<Unlocked>>, String> {
        let mut guard = lock_or_recover(&self.inner);
        if guard.is_some()
            && deadline_passed(lock::boot_now_ms(), self.deadline_ms.load(Ordering::SeqCst))
        {
            if let Some(unlocked) = guard.as_mut() {
                unlocked.wipe();
            }
            *guard = None;
            // Refresh before the guard is released: the next reader must not
            // see the expired deadline. The payload is gone, so the next
            // unlock or touch is what stores a real deadline.
            self.deadline_ms
                .store(deadline_after(0, lock::boot_now_ms()), Ordering::SeqCst);
            *lock_or_recover(&self.auto_lock_event) = Some(LockReason::Idle);
            return Err(LOCKED_ERR.to_string());
        }
        Ok(guard)
    }

    /// Wipe the payload and drop the key. Records nothing: the caller owns
    /// the event (`vault_lock` emits Manual directly; the tick's expiry goes
    /// through `access`, which records Idle). Returns `true` when an unlocked
    /// payload was actually dropped, so the minimize and tray paths emit only
    /// a real lock.
    pub(crate) fn lock_inner(&self) -> bool {
        let mut guard = lock_or_recover(&self.inner);
        let dropped = guard.is_some();
        if let Some(unlocked) = guard.as_mut() {
            unlocked.wipe();
        }
        *guard = None;
        dropped
    }

    /// Store the idle deadline from the current `autoLockMinutes` preference
    /// (`0` = never).
    pub(crate) fn refresh_deadline(&self, minutes: u64) {
        self.deadline_ms.store(
            deadline_after(minutes, lock::boot_now_ms()),
            Ordering::SeqCst,
        );
    }

    /// The reason an automatic lock happened, if one has not been drained
    /// yet. Command shells call this after every result.
    pub(crate) fn take_auto_lock(&self) -> Option<LockReason> {
        lock_or_recover(&self.auto_lock_event).take()
    }

    /// The outcome of the last save attempt, if no shell has drained it yet.
    pub(crate) fn take_save_event(&self) -> Option<SaveOutcome> {
        lock_or_recover(&self.save_event).take()
    }

    /// True while a failed write is parked. The quit flow asks before it lets
    /// the process exit and lose those changes.
    pub(crate) fn has_pending(&self) -> bool {
        lock_or_recover(&self.pending).is_some()
    }

    /// Drop the parked seal: a write that landed makes it stale, and a
    /// confirmed quit loses it with the process either way.
    pub(crate) fn drop_pending(&self) {
        *lock_or_recover(&self.pending) = None;
    }

    /// The refusal every mutation answers while the payload came from the
    /// `.bak`. Checked BEFORE the payload is touched, not only at write time:
    /// an edit that only ever lived in memory would be dropped by a quit that
    /// never prompted, because no seal was ever parked for it.
    pub(crate) fn ensure_writable(&self) -> Result<(), String> {
        if self.save_blocked.load(Ordering::SeqCst) {
            return Err("vault: restore the snapshot before saving".to_string());
        }
        Ok(())
    }
}

// ---- Save machinery ----
//
// Lock order, everywhere: save_lock, then inner, then pending. Nothing takes
// them in reverse, so no deadlock.
//
// `pending` holds the newest seal whose write failed. A SUCCESSFUL write
// clears it: a parked seal must never outlive a save that landed, or the
// retry would overwrite newer data (including a master password change) with
// older ciphertext.

/// Seal the current payload and write it, after any payload mutation. On
/// failure the seal is parked in `pending` and the error propagates; the
/// in-memory payload is already mutated, which is the contract the retry
/// path relies on.
pub(crate) fn commit(state: &VaultState, dir: &Path) -> Result<(), String> {
    // The lock is held across the seal AND the write, so a commit is one
    // ordered unit: two commits can never seal in one order and land in the
    // other, and an older seal can never land after a newer one.
    let _held = lock_or_recover(&state.save_lock);
    let sealed = {
        let guard = state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        seal_payload(&unlocked.payload, &unlocked.key, &unlocked.kdf)?
    };
    perform_save_locked(state, dir, sealed)
}

/// Write `file` with `save_lock` already held. On success any parked seal is
/// dropped (the disk now holds something at least as new); on failure this
/// file is parked as the newest seal.
pub(crate) fn perform_save_locked(
    state: &VaultState,
    dir: &Path,
    file: VaultFile,
) -> Result<(), String> {
    state.ensure_writable()?;
    match save_vault(dir, &file) {
        Ok(()) => {
            state.drop_pending();
            *lock_or_recover(&state.save_event) = Some(SaveOutcome::Succeeded);
            Ok(())
        }
        Err(e) => {
            *lock_or_recover(&state.pending) = Some(file);
            *lock_or_recover(&state.save_event) = Some(SaveOutcome::Failed(e.clone()));
            Err(e)
        }
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::modules::vault::entry_commands::*;
    use crate::modules::vault::file;
    use crate::modules::vault::query::*;
    use crate::modules::vault::session::*;
    use crate::modules::vault::test_util::*;

    /// A locked vault refuses the copy before any clipboard round trip, so
    /// this is testable headless; the round trip itself needs a display.
    #[test]
    fn clip_copy_field_lock_path() {
        let dir = TempDir::new("clip");
        let state = VaultState::default();
        assert_eq!(
            crate::modules::clipboard::clip_copy_field_inner(&state, &dir.0, "any", "password")
                .err()
                .unwrap(),
            LOCKED_ERR
        );
    }

    /// The stale-seal regression: a successful save must clear a parked
    /// seal, or the retry lands an older ciphertext over newer data (a lost
    /// edit, or a reverted master password change).
    #[test]
    fn a_successful_save_clears_a_parked_seal() {
        let dir = TempDir::new("pending");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let summary = vault_entry_upsert_inner(&state, &dir.0, draft(None, "One")).unwrap();
        let _ = summary;

        // Park a seal by pointing the vault PRIMARY at a directory (the .bak
        // keeps the pre-failure state, so the primary is what fails).
        let primary = dir.0.join(file::VAULT_FILE_NAME);
        let stash = dir.0.join("stash.json");
        std::fs::rename(&primary, &stash).unwrap();
        std::fs::create_dir_all(&primary).unwrap();
        // The .bak is the last complete copy: check what the in-memory list
        // looks like right after the failure.
        let titles_now: Vec<String> = vault_list_inner(&state)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.title)
            .collect();
        eprintln!("titles after failed save: {titles_now:?}");
        let err = vault_entry_upsert_inner(&state, &dir.0, draft(None, "Two")).unwrap_err();
        assert!(
            lock_or_recover(&state.pending).is_some(),
            "upsert must park: {err}"
        );

        // Remove the blocker, edit again: this save succeeds and must clear
        // the parked seal.
        std::fs::remove_dir_all(&primary).unwrap();
        std::fs::rename(&stash, &primary).unwrap();
        vault_entry_upsert_inner(&state, &dir.0, draft(None, "Three")).unwrap();
        assert!(
            lock_or_recover(&state.pending).is_none(),
            "a successful save must clear the parked seal"
        );

        // The retry is then a no-op: nothing is parked, the disk keeps
        // One+Three, and "Two" exists only in the in-memory payload.
        vault_retry_save_inner(&state, &dir.0).unwrap();
        let probe = VaultState::default();
        vault_unlock_inner(&probe, &dir.0, "master-pw").unwrap();
        let titles: Vec<String> = vault_list_inner(&probe)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.title)
            .collect();
        // "Three" on disk proves the newest in-memory state landed; the
        // parked One+Two seal was cleared by that same save, and the retry
        // above was a no-op (nothing was parked). "Two" is in the list only
        // because the successful save sealed the whole in-memory payload.
        assert!(titles.contains(&"Three".to_string()));
        assert!(titles.contains(&"Two".to_string()));
    }

    /// A failed change-master must not park the new-password seal for the
    /// retry to land behind the user's back.
    #[test]
    fn a_failed_change_master_parks_nothing() {
        let dir = TempDir::new("changefail");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        // Block the write.
        let primary = dir.0.join(file::VAULT_FILE_NAME);
        let stash = dir.0.join("stash.json");
        std::fs::rename(&primary, &stash).unwrap();
        std::fs::create_dir_all(&primary).unwrap();
        let err =
            vault_change_master_inner(&state, &dir.0, "master-pw", "new-master-pw").unwrap_err();
        assert!(
            !err.contains("at least 8"),
            "the failure must come from the write, not validation: {err}"
        );
        assert!(
            lock_or_recover(&state.pending).is_none(),
            "a failed change must park no seal"
        );
        // Un-block: the disk must still open with the OLD password.
        std::fs::remove_dir_all(&primary).unwrap();
        std::fs::rename(&stash, &primary).unwrap();
        let probe = VaultState::default();
        vault_unlock_inner(&probe, &dir.0, "master-pw").unwrap();
    }

    /// A manual lock emits exactly one vault-locked: the flag records only
    /// idle locks, which go through access().
    #[test]
    fn manual_lock_records_no_auto_lock_flag() {
        let state = VaultState::default();
        assert!(!state.lock_inner(), "nothing was unlocked to drop");
        assert_eq!(state.take_auto_lock(), None);
        assert_eq!(state.take_auto_lock(), None, "the flag must not re-fire");
    }

    /// A parked seal over an intact primary is newer data, not a broken file:
    /// unlock still takes the seal but leaves saving unblocked.
    #[test]
    fn a_parked_seal_does_not_block_saving() {
        let dir = TempDir::new("parked");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();

        // Park a seal by pointing the vault PRIMARY at a directory (the .bak
        // keeps the pre-failure copy).
        let primary = dir.0.join(file::VAULT_FILE_NAME);
        let stash = dir.0.join("stash.json");
        std::fs::rename(&primary, &stash).unwrap();
        std::fs::create_dir_all(&primary).unwrap();
        assert!(vault_entry_upsert_inner(&state, &dir.0, draft(None, "Parked")).is_err());
        assert!(lock_or_recover(&state.pending).is_some());
        // Put the intact primary back: the disk copy is stale, not broken.
        std::fs::remove_dir_all(&primary).unwrap();
        std::fs::rename(&stash, &primary).unwrap();

        // Re-unlock the same state so its parked seal takes part.
        vault_unlock_inner(&state, &dir.0, "master-pw").unwrap();
        assert!(!state.save_blocked.load(Ordering::SeqCst));
        let status = vault_status_inner(&state, &dir.0).unwrap();
        assert!(!status.save_blocked);
        assert!(status.save_pending);
        // The seal supplied the payload...
        assert!(vault_list_inner(&state)
            .unwrap()
            .entries
            .iter()
            .any(|e| e.title == "Parked"));
        // ...and the next commit simply lands.
        vault_entry_upsert_inner(&state, &dir.0, draft(None, "Landed")).unwrap();
        assert!(lock_or_recover(&state.pending).is_none());
    }

    /// The retry must keep the parked seal while it is blocked. Taking it and
    /// hitting the blocked early-return dropped it for good: the tick would
    /// clear the very state the quit prompt asks about, and the retry never
    /// landed once the snapshot was restored.
    #[test]
    fn a_blocked_retry_keeps_the_parked_seal() {
        let dir = TempDir::new("blockedretry");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();

        // Park a seal by pointing the primary at a directory.
        let primary = dir.0.join(file::VAULT_FILE_NAME);
        let stash = dir.0.join("stash.json");
        std::fs::rename(&primary, &stash).unwrap();
        std::fs::create_dir_all(&primary).unwrap();
        assert!(vault_entry_upsert_inner(&state, &dir.0, draft(None, "Parked")).is_err());
        assert!(state.has_pending());

        // Block saving, then retry: the seal must survive the attempt.
        state.save_blocked.store(true, Ordering::SeqCst);
        assert!(vault_retry_save_inner(&state, &dir.0).is_err());
        assert!(
            state.has_pending(),
            "a blocked retry must not consume the parked seal"
        );

        // Unblock and clear the path: the same seal still lands.
        std::fs::remove_dir_all(&primary).unwrap();
        std::fs::rename(&stash, &primary).unwrap();
        state.save_blocked.store(false, Ordering::SeqCst);
        vault_retry_save_inner(&state, &dir.0).unwrap();
        assert!(!state.has_pending());
        let probe = VaultState::default();
        vault_unlock_inner(&probe, &dir.0, "master-pw").unwrap();
        assert!(vault_list_inner(&probe)
            .unwrap()
            .entries
            .iter()
            .any(|e| e.title == "Parked"));
    }

    /// A staged preview is plaintext, so the wipe every lock path runs must
    /// reach it, not only the payload.
    #[test]
    fn wipe_scrubs_the_staged_preview() {
        let dir = TempDir::new("stagedwipe");
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        vault_entry_upsert_inner(&state, &dir.0, draft(None, "Staged")).unwrap();
        let mut unlocked = lock_or_recover(&state.inner).take().unwrap();
        let records = unlocked.payload.clone();
        assert_eq!(records.entries[0].password, "pw-1");
        unlocked.staged = Some(Staged {
            handle: next_stage_handle(),
            source: StagedSource::Backup,
            records: Some(records),
        });
        unlocked.wipe();
        let staged = unlocked.staged.as_ref().unwrap().records.as_ref().unwrap();
        assert_eq!(staged.entries[0].password, "");
        assert_eq!(unlocked.payload.entries[0].password, "");
    }
}
