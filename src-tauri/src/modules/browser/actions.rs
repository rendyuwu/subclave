//! Browser action handlers. Every handler takes plain values plus `&Host`, so
//! they unit-test without a Tauri runtime.

use std::path::Path;

use serde_json::{json, Value};

use crate::modules::browser::host::Host;
use crate::modules::browser::matching;
use crate::modules::browser::protocol::NmError;
use crate::modules::generator;
use crate::modules::prefs;
use crate::modules::vault::entry_commands::vault_entry_upsert_inner;
use crate::modules::vault::model::{
    DraftCustomField, EntryDraft, EntryUrl, MatchMode, VaultPayload, BROWSER_ID,
};
use crate::modules::vault::query::in_trash;
use crate::modules::vault::state::{commit, now_ms, VaultState};

/// An action's failure: an error code, and optionally the verbatim message the
/// extension surfaces (the vault's own text for a refused write).
#[derive(Debug)]
pub(crate) struct Failure {
    pub code: NmError,
    pub message: Option<String>,
}

impl Failure {
    pub(crate) fn code(code: NmError) -> Self {
        Self {
            code,
            message: None,
        }
    }

    pub(crate) fn message(code: NmError, message: impl Into<String>) -> Self {
        Self {
            code,
            message: Some(message.into()),
        }
    }
}

impl From<NmError> for Failure {
    fn from(code: NmError) -> Self {
        Failure::code(code)
    }
}

fn locked_failure() -> Failure {
    Failure::code(NmError::VaultLocked)
}

fn group_name(payload: &VaultPayload, group_id: &str) -> String {
    payload
        .groups
        .iter()
        .find(|g| g.id == group_id)
        .map(|g| g.name.clone())
        .unwrap_or_else(|| group_id.to_string())
}

fn required_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, Failure> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| Failure::code(NmError::BadRequest))
}

/// `get-logins { url, scope }`: the entries whose URL rules match the page,
/// plus `domain`, the page's registrable domain (or its host) for the picker
/// footer. Never returns a password.
pub(crate) fn get_logins(vault: &VaultState, params: &Value) -> Result<Value, Failure> {
    let raw_url = required_str(params, "url")?;
    let scope = required_str(params, "scope")?;
    if scope != "host" && scope != "all" {
        return Err(Failure::code(NmError::BadRequest));
    }
    let page = matching::parse_page(raw_url).ok_or_else(|| Failure::code(NmError::BadRequest))?;

    let guard = vault.access().map_err(|_| locked_failure())?;
    let unlocked = guard.as_ref().ok_or_else(locked_failure)?;
    let payload = &unlocked.payload;

    let mut all = Vec::new();
    for entry in &payload.entries {
        if in_trash(payload, &entry.group_id) {
            continue;
        }
        let mut any_match = false;
        let mut any_host = false;
        for url in &entry.urls {
            let Some(entry_url) = matching::parse_entry(&url.url) else {
                continue;
            };
            if matching::matches(&page, &entry_url, url.match_mode.clone()) {
                any_match = true;
                if matching::same_host(&page, &entry_url) {
                    any_host = true;
                }
            }
        }
        if !any_match {
            continue;
        }
        let row = json!({
            "id": entry.id,
            "title": entry.title,
            "username": entry.username,
            "group": group_name(payload, &entry.group_id),
            "lastUsedAt": entry.last_used_at,
        });
        all.push((row, any_host));
    }

    let other_matches = if scope == "host" {
        all.iter().filter(|(_, host)| !host).count()
    } else {
        0
    };
    let entries: Vec<Value> = if scope == "host" {
        all.into_iter()
            .filter(|(_, host)| *host)
            .map(|(row, _)| row)
            .collect()
    } else {
        all.into_iter().map(|(row, _)| row).collect()
    };
    let domain = page
        .host_str()
        .map(|h| matching::registrable(h).unwrap_or_else(|| h.to_ascii_lowercase()))
        .unwrap_or_default();
    Ok(json!({ "entries": entries, "otherMatches": other_matches, "domain": domain }))
}

