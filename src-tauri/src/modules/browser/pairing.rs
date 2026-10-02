//! The `associate` action: show the pairing dialog, wait for the user's
//! answer, and mint the client credentials on Allow.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;

use crate::modules::browser::actions::Failure;
use crate::modules::browser::auth;
use crate::modules::browser::host::Host;
use crate::modules::browser::protocol::NmError;
use crate::modules::browser::state::{BrowserState, PendingPairing};
use crate::modules::vault::model::BrowserClient;
use crate::modules::vault::state::{commit, now_ms, VaultState};

/// How long the dialog may stay unanswered before the request is denied.
const PAIRING_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) async fn associate(
    state: &BrowserState,
    host: &dyn Host,
    vault: &VaultState,
    dir: &Path,
    request_id: &str,
    params: &Value,
) -> Result<Value, Failure> {
    let browser = params
        .get("browser")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Failure::code(NmError::BadRequest))?;
    let profile_name = params
        .get("profileName")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Failure::code(NmError::BadRequest))?;
    let pair_nonce = params
        .get("pairNonce")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Failure::code(NmError::BadRequest))?;
    let nonce = auth::nonce_bytes(pair_nonce).ok_or_else(|| Failure::code(NmError::BadRequest))?;

    // Reveal the app before asking: with `closeToTray` the window is usually
    // hidden, and the dialog has to actually be seen.
    host.focus_app();

    let code = auth::pairing_code(&nonce);
    let (respond, answer) = oneshot::channel();
    let pending = PendingPairing {
        request_id: request_id.to_string(),
        respond,
    };
    if !state.begin_pairing(pending) {
        return Err(Failure::code(NmError::Busy));
    }
    host.emit_pairing_request(request_id, browser, profile_name, &code);

    let accepted = tokio::time::timeout(PAIRING_TIMEOUT, answer).await;
    state.end_pairing(request_id);
    if !matches!(accepted, Ok(Ok(true))) {
        return Err(Failure::code(NmError::PairingDenied));
    }

    vault
        .ensure_writable()
        .map_err(|m| Failure::message(NmError::BadRequest, m))?;

    let client_id = uuid::Uuid::new_v4().to_string();
    let secret = auth::random_nonce_b64();
    let family = if browser == "Firefox" {
        "firefox"
    } else {
        "chromium"
    };

    {
        let mut guard = vault
            .access()
            .map_err(|_| Failure::code(NmError::VaultLocked))?;
        let unlocked = guard
            .as_mut()
            .ok_or_else(|| Failure::code(NmError::VaultLocked))?;
        unlocked.payload.device.browser_clients.push(BrowserClient {
            id: client_id.clone(),
            name: browser.to_string(),
            family: family.to_string(),
            secret: secret.clone(),
            paired_at: now_ms(),
            last_seen_at: None,
        });
    }
    commit(vault, dir).map_err(|m| Failure::message(NmError::BadRequest, m))?;

    Ok(serde_json::json!({ "clientId": client_id, "secret": secret }))
}
