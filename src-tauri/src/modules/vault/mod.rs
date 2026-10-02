//! Vault state, the Tauri command layer, and the modules they are split over.
//!
//! [`VaultState`] holds the unlocked payload, the key, the KDF parameters, a
//! pending sealed write after a failed save, and the idle-lock deadline.
//!
//! Every command shell is `pub async fn` and does its blocking work on
//! `tauri::async_runtime::spawn_blocking` through [`events::run_blocking`] (on
//! Windows a sync command runs on the WebView2 UI thread, so blocking there
//! freezes the window; the `no_new_sync_tauri_commands` test in
//! `src-tauri/src/commands.rs` enforces this). `State<'_, _>` is not `'static`
//! and cannot move into that closure, so `run_blocking` clones the
//! `AppHandle` and resolves the managed state inside the closure. Inner
//! functions take `&VaultState` plus plain values, which is what keeps the
//! whole core testable without a Tauri runtime.

pub mod entry_commands;
pub mod events;
pub mod file;
pub mod group_commands;
pub mod kdf;
pub mod lock;
pub mod merge_history;
pub mod model;
pub mod query;
pub mod session;
pub mod state;

// `#[tauri::command]` emits a hidden macro per command (`__cmd__<name>` and
// `__tauri_command_name_<name>`) in the module where the command is written,
// and `generate_handler!` in `lib.rs` looks those up under the path it is
// given. Re-export them beside their functions so the `modules::vault::<name>`
// paths keep resolving.
pub use entry_commands::{
    __cmd__vault_entry_delete, __cmd__vault_entry_move, __cmd__vault_entry_restore,
    __cmd__vault_entry_restore_version, __cmd__vault_entry_trash, __cmd__vault_entry_upsert,
    __tauri_command_name_vault_entry_delete, __tauri_command_name_vault_entry_move,
    __tauri_command_name_vault_entry_restore, __tauri_command_name_vault_entry_restore_version,
    __tauri_command_name_vault_entry_trash, __tauri_command_name_vault_entry_upsert,
    vault_entry_delete, vault_entry_move, vault_entry_restore, vault_entry_restore_version,
    vault_entry_trash, vault_entry_upsert,
};
pub use events::LockReason;
pub use group_commands::{
    __cmd__vault_group_delete, __cmd__vault_group_upsert, __tauri_command_name_vault_group_delete,
    __tauri_command_name_vault_group_upsert, vault_group_delete, vault_group_upsert,
};
pub use query::{
    __cmd__vault_entry_get, __cmd__vault_entry_reveal, __cmd__vault_list, __cmd__vault_search,
    __tauri_command_name_vault_entry_get, __tauri_command_name_vault_entry_reveal,
    __tauri_command_name_vault_list, __tauri_command_name_vault_search, vault_entry_get,
    vault_entry_reveal, vault_list, vault_search, VaultList,
};
pub use session::{
    __cmd__vault_change_master, __cmd__vault_create, __cmd__vault_lock,
    __cmd__vault_restore_snapshot, __cmd__vault_retry_save, __cmd__vault_status,
    __cmd__vault_touch, __cmd__vault_unlock, __tauri_command_name_vault_change_master,
    __tauri_command_name_vault_create, __tauri_command_name_vault_lock,
    __tauri_command_name_vault_restore_snapshot, __tauri_command_name_vault_retry_save,
    __tauri_command_name_vault_status, __tauri_command_name_vault_touch,
    __tauri_command_name_vault_unlock, vault_change_master, vault_create, vault_lock,
    vault_restore_snapshot, vault_retry_save, vault_status, vault_touch, vault_unlock, VaultStatus,
};
pub use state::{Unlocked, VaultState, LOCKED_ERR};

// These two are referenced only from the sync engine's tests, so the plain
// library build has no use for the re-export.
#[allow(unused_imports)]
pub(crate) use entry_commands::vault_entry_upsert_inner;
pub(crate) use events::{drain_auto_lock, drain_save_event, emit_changed, emit_locked};
pub(crate) use query::resolve_field;
#[allow(unused_imports)]
pub(crate) use session::vault_create_inner;
pub(crate) use session::{install_new_vault, vault_dir, vault_retry_save_inner};
pub(crate) use state::{commit, next_stage_handle, replace_staged, Staged, StagedSource};

/// Shared fixtures for the module's tests. Kept here so the per-module test
/// suites do not each re-declare the temp directory and the draft builders.
#[cfg(test)]
pub(crate) mod test_util {
    use crate::modules::vault::model::{EntryDraft, GroupDraft, ROOT_ID};

    /// A private directory under the system temp dir, removed on drop.
    pub(crate) struct TempDir(pub(crate) std::path::PathBuf);

    impl TempDir {
        pub(crate) fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subclave-vault-flow-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id(),
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn draft(id: Option<String>, title: &str) -> EntryDraft {
        EntryDraft {
            id,
            group_id: ROOT_ID.into(),
            title: title.into(),
            username: "user".into(),
            password: Some("pw-1".into()),
            urls: vec![],
            notes: String::new(),
            totp: None,
            custom_fields: vec![],
            tags: vec![" tag ".into(), "TAG".into()],
            icon: None,
            color: None,
            favorite: false,
            expires_at: None,
        }
    }

    pub(crate) fn group_draft(
        id: Option<String>,
        parent: Option<String>,
        name: &str,
    ) -> GroupDraft {
        GroupDraft {
            id,
            parent_id: parent,
            name: name.into(),
            icon: None,
            color: None,
        }
    }
}