/// `get-credential { id, url, via }`: release one credential, stamp its use,
/// and learn the origin when it was released on a domain-only match.
pub(crate) fn get_credential(
    vault: &VaultState,
    host: &dyn Host,
    dir: &Path,
    params: &Value,
) -> Result<Value, Failure> {
    let id = required_str(params, "id")?;
    let raw_url = required_str(params, "url")?;
    let via = required_str(params, "via")?;
    let inline = via == "inline";
    let page = matching::parse_page(raw_url).ok_or_else(|| Failure::code(NmError::BadRequest))?;
    vault
        .ensure_writable()
        .map_err(|m| Failure::message(NmError::BadRequest, m))?;

    let mut guard = vault.access().map_err(|_| locked_failure())?;
    let unlocked = guard.as_mut().ok_or_else(locked_failure)?;
    let payload = &mut unlocked.payload;

    let now = now_ms();
    let emitted_id = id.to_string();
    let (username, password) = {
        let trashed = payload
            .entries
            .iter()
            .find(|e| e.id == id)
            .map(|e| in_trash(payload, &e.group_id))
            .unwrap_or(false);
        if trashed {
            return Err(Failure::code(NmError::NoMatch));
        }
        let entry = payload
            .entries
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| Failure::code(NmError::NoMatch))?;

        // First matching URL's mode decides the origin append; the inline
        // exact-host rule accepts any same-host URL, as `get_logins` does.
        let mut mode: Option<MatchMode> = None;
        let mut host_match = false;
        for url in &entry.urls {
            let Some(entry_url) = matching::parse_entry(&url.url) else {
                continue;
            };
            if matching::matches(&page, &entry_url, url.match_mode.clone()) {
                if matching::same_host(&page, &entry_url) {
                    host_match = true;
                }
                if mode.is_none() {
                    mode = Some(url.match_mode.clone());
                }
            }
        }
        let mode = mode.ok_or_else(|| Failure::code(NmError::NoMatch))?;
        if inline && !host_match {
            return Err(Failure::code(NmError::NoMatch));
        }

        entry.last_used_at = Some(now);
        // A credential released on the strength of a domain-only match gets
        // the page's origin remembered, so the next visit is a host match.
        if mode == MatchMode::Domain && !host_match && !inline {
            let origin = matching::origin(&page);
            if !entry.urls.iter().any(|u| u.url == origin) {
                entry.urls.push(EntryUrl {
                    url: origin,
                    match_mode: MatchMode::Host,
                });
            }
        }
        (entry.username.clone(), entry.password.clone())
    };
    payload.device.sync.mark_dirty("entry", id);
    drop(guard);
    commit(vault, dir).map_err(|m| Failure::message(NmError::BadRequest, m))?;
    host.emit_vault_changed(&[emitted_id], "browser");
    Ok(json!({ "username": username, "password": password }))
}

fn draft_from_stored(
    entry: &crate::modules::vault::model::Entry,
    username: &str,
    password: &str,
) -> EntryDraft {
    EntryDraft {
        id: Some(entry.id.clone()),
        group_id: entry.group_id.clone(),
        title: entry.title.clone(),
        username: username.to_string(),
        password: Some(password.to_string()),
        urls: entry.urls.clone(),
        notes: entry.notes.clone(),
        // None = unchanged, so the stored TOTP survives.
        totp: None,
        custom_fields: entry
            .custom_fields
            .iter()
            .map(|f| DraftCustomField {
                name: f.name.clone(),
                hidden: f.hidden,
                // None = keep the stored value on an existing same-name field.
                value: None,
            })
            .collect(),
        tags: entry.tags.clone(),
        icon: entry.icon.clone(),
        color: entry.color.clone(),
        favorite: entry.favorite,
        expires_at: entry.expires_at,
    }
}

