//! Browser integration: the native messaging manifests, the local socket
//! server, pairing, and the actions the extension calls.
//!
//! The wire contract lives in [`protocol`], matching in [`matching`], and the
//! Tauri surface is the six commands at the bottom.

pub mod actions;
mod auth;
mod host;
mod manifests;
pub mod matching;
mod pairing;
mod protocol;
mod server;
mod state;

#[cfg(test)]
mod test_util;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::modules::vault::model::BrowserClient;
use crate::modules::vault::state::{commit, VaultState, LOCKED_ERR};

pub use manifests::Family;
pub(crate) use server::close_all;
pub(crate) use state::BrowserState;

/// A paired client as the webview sees it: the secret never leaves Rust.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserClientSummary {
    pub id: String,
    pub name: String,
    pub family: String,
    pub paired_at: u64,
    pub last_seen_at: Option<u64>,
}

fn summary_of_client(client: &BrowserClient) -> BrowserClientSummary {
    BrowserClientSummary {
        id: client.id.clone(),
        name: client.name.clone(),
        family: client.family.clone(),
        paired_at: client.paired_at,
        last_seen_at: client.last_seen_at,
    }
}

/// Call `start(app)` from `setup()`; a refusal is recorded, never panicked.
pub fn start(app: AppHandle) {
    server::start(app);
}

/// Rewrite the manifests for every enabled family (idempotent), refreshing
/// the AppImage proxy copy first.
pub fn startup_refresh(app: &AppHandle) {
    manifests::startup_refresh(app);
}

#[tauri::command]
pub async fn browser_integration_status(app: AppHandle) -> Result<Value, String> {
    let listen_error = app.state::<BrowserState>().listen_error();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(Value::Array(manifests::status_rows(&app, listen_error)))
    })
    .await
    .map_err(|e| format!("browser: task failed: {e}"))?
}

#[tauri::command]
pub async fn browser_integration_set(
    app: AppHandle,
    family: Family,
    enabled: bool,
) -> Result<(), String> {
    // A manifest that points at a socket nobody serves is a silently dead
    // channel, so enabling is refused with the listener's own message.
    if enabled {
        if let Some(message) = app.state::<BrowserState>().listen_error() {
            return Err(message);
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        if enabled {
            manifests::write_for_family(&app, family)
        } else {
            manifests::remove_for_family(&app, family)
        }
    })
    .await
    .map_err(|e| format!("browser: task failed: {e}"))?
}

#[tauri::command]
pub async fn browser_clients_list(app: AppHandle) -> Result<Vec<BrowserClientSummary>, String> {
    crate::modules::vault::events::run_blocking(&app, |state| {
        let guard = state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        Ok(unlocked
            .payload
            .device
            .browser_clients
            .iter()
            .map(summary_of_client)
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn browser_client_rename(app: AppHandle, id: String, name: String) -> Result<(), String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() || trimmed.chars().count() > 64 {
        return Err("browser: name must be 1 to 64 characters".to_string());
    }
    let dir = crate::modules::vault::vault_dir(&app)?;
    crate::modules::vault::events::run_blocking(&app, move |state| {
        rename_client_inner(state, &dir, &id, &trimmed)
    })
    .await
}

fn rename_client_inner(
    state: &VaultState,
    dir: &std::path::Path,
    id: &str,
    name: &str,
) -> Result<(), String> {
    state.ensure_writable()?;
    {
        let mut guard = state.access()?;
        let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
        let client = unlocked
            .payload
            .device
            .browser_clients
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| "browser: no such client".to_string())?;
        client.name = name.to_string();
    }
    commit(state, dir)
}

#[tauri::command]
pub async fn browser_client_revoke(app: AppHandle, id: String) -> Result<(), String> {
    let dir = crate::modules::vault::vault_dir(&app)?;
    let target = id.clone();
    crate::modules::vault::events::run_blocking(&app, move |state| {
        revoke_client_inner(state, &dir, &target)
    })
    .await?;
    // Only this client's live connections are disturbed; other paired
    // browsers keep theirs.
    server::close_client(&app, &id);
    Ok(())
}

fn revoke_client_inner(state: &VaultState, dir: &std::path::Path, id: &str) -> Result<(), String> {
    state.ensure_writable()?;
    {
        let mut guard = state.access()?;
        let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
        unlocked
            .payload
            .device
            .browser_clients
            .retain(|c| c.id != id);
    }
    commit(state, dir)
}

#[tauri::command]
pub async fn browser_pairing_respond(
    app: AppHandle,
    request_id: String,
    accept: bool,
) -> Result<(), String> {
    app.state::<BrowserState>()
        .answer_pairing(&request_id, accept);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::modules::vault::session::{vault_create_inner, vault_unlock_inner};
    use crate::modules::vault::test_util::TempDir;

    /// An unlocked vault holding two paired clients.
    fn paired(tag: &str) -> (VaultState, TempDir) {
        let dir = TempDir::new(tag);
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        {
            let mut guard = state.access().unwrap();
            let payload = &mut guard.as_mut().unwrap().payload;
            for (id, family) in [("c1", "chromium"), ("c2", "firefox")] {
                payload.device.browser_clients.push(BrowserClient {
                    id: id.into(),
                    name: id.into(),
                    family: family.into(),
                    secret: "secret".into(),
                    paired_at: 1,
                    last_seen_at: None,
                });
            }
        }
        commit(&state, &dir.0).unwrap();
        (state, dir)
    }

    fn client_ids(state: &VaultState) -> Vec<String> {
        state
            .access()
            .unwrap()
            .as_ref()
            .unwrap()
            .payload
            .device
            .browser_clients
            .iter()
            .map(|c| c.id.clone())
            .collect()
    }

    #[test]
    fn revoke_removes_exactly_one_client_and_commits_it() {
        let (state, dir) = paired("revoke");
        revoke_client_inner(&state, &dir.0, "c1").unwrap();
        assert_eq!(client_ids(&state), vec!["c2".to_string()]);

        let probe = VaultState::default();
        vault_unlock_inner(&probe, &dir.0, "master-pw").unwrap();
        assert_eq!(
            client_ids(&probe),
            vec!["c2".to_string()],
            "the removal reached disk"
        );
    }

    #[test]
    fn rename_updates_the_named_client_and_refuses_an_unknown_id() {
        let (state, dir) = paired("rename");
        rename_client_inner(&state, &dir.0, "c1", "Work").unwrap();
        let names: Vec<String> = state
            .access()
            .unwrap()
            .as_ref()
            .unwrap()
            .payload
            .device
            .browser_clients
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(names, vec!["Work".to_string(), "c2".to_string()]);
        assert!(rename_client_inner(&state, &dir.0, "ghost", "x").is_err());
    }

    #[tokio::test]
    async fn close_client_notifies_only_that_clients_connections() {
        let state = BrowserState::default();
        let (first, first_close) = state.register_conn();
        let (second, second_close) = state.register_conn();
        state.set_conn_client(first, "a");
        state.set_conn_client(second, "b");

        state.close_client("a");
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(100),
                first_close.notified()
            )
            .await
            .is_ok(),
            "the revoked client's live connection is notified"
        );
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(50),
                second_close.notified()
            )
            .await
            .is_err(),
            "another client keeps its connection"
        );
    }
}