/// `save-login { url, username, password, entryId?, via }`: update the named
/// entry, or create one in the Browser group.
pub(crate) fn save_login(
    vault: &VaultState,
    host: &dyn Host,
    dir: &Path,
    params: &Value,
) -> Result<Value, Failure> {
    let raw_url = required_str(params, "url")?;
    let username = required_str(params, "username")?;
    let password = required_str(params, "password")?;
    let via = required_str(params, "via")?;
    let page = matching::parse_page(raw_url).ok_or_else(|| Failure::code(NmError::BadRequest))?;
    let entry_id = params.get("entryId").and_then(|v| v.as_str());
    let inline = via == "inline";

    vault
        .ensure_writable()
        .map_err(|m| Failure::message(NmError::BadRequest, m))?;

    let draft = match entry_id {
        Some(id) => {
            let guard = vault.access().map_err(|_| locked_failure())?;
            let unlocked = guard.as_ref().ok_or_else(locked_failure)?;
            let payload = &unlocked.payload;
            let entry = payload
                .entries
                .iter()
                .find(|e| e.id == id)
                .ok_or_else(|| Failure::code(NmError::NoMatch))?;
            if in_trash(payload, &entry.group_id) {
                return Err(Failure::code(NmError::NoMatch));
            }
            let matches_page = entry.urls.iter().any(|u| {
                matching::parse_entry(&u.url)
                    .map(|eu| {
                        matching::matches(&page, &eu, u.match_mode.clone())
                            && (!inline || matching::same_host(&page, &eu))
                    })
                    .unwrap_or(false)
            });
            if !matches_page {
                return Err(Failure::code(NmError::NoMatch));
            }
            draft_from_stored(entry, username, password)
        }
        None => {
            let host_name = page
                .host_str()
                .ok_or_else(|| Failure::code(NmError::BadRequest))?
                .to_string();
            EntryDraft {
                id: None,
                group_id: BROWSER_ID.to_string(),
                title: host_name,
                username: username.to_string(),
                password: Some(password.to_string()),
                urls: vec![EntryUrl {
                    url: matching::origin(&page),
                    match_mode: MatchMode::Host,
                }],
                notes: String::new(),
                totp: None,
                custom_fields: Vec::new(),
                tags: Vec::new(),
                icon: None,
                color: None,
                favorite: false,
                expires_at: None,
            }
        }
    };

    let created = draft.id.is_none();
    let summary = vault_entry_upsert_inner(vault, dir, draft)
        .map_err(|m| Failure::message(NmError::BadRequest, m))?;
    host.emit_vault_changed(std::slice::from_ref(&summary.id), "browser");
    Ok(json!({ "id": summary.id, "created": created }))
}

/// `generate-password`: the same generator and options the app's popover uses.
pub(crate) fn generate_password(dir: &Path, _params: &Value) -> Result<Value, Failure> {
    use ring::rand::SecureRandom as _;
    let options = prefs::read(dir).generator;
    let random = ring::rand::SystemRandom::new();
    let password = generator::generate(&options, &mut |buf| {
        random.fill(buf).expect("browser: system rng failed");
    })
    .map_err(|m| Failure::message(NmError::BadRequest, m))?;
    Ok(json!({ "password": password }))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::modules::browser::protocol::NmRequest;
    use crate::modules::browser::server::{dispatch, Reply};
    use crate::modules::browser::state::BrowserState;
    use crate::modules::browser::test_util::{Recorded, TestHost};
    use crate::modules::vault::entry_commands::{
        vault_entry_trash_inner, vault_entry_upsert_inner,
    };
    use crate::modules::vault::model::{EntryUrl, MatchMode, BROWSER_ID, ROOT_ID};
    use crate::modules::vault::session::vault_create_inner;
    use crate::modules::vault::state::VaultState;
    use crate::modules::vault::test_util::{draft, TempDir};
    use serde_json::json;
    use std::sync::Arc;

    fn request(v: u32, action: &str, params: serde_json::Value) -> NmRequest {
        serde_json::from_value(json!({
            "v": v, "id": "req-1", "action": action, "params": params,
        }))
        .unwrap()
    }

    fn setup(tag: &str) -> (VaultState, TempDir, Arc<TestHost>) {
        let dir = TempDir::new(tag);
        let state = VaultState::default();
        vault_create_inner(&state, &dir.0, "master-pw").unwrap();
        let host = Arc::new(TestHost::new(dir.0.clone()));
        (state, dir, host)
    }

    fn draft_with_url(group: &str, title: &str, url: &str, mode: MatchMode) -> EntryDraft {
        let mut d = draft(None, title);
        d.group_id = group.to_string();
        d.urls = vec![EntryUrl {
            url: url.to_string(),
            match_mode: mode,
        }];
        d
    }

    async fn run(
        state: &BrowserState,
        vault: &VaultState,
        host: &dyn Host,
        dir: &std::path::Path,
        req: &NmRequest,
    ) -> Reply {
        let mut conn = crate::modules::browser::server::Conn::new(1);
        dispatch(&mut conn, req, state, vault, host, dir).await
    }

    #[tokio::test]
    async fn status_works_while_locked_but_actions_do_not() {
        let dir = TempDir::new("locked");
        let vault = VaultState::default();
        let host = TestHost::new(dir.0.clone());
        let state = BrowserState::default();

        let reply = run(
            &state,
            &vault,
            &host,
            &dir.0,
            &request(1, "status", json!({})),
        )
        .await;
        assert_eq!(reply.value["ok"], true);
        assert_eq!(reply.value["result"]["locked"], true);
        assert_eq!(reply.value["result"]["protocol"], 1);
        assert_eq!(reply.value["result"]["appVersion"], "0.1.0-test");

        let reply = run(
            &state,
            &vault,
            &host,
            &dir.0,
            &request(1, "focus-app", json!({})),
        )
        .await;
        assert_eq!(reply.value["ok"], true);
        assert_eq!(host.focus_count(), 1);

        let reply = run(
            &state,
            &vault,
            &host,
            &dir.0,
            &request(
                1,
                "get-logins",
                json!({ "url": "https://example.com", "scope": "all" }),
            ),
        )
        .await;
        assert_eq!(reply.value["error"]["code"], "vault-locked");
    }

    #[tokio::test]
    async fn precedence_before_and_after_hello() {
        let (vault, dir, host) = setup("precedence");
        let state = BrowserState::default();

        // Unlocked, before hello: everything but associate/hello is
        // not-associated.
        let reply = run(
            &state,
            &vault,
            host.as_ref(),
            &dir.0,
            &request(
                1,
                "get-logins",
                json!({ "url": "https://x.com", "scope": "all" }),
            ),
        )
        .await;
        assert_eq!(reply.value["error"]["code"], "not-associated");

        // Unknown action is not-associated too.
        let reply = run(
            &state,
            &vault,
            host.as_ref(),
            &dir.0,
            &request(1, "nope", json!({})),
        )
        .await;
        assert_eq!(reply.value["error"]["code"], "not-associated");

        // hello with an unknown client id is not-associated (the nonce is
        // valid, so it is the lookup that refuses).
        let reply = run(
            &state,
            &vault,
            host.as_ref(),
            &dir.0,
            &request(
                1,
                "hello",
                json!({
                    "clientId": "ghost",
                    "extNonce": crate::modules::browser::auth::random_nonce_b64(),
                }),
            ),
        )
        .await;
        assert_eq!(reply.value["error"]["code"], "not-associated");

        // A malformed nonce is a bad request.
        let reply = run(
            &state,
            &vault,
            host.as_ref(),
            &dir.0,
            &request(
                1,
                "hello",
                json!({ "clientId": "ghost", "extNonce": "AAAA" }),
            ),
        )
        .await;
        assert_eq!(reply.value["error"]["code"], "bad-request");

        // A second hello would need a real client; covered by the channel test.
        // The version check runs first.
        let reply = run(
            &state,
            &vault,
            host.as_ref(),
            &dir.0,
            &request(2, "status", json!({})),
        )
        .await;
        assert_eq!(reply.value["error"]["code"], "version");
    }

    #[tokio::test]
    async fn bad_proof_closes_the_connection() {
        let (vault, dir, host) = setup("badproof");
        let state = BrowserState::default();
        let mut conn = crate::modules::browser::server::Conn::new(1);
        // Craft a hello with a real client by pairing first.
        let (client_id, _secret) = pair(&state, &vault, host.as_ref(), &dir.0).await;
        let ext_nonce = crate::modules::browser::auth::random_nonce_b64();
        let hello = request(
            1,
            "hello",
            json!({ "clientId": client_id, "extNonce": ext_nonce }),
        );
        let reply = dispatch(&mut conn, &hello, &state, &vault, host.as_ref(), &dir.0).await;
        assert_eq!(reply.value["ok"], true);

        let auth = request(1, "auth", json!({ "extProof": "AAAA" }));
        let reply = dispatch(&mut conn, &auth, &state, &vault, host.as_ref(), &dir.0).await;
        assert_eq!(reply.value["error"]["code"], "auth-failed");
        assert!(reply.close);
    }

    /// Pair a client through the real `associate` path, answering it from the
    /// same task so the oneshot resolves.
    async fn pair(
        state: &BrowserState,
        vault: &VaultState,
        host: &dyn Host,
        dir: &std::path::Path,
    ) -> (String, String) {
        let params = json!({
            "browser": "Chrome",
            "profileName": "Default",
            "pairNonce": crate::modules::browser::auth::random_nonce_b64(),
        });
        let assoc =
            crate::modules::browser::pairing::associate(state, host, vault, dir, "pair-1", &params);
        let answer = async {
            loop {
                if state.pending_request_id().as_deref() == Some("pair-1") {
                    state.answer_pairing("pair-1", true);
                    return;
                }
                tokio::task::yield_now().await;
            }
        };
        let (result, ()) = tokio::join!(assoc, answer);
        let result = result.expect("pairing allowed");
        (
            result["clientId"].as_str().unwrap().to_string(),
            result["secret"].as_str().unwrap().to_string(),
        )
    }

    #[tokio::test]
    async fn associate_stores_one_client_and_is_busy_while_pending() {
        use crate::modules::browser::state::PendingPairing;

        let (vault, dir, host) = setup("associate");
        let state = BrowserState::default();
        let params = json!({
            "browser": "Firefox",
            "profileName": "Default",
            "pairNonce": crate::modules::browser::auth::random_nonce_b64(),
        });

        // Occupy the single pairing slot: the next associate is busy without
        // waiting.
        let (respond, _answer) = tokio::sync::oneshot::channel();
        assert!(state.begin_pairing(PendingPairing {
            request_id: "held".into(),
            respond,
        }));
        let busy = crate::modules::browser::pairing::associate(
            &state,
            host.as_ref(),
            &vault,
            &dir.0,
            "pair-2",
            &params,
        )
        .await;
        assert_eq!(busy.err().map(|f| f.code), Some(NmError::Busy));
        assert_eq!(state.pending_request_id().as_deref(), Some("held"));
        state.cancel_pairing();

        // Deny: no client is stored.
        let assoc = crate::modules::browser::pairing::associate(
            &state,
            host.as_ref(),
            &vault,
            &dir.0,
            "pair-1",
            &params,
        );
        let answer = async {
            loop {
                if state.pending_request_id().as_deref() == Some("pair-1") {
                    state.answer_pairing("pair-1", false);
                    return;
                }
                tokio::task::yield_now().await;
            }
        };
        let (denied, ()) = tokio::join!(assoc, answer);
        assert_eq!(denied.err().map(|f| f.code), Some(NmError::PairingDenied));
        assert!(host.focus_count() >= 1, "the dialog is revealed first");
        {
            let guard = vault.access().unwrap();
            assert!(guard
                .as_ref()
                .unwrap()
                .payload
                .device
                .browser_clients
                .is_empty());
        }

        // Allow: exactly one client with a 32-byte secret.
        let (client_id, secret) = pair(&state, &vault, host.as_ref(), &dir.0).await;
        assert!(!client_id.is_empty());
        assert_eq!(
            crate::modules::browser::auth::nonce_bytes(&secret).map(|b| b.len()),
            Some(32)
        );
        let guard = vault.access().unwrap();
        let clients = &guard.as_ref().unwrap().payload.device.browser_clients;
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].family, "chromium");
        assert!(clients[0].last_seen_at.is_none());
    }

    /// A lock drops the pending sender (`close_all` -> `cancel_pairing`), so the
    /// waiting `associate` answers `pairing-denied` instead of hanging.
    #[tokio::test]
    async fn a_lock_cancels_a_pending_pairing() {
        let (vault, dir, host) = setup("cancelpair");
        let state = BrowserState::default();
        let params = json!({
            "browser": "Chrome",
            "profileName": "Default",
            "pairNonce": crate::modules::browser::auth::random_nonce_b64(),
        });

        let assoc = crate::modules::browser::pairing::associate(
            &state,
            host.as_ref(),
            &vault,
            &dir.0,
            "pair-lock",
            &params,
        );
        let cancel = async {
            loop {
                if state.pending_request_id().as_deref() == Some("pair-lock") {
                    state.cancel_pairing();
                    return;
                }
                tokio::task::yield_now().await;
            }
        };
        let (result, ()) = tokio::join!(assoc, cancel);
        assert_eq!(result.err().map(|f| f.code), Some(NmError::PairingDenied));
        assert!(
            state.pending_request_id().is_none(),
            "the cancelled request leaves no pending slot"
        );
        let guard = vault.access().unwrap();
        assert!(guard
            .as_ref()
            .unwrap()
            .payload
            .device
            .browser_clients
            .is_empty());
    }

    #[tokio::test]
    async fn get_logins_scope_and_trash() {
        let (vault, dir, host) = setup("logins");

        let sub = vault_entry_upsert_inner(
            &vault,
            &dir.0,
            draft_with_url(ROOT_ID, "Sub", "https://github.com", MatchMode::Domain),
        )
        .unwrap();
        let host_entry = vault_entry_upsert_inner(
            &vault,
            &dir.0,
            draft_with_url(
                ROOT_ID,
                "Host",
                "https://accounts.github.com/x",
                MatchMode::Host,
            ),
        )
        .unwrap();
        let trashed = vault_entry_upsert_inner(
            &vault,
            &dir.0,
            draft_with_url(
                ROOT_ID,
                "Gone",
                "https://accounts.github.com/y",
                MatchMode::Host,
            ),
        )
        .unwrap();
        vault_entry_trash_inner(&vault, &dir.0, vec![trashed.id.clone()]).unwrap();

        let all = actions_get_logins(
            &vault,
            &json!({ "url": "https://accounts.github.com/app", "scope": "all" }),
        )
        .unwrap();
        let ids: Vec<&str> = all["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&sub.id.as_str()));
        assert!(ids.contains(&host_entry.id.as_str()));
        assert!(!ids.contains(&trashed.id.as_str()));
        assert_eq!(all["otherMatches"], 0);

        let scoped = actions_get_logins(
            &vault,
            &json!({ "url": "https://accounts.github.com/app", "scope": "host" }),
        )
        .unwrap();
        let ids: Vec<&str> = scoped["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![host_entry.id.as_str()]);
        assert_eq!(scoped["otherMatches"], 1);
        assert_eq!(scoped["domain"], "github.com");
        // A password never crosses the wire.
        assert!(scoped["entries"][0].get("password").is_none());
        let _ = host;

        // An IP page reports its own host, never a psl guess like "0.1".
        let ip = actions_get_logins(
            &vault,
            &json!({ "url": "http://127.0.0.1:8080/", "scope": "host" }),
        )
        .unwrap();
        assert_eq!(ip["domain"], "127.0.0.1");

        // Unknown scope is bad-request.
        assert!(actions_get_logins(
            &vault,
            &json!({ "url": "https://accounts.github.com/app", "scope": "weird" })
        )
        .is_err());
    }

    fn actions_get_logins(
        vault: &VaultState,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, Failure> {
        get_logins(vault, params)
    }

    #[tokio::test]
    async fn get_credential_inline_rule_and_origin_append() {
        let (vault, dir, host) = setup("cred");
        let entry = vault_entry_upsert_inner(
            &vault,
            &dir.0,
            draft_with_url(ROOT_ID, "Sub", "https://github.com", MatchMode::Domain),
        )
        .unwrap();

        // Inline on a domain-only match is no-match.
        let inline = get_credential(
            &vault,
            host.as_ref(),
            &dir.0,
            &json!({ "id": entry.id, "url": "https://accounts.github.com/login", "via": "inline" }),
        );
        assert!(matches!(inline, Err(ref f) if f.code == NmError::NoMatch));

        // Popup on the same page releases it and appends the origin.
        let value = get_credential(
            &vault,
            host.as_ref(),
            &dir.0,
            &json!({ "id": entry.id, "url": "https://accounts.github.com/login", "via": "popup" }),
        )
        .unwrap();
        assert_eq!(value["password"], "pw-1");

        let guard = vault.access().unwrap();
        let stored = guard
            .as_ref()
            .unwrap()
            .payload
            .entries
            .iter()
            .find(|e| e.id == entry.id)
            .unwrap();
        assert!(stored.last_used_at.is_some());
        let appended = stored
            .urls
            .iter()
            .find(|u| u.url == "https://accounts.github.com")
            .expect("origin appended");
        assert_eq!(appended.match_mode, MatchMode::Host);
        // No history version was added by the append.
        assert!(stored.history.is_empty());
        assert!(guard
            .as_ref()
            .unwrap()
            .payload
            .device
            .sync
            .dirty
            .contains(&format!("entry:{}", entry.id)));
        drop(guard);

        // The entry's first URL is still the Domain one, but the appended
        // same-host URL now satisfies the inline rule.
        let value = get_credential(
            &vault,
            host.as_ref(),
            &dir.0,
            &json!({ "id": entry.id, "url": "https://accounts.github.com/login", "via": "inline" }),
        )
        .unwrap();
        assert_eq!(value["username"], "user");
        assert_eq!(value["password"], "pw-1");

        assert!(host
            .events()
            .iter()
            .any(|e| matches!(e, Recorded::VaultChanged { origin, .. } if origin == "browser")));
    }

    #[tokio::test]
    async fn save_login_creates_then_updates_with_history() {
        let (vault, dir, host) = setup("savelogin");

        let created = save_login(
            &vault,
            host.as_ref(),
            &dir.0,
            &json!({
                "url": "https://newsite.example/signup",
                "username": "user@example.com",
                "password": "first-pw",
                "via": "popup",
            }),
        )
        .unwrap();
        assert_eq!(created["created"], true);
        let id = created["id"].as_str().unwrap().to_string();

        let guard = vault.access().unwrap();
        let stored = guard
            .as_ref()
            .unwrap()
            .payload
            .entries
            .iter()
            .find(|e| e.id == id)
            .unwrap();
        assert_eq!(stored.group_id, BROWSER_ID);
        assert_eq!(stored.title, "newsite.example");
        assert_eq!(stored.urls.len(), 1);
        assert_eq!(stored.urls[0].url, "https://newsite.example");
        assert_eq!(stored.urls[0].match_mode, MatchMode::Host);
        drop(guard);

        let updated = save_login(
            &vault,
            host.as_ref(),
            &dir.0,
            &json!({
                "url": "https://newsite.example/signup",
                "username": "user@example.com",
                "password": "second-pw",
                "entryId": id,
                "via": "popup",
            }),
        )
        .unwrap();
        assert_eq!(updated["created"], false);
        let guard = vault.access().unwrap();
        let stored = guard
            .as_ref()
            .unwrap()
            .payload
            .entries
            .iter()
            .find(|e| e.id == id)
            .unwrap();
        assert_eq!(stored.password, "second-pw");
        assert_eq!(stored.history.len(), 1);
        assert_eq!(stored.history[0].password, "first-pw");
        assert_eq!(
            stored.history[0].reason,
            crate::modules::vault::model::VersionReason::Edit
        );
        drop(guard);

        // An entryId that does not match the page URL is no-match.
        let mismatch = save_login(
            &vault,
            host.as_ref(),
            &dir.0,
            &json!({
                "url": "https://elsewhere.example/",
                "username": "u",
                "password": "p",
                "entryId": id,
                "via": "popup",
            }),
        );
        assert!(matches!(mismatch, Err(ref f) if f.code == NmError::NoMatch));

        // The inline rule: an entry that matched by domain only must not be
        // updated through an inline fill, whose page URL is the weaker signal,
        // while the same update through the popup is allowed.
        let domain_entry = vault_entry_upsert_inner(
            &vault,
            &dir.0,
            draft_with_url(
                BROWSER_ID,
                "Domain only",
                "https://example.com/login",
                MatchMode::Domain,
            ),
        )
        .unwrap();
        let inline = save_login(
            &vault,
            host.as_ref(),
            &dir.0,
            &json!({
                "url": "https://accounts.example.com/login",
                "username": "u",
                "password": "p",
                "entryId": domain_entry.id,
                "via": "inline",
            }),
        );
        assert!(matches!(&inline, Err(f) if f.code == NmError::NoMatch));
        let popup = save_login(
            &vault,
            host.as_ref(),
            &dir.0,
            &json!({
                "url": "https://accounts.example.com/login",
                "username": "u",
                "password": "p",
                "entryId": domain_entry.id,
                "via": "popup",
            }),
        )
        .unwrap();
        assert_eq!(popup["created"], false);
    }

    #[tokio::test]
    async fn generate_password_uses_the_app_options() {
        let dir = TempDir::new("gen");
        std::fs::write(
            dir.0.join(crate::modules::prefs::SETTINGS_FILE_NAME),
            r#"{"generator":{"length":12,"lower":true,"upper":false,"digits":false,"symbols":false}}"#,
        )
        .unwrap();
        let value = generate_password(&dir.0, &json!({})).unwrap();
        let password = value["password"].as_str().unwrap();
        assert_eq!(password.chars().count(), 12);
        assert!(password.chars().all(|c| c.is_ascii_lowercase()));
    }

    #[tokio::test]
    async fn a_browser_write_marks_the_entry_dirty() {
        let (vault, dir, host) = setup("dirty");
        let entry = vault_entry_upsert_inner(
            &vault,
            &dir.0,
            draft_with_url(ROOT_ID, "D", "https://dirty.example", MatchMode::Host),
        )
        .unwrap();
        // The upsert already marked it; clear the map to prove the action does.
        {
            let mut guard = vault.access().unwrap();
            guard.as_mut().unwrap().payload.device.sync.dirty.clear();
        }
        get_credential(
            &vault,
            host.as_ref(),
            &dir.0,
            &json!({ "id": entry.id, "url": "https://dirty.example/login", "via": "popup" }),
        )
        .unwrap();
        let guard = vault.access().unwrap();
        assert!(guard
            .as_ref()
            .unwrap()
            .payload
            .device
            .sync
            .dirty
            .contains(&format!("entry:{}", entry.id)));
    }
}
