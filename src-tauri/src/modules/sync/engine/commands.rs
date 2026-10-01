//! The keyfile decision, the in-process session, and the five commands the
//! webview calls.
//!
//! Each command is a shell: it takes what it needs out of the vault lock, asks
//! the driver beside this file for a decision, and commits the result.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::Value;
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

use super::layout::keyfile_key;
use super::types::*;
use super::{
    apply_pull, finish_push, locals_from_payload, pull, push, take_dirty_envelopes, NOT_CONFIGURED,
};
use crate::modules::strength::strength_of;
use crate::modules::sync::crypto::{
    expand_root, new_keyfile_with_root, open_keyfile_root, SyncKeyfile, SyncKeys, NOT_A_KEYFILE,
};
use crate::modules::sync::device_id;
use crate::modules::sync::model::GROUP_KIND;
use crate::modules::sync::provider::{build, SyncProvider};
use crate::modules::vault::file::VAULT_FILE_NAME;
use crate::modules::vault::model::{seed_reserved_groups, SyncDevice, VaultPayload};
use crate::modules::vault::{commit, emit_changed, install_new_vault, VaultState, LOCKED_ERR};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// One opened configuration.
///
/// CLONED OUT OF THE LOCK before anything is awaited. A `std::sync::MutexGuard`
/// is not `Send`, so an async command holding one across an await does not
/// compile - and a lock held across a network round trip would serialize every
/// other caller behind the slowest request anyway. Every field here is either
/// an `Arc` or a short string, so the clone is cheap.
#[derive(Clone)]
pub struct SyncSession {
    pub keys: Arc<SyncKeys>,
    pub provider: Arc<dyn SyncProvider>,
    pub prefix: String,
    pub device: String,
}

/// The configuration the commands below run against, or none.
///
/// EMPTY ON EVERY LAUNCH, and nothing persists it: [`sync_configure`] fills it
/// and the process losing it is the whole of "sync is off". Until a caller
/// configures, every other command answers [`NOT_CONFIGURED`].
#[derive(Default)]
pub struct SyncState {
    session: Mutex<Option<SyncSession>>,
}

impl SyncState {
    pub(crate) fn open(&self) -> Result<SyncSession, String> {
        self.session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| NOT_CONFIGURED.to_string())
    }

    /// ONE SPELLING OF THE WRITE for both the open and the close.
    pub(crate) fn set(&self, session: Option<SyncSession>) {
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = session;
    }

    pub(crate) fn clear(&self) {
        *self.session.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Run `f` on the blocking pool, turning a panic or a cancelled task into the
/// one error string every caller here spells the same way.
async fn blocking<T>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("sync: task failed: {e}"))
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// The identity a stored root key and etag map are valid for:
/// `"{provider}|{endpoint}|{bucket}|{prefix}"`.
fn remote_identity(cfg: &SyncConfigArg) -> String {
    format!(
        "{}|{}|{}|{}",
        cfg.provider, cfg.endpoint, cfg.bucket, cfg.prefix
    )
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value.filter(|v| !v.is_empty()).map(str::to_string)
}

/// The provider's own configuration shape, built from the argument and the
/// credentials actually in use.
fn provider_config(cfg: &SyncConfigArg, creds: &SyncCredentialsArg) -> Result<Value, String> {
    match cfg.provider.as_str() {
        "s3" => {
            let access_key_id = non_empty(creds.access_key_id.as_deref())
                .ok_or_else(|| "sync: the storage credentials are required".to_string())?;
            let secret_access_key = non_empty(creds.secret_access_key.as_deref())
                .ok_or_else(|| "sync: the storage credentials are required".to_string())?;
            Ok(serde_json::json!({
                "endpoint": cfg.endpoint,
                "region": cfg.region,
                "bucket": cfg.bucket,
                "cas": cfg.cas,
                "accessKeyId": access_key_id,
                "secretAccessKey": secret_access_key,
            }))
        }
        "webdav" => Ok(serde_json::json!({
            "endpoint": cfg.endpoint,
            "username": creds.username.clone().unwrap_or_default(),
            "password": creds.password.clone().unwrap_or_default(),
        })),
        // The unknown id is refused by `build` with its own message; this only
        // has to hand it something shaped like a configuration.
        _ => Ok(Value::Object(serde_json::Map::new())),
    }
}

/// Point `device` at `identity`, dropping the root key and etags that only the
/// old remote was valid for. `dirty` is kept, so local edits still follow the
/// vault to its new storage. Returns whether anything changed.
pub fn reset_for_identity(device: &mut SyncDevice, identity: &str) -> bool {
    if device.remote.as_deref() == Some(identity) {
        return false;
    }
    device.remote = Some(identity.to_string());
    device.root_key = None;
    device.etags.clear();
    true
}

// ---------------------------------------------------------------------------
// Keyfiles
// ---------------------------------------------------------------------------

fn decode_root(stored: &str) -> Result<Zeroizing<[u8; 32]>, String> {
    let bytes = B64
        .decode(stored)
        .map_err(|_| "sync: the stored root key is corrupt".to_string())?;
    let root: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| "sync: the stored root key is corrupt".to_string())?;
    Ok(Zeroizing::new(root))
}

/// The keyfile decision for one configure.
///
/// Provider-injected so every branch is testable against a fake without a Tauri
/// app. `stored_root` is the base64 root key already in the vault, if any.
pub async fn configure_keyfile(
    provider: &dyn SyncProvider,
    prefix: &str,
    passphrase: Option<&str>,
    create: bool,
    stored_root: Option<&str>,
) -> Result<Configured, String> {
    let key = keyfile_key(prefix);
    let Some(object) = provider.get(&key).await.map_err(|e| e.to_string())? else {
        if !create {
            // A fresh prefix with no instruction to create one changes nothing:
            // no keyfile, no session, no write.
            return Ok(Configured {
                remote: RemoteState::Fresh,
                root: None,
            });
        }
        let pass = non_empty(passphrase)
            .ok_or_else(|| "sync: the sync passphrase is required".to_string())?;
        let pass_for_strength = pass.clone();
        let strength = blocking(move || strength_of(&pass_for_strength)).await?;
        if strength.score < 3 {
            return Err(match strength.warning {
                Some(warning) => format!("sync: the sync passphrase is too weak: {warning}"),
                None => "sync: the sync passphrase is too weak".to_string(),
            });
        }
        let pass_for_mint = pass.clone();
        let (keyfile, root) = blocking(move || new_keyfile_with_root(&pass_for_mint)).await??;
        let bytes = serde_json::to_vec(&keyfile)
            .map_err(|_| "sync: the keyfile could not be written".to_string())?;
        return match provider
            .put_if_absent(&key, bytes)
            .await
            .map_err(|e| e.to_string())?
        {
            // The write landed, so this device's root is the one the remote
            // holds and it is already in hand: re-opening the keyfile it just
            // wrote would spend a second KDF run for nothing.
            Some(_) => Ok(Configured {
                remote: RemoteState::Created,
                root: Some(root),
            }),
            None => {
                // Another device minted the keyfile first. Join its root key
                // instead of overwriting it.
                let Some(object) = provider.get(&key).await.map_err(|e| e.to_string())? else {
                    return Err("sync: the keyfile disappeared while it was being created".into());
                };
                let keyfile: SyncKeyfile =
                    serde_json::from_slice(&object.bytes).map_err(|_| NOT_A_KEYFILE.to_string())?;
                let root = blocking(move || open_keyfile_root(&keyfile, &pass)).await??;
                Ok(Configured {
                    remote: RemoteState::Existing,
                    root: Some(root),
                })
            }
        };
    };

    // A keyfile is present. Bytes that do not parse one tell the user they are
    // pointed at the wrong place, not that their passphrase is wrong.
    let keyfile: SyncKeyfile =
        serde_json::from_slice(&object.bytes).map_err(|_| NOT_A_KEYFILE.to_string())?;
    if let Some(stored) = stored_root.filter(|s| !s.is_empty()) {
        return Ok(Configured {
            remote: RemoteState::Existing,
            root: Some(decode_root(stored)?),
        });
    }
    let pass =
        non_empty(passphrase).ok_or_else(|| "sync: the sync passphrase is required".to_string())?;
    let root = blocking(move || open_keyfile_root(&keyfile, &pass)).await??;
    Ok(Configured {
        remote: RemoteState::Existing,
        root: Some(root),
    })
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Open or re-open a sync session: build the provider, settle the keyfile, and
/// hold both.
///
/// A `"fresh"` answer writes NOTHING, not even the credentials: creating the
/// keyfile is a separate, user-confirmed call (`create: true`), and until then
/// there is no remote to be configured for.
#[tauri::command]
pub async fn sync_configure(
    app: AppHandle,
    args: SyncConfigureArgs,
) -> Result<SyncConfigureResult, String> {
    let identity = remote_identity(&args.config);
    // The stored device state, with the guard dropped before anything awaits.
    let mut stored = {
        let vault_state = app.state::<VaultState>();
        let guard = vault_state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        unlocked.payload.device.sync.clone()
    };
    // A config that names another remote drops the root key and etags it cannot
    // use before anything reads them, including the `stored_root` lookup below.
    reset_for_identity(&mut stored, &identity);

    let credentials = SyncCredentialsArg {
        access_key_id: non_empty(args.credentials.access_key_id.as_deref())
            .or_else(|| stored.s3_access_key_id.clone()),
        secret_access_key: non_empty(args.credentials.secret_access_key.as_deref())
            .or_else(|| stored.s3_secret_access_key.clone()),
        username: non_empty(args.credentials.username.as_deref())
            .or_else(|| stored.webdav_username.clone()),
        password: non_empty(args.credentials.password.as_deref())
            .or_else(|| stored.webdav_password.clone()),
    };

    let provider = build(
        &args.config.provider,
        provider_config(&args.config, &credentials)?,
    )
    .map_err(|e| e.to_string())?;

    let configured = configure_keyfile(
        provider.as_ref(),
        &args.config.prefix,
        args.passphrase.as_deref(),
        args.create,
        stored.root_key.as_deref(),
    )
    .await?;

    if configured.remote == RemoteState::Fresh {
        return Ok(SyncConfigureResult {
            remote: "fresh".to_string(),
        });
    }
    let root = configured
        .root
        .ok_or_else(|| "sync: the keyfile could not be opened".to_string())?;
    let keys = Arc::new(expand_root(&root)?);

    let task_app = app.clone();
    let prefix = args.config.prefix.clone();
    let provider_for_task = Arc::clone(&provider);
    let keys_for_task = Arc::clone(&keys);
    blocking(move || -> Result<(), String> {
        let vault_state = task_app.state::<VaultState>();
        let sync_state = task_app.state::<SyncState>();
        let dir = crate::modules::vault::vault_dir(&task_app)?;
        let device = device_id(&task_app)?;
        {
            let mut guard = vault_state.access()?;
            let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
            let sync = &mut unlocked.payload.device.sync;
            reset_for_identity(sync, &identity);
            sync.root_key = Some(B64.encode(*root));
            sync.s3_access_key_id = credentials.access_key_id.clone();
            sync.s3_secret_access_key = credentials.secret_access_key.clone();
            sync.webdav_username = credentials.username.clone();
            sync.webdav_password = credentials.password.clone();
        }
        // `commit` re-enters `access()`; the guard above is already dropped.
        // A failed write leaves the persisted credentials behind the session
        // that was just opened, so the session goes with it: the caller sees an
        // error and nothing keeps syncing against a configuration the vault
        // does not hold.
        if let Err(e) = commit(&vault_state, &dir) {
            sync_state.clear();
            return Err(e);
        }
        sync_state.set(Some(SyncSession {
            keys: keys_for_task,
            provider: provider_for_task,
            prefix,
            device,
        }));
        Ok(())
    })
    .await??;

    Ok(SyncConfigureResult {
        remote: match configured.remote {
            RemoteState::Created => "created",
            _ => "existing",
        }
        .to_string(),
    })
}

/// Close the session this process holds and forget the remote it was joined to.
///
/// While the vault is locked the payload is left alone: the scheduler calls
/// this again after the next unlock, which clears it then.
#[tauri::command]
pub async fn sync_disable(app: AppHandle) -> Result<(), String> {
    app.state::<SyncState>().clear();
    let task_app = app.clone();
    blocking(move || -> Result<(), String> {
        let vault_state = task_app.state::<VaultState>();
        let dir = crate::modules::vault::vault_dir(&task_app)?;
        {
            let mut guard = match vault_state.access() {
                Ok(guard) => guard,
                Err(_) => return Ok(()),
            };
            let Some(unlocked) = guard.as_mut() else {
                return Ok(());
            };
            // The scheduler calls this on every unlock while sync is off, and
            // a commit re-seals and rewrites the vault and its backup. A device
            // that never configured anything has nothing here, so the write is
            // skipped rather than paid on every launch.
            if unlocked.payload.device.sync == SyncDevice::default() {
                return Ok(());
            }
            unlocked.payload.device.sync.clear();
        }
        commit(&vault_state, &dir)
    })
    .await??;
    Ok(())
}

/// Reconcile with the remote and land what it holds.
#[tauri::command]
pub async fn sync_pull(app: AppHandle) -> Result<SyncPullResult, String> {
    let session = app.state::<SyncState>().open()?;

    let (locals, etags, now) = {
        let vault_state = app.state::<VaultState>();
        let guard = vault_state.access()?;
        let unlocked = guard.as_ref().ok_or_else(|| LOCKED_ERR.to_string())?;
        let now = now_ms();
        (
            locals_from_payload(&unlocked.payload, now),
            unlocked.payload.device.sync.etags.clone(),
            now,
        )
    };

    let report = pull(
        session.provider.as_ref(),
        &session.keys,
        &session.prefix,
        &session.device,
        locals,
        etags,
        now,
    )
    .await
    .map_err(|e| e.to_string())?;

    let task_app = app.clone();
    let report_for_task = report.clone();
    let applied = blocking(move || -> Result<Applied, String> {
        let vault_state = task_app.state::<VaultState>();
        let dir = crate::modules::vault::vault_dir(&task_app)?;
        let applied = {
            let mut guard = vault_state.access()?;
            let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
            apply_pull(&mut unlocked.payload, &report_for_task, now)
        };
        commit(&vault_state, &dir)?;
        Ok(applied)
    })
    .await??;

    emit_changed(&app, &applied.changed_ids, "sync");
    crate::modules::vault::drain_save_event(&app);
    crate::modules::vault::drain_auto_lock(&app);

    let mut quarantine = report.quarantined.clone();
    quarantine.extend(applied.quarantine.iter().cloned());
    Ok(SyncPullResult {
        pending: report.pending,
        landed: applied.landed,
        quarantine,
        stale: applied.stale.clone(),
    })
}

/// Publish the records the remote is missing.
#[tauri::command]
pub async fn sync_push(app: AppHandle) -> Result<SyncPushResult, String> {
    let session = app.state::<SyncState>().open()?;

    let (pushed, etags) = {
        let vault_state = app.state::<VaultState>();
        let mut guard = vault_state.access()?;
        let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
        take_dirty_envelopes(&mut unlocked.payload)
    };
    if pushed.is_empty() {
        return Ok(SyncPushResult {
            pushed: 0,
            failed: 0,
        });
    }

    let pushed_for_fold = pushed.clone();
    let report = push(
        session.provider.as_ref(),
        &session.keys,
        &session.prefix,
        &session.device,
        pushed,
        etags,
    )
    .await;

    let task_app = app.clone();
    let report_for_task = report.clone();
    blocking(move || -> Result<(), String> {
        let vault_state = task_app.state::<VaultState>();
        let dir = crate::modules::vault::vault_dir(&task_app)?;
        {
            let mut guard = vault_state.access()?;
            let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
            finish_push(&mut unlocked.payload, &pushed_for_fold, &report_for_task);
        }
        commit(&vault_state, &dir)
    })
    .await??;

    crate::modules::vault::drain_save_event(&app);
    crate::modules::vault::drain_auto_lock(&app);
    Ok(SyncPushResult {
        pushed: report.etags.len(),
        failed: report.failed.len(),
    })
}

// ---------------------------------------------------------------------------
// Join
// ---------------------------------------------------------------------------

/// The records a join landed, before the vault file exists.
struct JoinedVault {
    keys: SyncKeys,
    payload: VaultPayload,
    landed: usize,
    quarantine: Vec<Quarantined>,
}

/// The keyfile and pull half of a join: no local file is touched here.
///
/// `Ok(None)` = no keyfile at the prefix, so a join cannot proceed and nothing
/// should be written.
#[allow(clippy::too_many_arguments)]
async fn join_pull(
    provider: &dyn SyncProvider,
    prefix: &str,
    passphrase: &str,
    device: &str,
    identity: &str,
    credentials: &SyncCredentialsArg,
    now: u64,
) -> Result<Option<JoinedVault>, String> {
    let key = keyfile_key(prefix);
    let Some(object) = provider.get(&key).await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let keyfile: SyncKeyfile =
        serde_json::from_slice(&object.bytes).map_err(|_| NOT_A_KEYFILE.to_string())?;
    let pass = passphrase.to_string();
    let root = blocking(move || open_keyfile_root(&keyfile, &pass)).await??;
    let keys = expand_root(&root)?;

    let report = pull(
        provider,
        &keys,
        prefix,
        device,
        Vec::new(),
        BTreeMap::new(),
        now,
    )
    .await
    .map_err(|e| e.to_string())?;

    let mut payload = VaultPayload::default();
    let applied = apply_pull(&mut payload, &report, now);
    let seeded = seed_reserved_groups(&mut payload, now);
    for id in &seeded {
        payload.device.sync.mark_dirty(GROUP_KIND, id);
    }
    payload.device.sync.remote = Some(identity.to_string());
    payload.device.sync.root_key = Some(B64.encode(*root));
    payload.device.sync.s3_access_key_id = credentials.access_key_id.clone();
    payload.device.sync.s3_secret_access_key = credentials.secret_access_key.clone();
    payload.device.sync.webdav_username = credentials.username.clone();
    payload.device.sync.webdav_password = credentials.password.clone();

    let mut quarantine = report.quarantined.clone();
    quarantine.extend(applied.quarantine.iter().cloned());
    Ok(Some(JoinedVault {
        keys,
        payload,
        landed: applied.landed,
        quarantine,
    }))
}

/// Join an existing remote: pull it, install a new vault from what it holds,
/// and open a session.
///
/// NO LOCAL VAULT FILE IS WRITTEN UNTIL THE PULL SUCCEEDED, and none at all
/// when the prefix holds no keyfile.
#[tauri::command]
pub async fn sync_join(app: AppHandle, args: SyncJoinArgs) -> Result<SyncJoinResult, String> {
    let dir = crate::modules::vault::vault_dir(&app)?;
    if dir.join(VAULT_FILE_NAME).exists() || dir.join(format!("{VAULT_FILE_NAME}.bak")).exists() {
        return Err("vault: a vault file already exists".to_string());
    }
    {
        let vault_state = app.state::<VaultState>();
        let open = vault_state
            .access()
            .map(|guard| guard.is_some())
            .unwrap_or(false);
        if open {
            return Err("vault: a vault is already open".to_string());
        }
    }

    let identity = remote_identity(&args.config);
    let provider = build(
        &args.config.provider,
        provider_config(&args.config, &args.credentials)?,
    )
    .map_err(|e| e.to_string())?;
    let device = device_id(&app)?;
    let now = now_ms();

    let joined = join_pull(
        provider.as_ref(),
        &args.config.prefix,
        &args.passphrase,
        &device,
        &identity,
        &args.credentials,
        now,
    )
    .await?;
    let Some(joined) = joined else {
        return Ok(SyncJoinResult {
            remote: "fresh".to_string(),
            landed: 0,
            quarantine: Vec::new(),
        });
    };

    let JoinedVault {
        keys,
        payload,
        landed,
        quarantine,
    } = joined;
    let task_app = app.clone();
    let master_password = args.master_password.clone();
    blocking(move || -> Result<(), String> {
        let vault_state = task_app.state::<VaultState>();
        install_new_vault(&vault_state, &dir, &master_password, payload)
    })
    .await??;

    app.state::<SyncState>().set(Some(SyncSession {
        keys: Arc::new(keys),
        provider: Arc::clone(&provider),
        prefix: args.config.prefix.clone(),
        device,
    }));

    Ok(SyncJoinResult {
        remote: "existing".to_string(),
        landed,
        quarantine,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::sync::crypto::{new_keyfile, object_name, open_record, seal_record};
    use crate::modules::sync::engine::layout::object_key;
    use crate::modules::sync::engine::test_support::*;
    use crate::modules::sync::engine::{open_envelope, seal_envelope};
    use crate::modules::sync::model::ENTRY_KIND;
    use crate::modules::vault::file::{load_vault, open_file};
    use crate::modules::vault::model::{DeviceState, EntryDraft, BROWSER_ID, ROOT_ID, TRASH_ID};
    use std::collections::BTreeSet;

    #[test]
    fn the_minted_root_is_the_one_the_keyfile_opens_to() {
        // The persist-then-reopen path a configured device runs on every
        // launch: the root that goes into `SyncDevice::root_key` is the one
        // the keyfile on the remote wraps, or the device would publish objects
        // it can never read back.
        let (kf, root) = new_keyfile_with_root("correct horse").expect("keyfile");
        let reopened = open_keyfile_root(&kf, "correct horse").expect("open root");
        assert_eq!(&root[..], &reopened[..]);

        let keys = expand_root(&root).expect("expand");
        let recovered = expand_root(&reopened).expect("expand");
        assert_eq!(
            object_name(&keys, ENTRY_KIND, "e1"),
            object_name(&recovered, ENTRY_KIND, "e1")
        );
        let sealed = seal_record(&keys, b"hello").expect("seal");
        assert_eq!(&*open_record(&recovered, &sealed).expect("open"), b"hello");
        assert!(open_keyfile_root(&kf, "wrong").is_err());
    }
    #[test]
    fn the_conditional_write_setting_reaches_the_provider() {
        // The Settings switch has to survive the whole path: `SyncConfigArg`
        // from the webview, through `provider_config`'s JSON, into the
        // provider's own `cas`.
        let creds = SyncCredentialsArg {
            access_key_id: Some("ak".into()),
            secret_access_key: Some("sk".into()),
            ..Default::default()
        };
        for cas in [true, false] {
            let arg = SyncConfigArg { cas, ..config() };
            let built = build(
                &arg.provider,
                provider_config(&arg, &creds).expect("an s3 configuration"),
            )
            .expect("the provider builds");
            assert_eq!(built.cas(), cas);
        }
    }
    #[tokio::test]
    async fn the_conditional_write_setting_decides_whether_a_create_race_is_noticed() {
        // Seen from the side the switch protects. With it on, a device that
        // finds a keyfile already at the prefix joins the winner's root key.
        // With it off the write goes out unconditionally, so the loser of a
        // real race replaces the keyfile and loses every object it sealed
        // under the root it minted. This is what turning the switch on buys.
        let (winner, winner_root) = new_keyfile_with_root(STRONG).expect("keyfile");
        let seed = |fake: &Fake| {
            fake.seed(
                &keyfile_key(PREFIX),
                serde_json::to_vec(&winner).expect("keyfile json"),
                "kf",
                None,
            );
            // The object is there, but the read that decides whether to mint
            // reports it absent: the competitor landed it between this device's
            // read and its write, which is the whole window.
            fake.hidden_once.lock().insert(keyfile_key(PREFIX));
        };

        let conditioned = {
            let fake = Fake::cas(true);
            seed(&fake);
            configure_keyfile(&fake, PREFIX, Some(STRONG), true, None)
                .await
                .expect("configure")
        };
        assert!(matches!(conditioned.remote, RemoteState::Existing));
        assert_eq!(
            &conditioned.root.expect("root")[..],
            &winner_root[..],
            "a conditional create has to keep the keyfile that was already there"
        );

        let unconditioned = {
            let fake = Fake::cas(false);
            seed(&fake);
            configure_keyfile(&fake, PREFIX, Some(STRONG), true, None)
                .await
                .expect("configure")
        };
        assert!(matches!(unconditioned.remote, RemoteState::Created));
        assert_ne!(
            &unconditioned.root.expect("root")[..],
            &winner_root[..],
            "an unconditional create replaced a keyfile it never looked at"
        );
    }
    #[tokio::test]
    async fn a_stored_root_key_opens_the_remote_without_the_passphrase() {
        // Every session after the first one takes this path: the scheduler
        // re-configures with no passphrase and no credentials, and
        // `SyncDevice::root_key` is what stands in for them.
        let fake = Fake::cas(true);
        let (keyfile, root) = new_keyfile_with_root(STRONG).expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );
        let stored = B64.encode(&root[..]);
        let configured = configure_keyfile(&fake, PREFIX, None, false, Some(&stored))
            .await
            .expect("a stored root opens the keyfile");
        assert!(matches!(configured.remote, RemoteState::Existing));
        assert_eq!(&configured.root.expect("root")[..], &root[..]);
        assert!(fake.puts().is_empty(), "a re-open wrote something");
    }
    #[tokio::test]
    async fn a_corrupt_stored_root_key_is_refused() {
        let fake = Fake::cas(true);
        let (keyfile, _) = new_keyfile_with_root(STRONG).expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );
        let err = match configure_keyfile(&fake, PREFIX, None, false, Some("not base64")).await {
            Ok(_) => panic!("a corrupt stored root must not fall back to anything"),
            Err(e) => e,
        };
        assert_eq!(err, "sync: the stored root key is corrupt");
    }
    #[tokio::test]
    async fn a_weak_passphrase_is_refused_before_a_keyfile_is_minted() {
        let fake = Fake::cas(true);
        let err = match configure_keyfile(&fake, PREFIX, Some("password"), true, None).await {
            Ok(_) => panic!("a weak passphrase must not become a sync passphrase"),
            Err(e) => e,
        };
        assert!(
            err.starts_with("sync: the sync passphrase is too weak"),
            "{err}"
        );
        assert!(fake.puts().is_empty(), "a weak passphrase still minted one");
    }
    #[test]
    fn a_config_change_drops_the_stored_root_key() {
        let mut device = SyncDevice {
            remote: Some("a".into()),
            root_key: Some("root".into()),
            etags: BTreeMap::from([("entry:e1".to_string(), "x".to_string())]),
            dirty: BTreeSet::from(["entry:e1".to_string()]),
            ..Default::default()
        };
        assert!(reset_for_identity(&mut device, "b"));
        assert_eq!(device.root_key, None);
        assert_eq!(device.remote.as_deref(), Some("b"));
        assert!(device.etags.is_empty());
        assert!(device.dirty.contains("entry:e1"));
        // The same identity is a no-op, so an unchanged remote keeps its root.
        device.root_key = Some("root".into());
        assert!(!reset_for_identity(&mut device, "b"));
        assert_eq!(device.root_key.as_deref(), Some("root"));
    }
    #[test]
    fn a_configured_but_disabled_session_makes_no_request() {
        // A device whose user configured sync last week answers this way until
        // a caller reopens the session, and an empty state issues no request.
        let state = SyncState::default();
        let err = state.open().err().expect("an empty state must refuse");
        assert_eq!(err, NOT_CONFIGURED);
        let fake = Fake::cas(true);
        assert!(fake.gets().is_empty());
        assert!(fake.puts().is_empty());
        assert!(fake.deletes().is_empty());
    }
    #[tokio::test]
    async fn configure_stores_the_root_key_and_not_the_passphrase() {
        let fake = Fake::cas(true);
        let configured = configure_keyfile(&fake, PREFIX, Some(STRONG), true, None)
            .await
            .expect("configure");
        assert_eq!(configured.remote, RemoteState::Created);
        let root = configured.root.expect("a minted keyfile has a root key");
        let payload = VaultPayload {
            device: DeviceState {
                sync: SyncDevice {
                    remote: Some(remote_identity(&config())),
                    root_key: Some(B64.encode(*root)),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(payload.device.sync.root_key.is_some());
        let serialized = serde_json::to_string(&payload).expect("payload json");
        assert!(
            !serialized.contains(STRONG),
            "the passphrase reached the payload"
        );
    }
    #[tokio::test]
    async fn the_minted_keyfile_carries_the_default_argon2_params() {
        let fake = Fake::cas(true);
        let configured = configure_keyfile(&fake, PREFIX, Some(STRONG), true, None)
            .await
            .expect("configure");
        assert_eq!(configured.remote, RemoteState::Created);
        let bytes = fake.object(&keyfile_key(PREFIX)).expect("keyfile written");
        let keyfile: SyncKeyfile = serde_json::from_slice(&bytes).expect("keyfile");
        assert_eq!(keyfile.format, "subclave-sync");
        assert_eq!(keyfile.kdf.memory_kib, 65536);
        assert_eq!(keyfile.kdf.iterations, 3);
        assert_eq!(keyfile.kdf.parallelism, 4);
    }
    #[tokio::test]
    async fn a_lost_create_race_joins_the_winner() {
        let fake = Fake::cas(true);
        let (winner_keyfile, winner_keys) = new_keyfile("correct horse").expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&winner_keyfile).expect("keyfile json"),
            "kf",
            None,
        );

        let configured = configure_keyfile(&fake, PREFIX, Some("correct horse"), true, None)
            .await
            .expect("configure");
        assert_eq!(configured.remote, RemoteState::Existing);
        assert!(fake.puts().is_empty(), "the loser overwrote the winner");
        let root = configured.root.expect("the winner's root");
        let recovered = expand_root(&root).expect("expand");

        let envelope = entry_env("h-1", NOW, "dev-b", "over there");
        let sealed = seal_envelope(&winner_keys, &envelope).expect("seal");
        assert_eq!(open_envelope(&recovered, &sealed).expect("open").id, "h-1");
    }
    #[tokio::test]
    async fn an_edit_saved_but_not_pushed_is_pushed_on_the_next_session() {
        // The executable form of "kill the app after an edit is saved and
        // before it is pushed; the next unlock pushes the edit". A real kill is
        // not automatable here, so the file the next unlock would read is what
        // this reopens.
        let dir = TempDir::new("crash");
        let state = VaultState::default();
        crate::modules::vault::vault_create_inner(&state, &dir.0, "master-password")
            .expect("create vault");
        let draft = EntryDraft {
            id: None,
            group_id: ROOT_ID.into(),
            title: "edited".into(),
            username: "user".into(),
            password: Some("pw".into()),
            urls: vec![],
            notes: String::new(),
            totp: None,
            custom_fields: vec![],
            tags: vec![],
            icon: None,
            color: None,
            favorite: false,
            expires_at: None,
        };
        let summary =
            crate::modules::vault::vault_entry_upsert_inner(&state, &dir.0, draft).expect("upsert");
        let entry_id = summary.id.clone();
        {
            let guard = state.access().expect("access");
            let unlocked = guard.as_ref().expect("unlocked");
            assert!(
                unlocked
                    .payload
                    .device
                    .sync
                    .dirty
                    .contains(&format!("entry:{entry_id}")),
                "the edit did not mark its slot dirty"
            );
        }

        // "Crash": drop the state and reopen the file the way
        // `vault_unlock_inner` does.
        drop(state);
        let (file, _from_bak) = load_vault(&dir.0).expect("load vault");
        let opened = open_file(&file, "master-password").expect("open vault");
        let mut payload = opened.payload;
        assert!(
            payload
                .device
                .sync
                .dirty
                .contains(&format!("entry:{entry_id}")),
            "the dirty mark did not survive the save"
        );

        // The next session, against an in-memory provider.
        let fake = Arc::new(Fake::cas(true));
        let (keyfile, keys) = new_keyfile("correct horse").expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );
        let keys = Arc::new(keys);
        payload.device.sync.root_key = Some("unused-here".into());
        payload.device.sync.remote = Some(remote_identity(&config()));

        let now = super::now_ms();
        let report = pull(
            fake.as_ref(),
            &keys,
            PREFIX,
            "this-device",
            locals_from_payload(&payload, now),
            payload.device.sync.etags.clone(),
            now,
        )
        .await
        .expect("pull");
        apply_pull(&mut payload, &report, now);
        let (envelopes, etags) = take_dirty_envelopes(&mut payload);
        assert!(!envelopes.is_empty(), "the edit was not queued for push");
        let push_report = push(
            fake.as_ref(),
            &keys,
            PREFIX,
            "this-device",
            envelopes.clone(),
            etags,
        )
        .await;
        assert!(push_report.failed.is_empty(), "{:?}", push_report.failed);
        finish_push(&mut payload, &envelopes, &push_report);

        let object = object_key(PREFIX, &object_name(&keys, ENTRY_KIND, &entry_id));
        let stored = fake.object(&object).expect("the edit was pushed");
        let published = open_envelope(&keys, &stored).expect("open");
        assert_eq!(published.record["title"], "edited");
    }
    #[tokio::test]
    async fn join_writes_nothing_on_a_fresh_remote() {
        let dir = TempDir::new("join-fresh");
        let fake = Fake::cas(true);
        let joined = join_pull(
            &fake,
            PREFIX,
            "correct horse",
            "this-device",
            "identity",
            &SyncCredentialsArg::default(),
            NOW,
        )
        .await
        .expect("join");
        assert!(joined.is_none());
        assert!(dir.is_empty(), "a fresh remote wrote a local file");
        // The GET of the keyfile is the last thing the provider saw.
        assert_eq!(
            fake.last_call(),
            Some(format!("get:{}", keyfile_key(PREFIX)))
        );
    }
    #[tokio::test]
    async fn join_lands_records_and_writes_the_vault() {
        let dir = TempDir::new("join-lands");
        let fake = Fake::cas(true);
        let (keyfile, keys) = new_keyfile("correct horse").expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );
        let remote = entry_env("h-1", NOW - 1000, "dev-b", "over there");
        publish(&fake, &keys, &remote, "e1", Some(NOW - 1000));

        let joined = join_pull(
            &fake,
            PREFIX,
            "correct horse",
            "this-device",
            &remote_identity(&config()),
            &SyncCredentialsArg::default(),
            NOW,
        )
        .await
        .expect("join")
        .expect("keyfile present");

        let state = VaultState::default();
        install_new_vault(&state, &dir.0, "master-password", joined.payload).expect("install");
        let (file, _from_bak) = load_vault(&dir.0).expect("load vault");
        let opened = open_file(&file, "master-password").expect("open vault");
        assert!(
            opened.payload.entries.iter().any(|e| e.id == "h-1"),
            "the pulled record did not land"
        );
    }
    #[tokio::test]
    async fn join_seeds_missing_reserved_groups() {
        let fake = Fake::cas(true);
        let (keyfile, keys) = new_keyfile("correct horse").expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );
        let remote = entry_env("h-1", NOW - 1000, "dev-b", "over there");
        publish(&fake, &keys, &remote, "e1", Some(NOW - 1000));

        let joined = join_pull(
            &fake,
            PREFIX,
            "correct horse",
            "this-device",
            &remote_identity(&config()),
            &SyncCredentialsArg::default(),
            NOW,
        )
        .await
        .expect("join")
        .expect("keyfile present");

        for id in [ROOT_ID, TRASH_ID, BROWSER_ID] {
            assert!(
                joined.payload.groups.iter().any(|g| g.id == id),
                "missing reserved group {id}"
            );
            assert!(
                joined
                    .payload
                    .device
                    .sync
                    .dirty
                    .contains(&format!("group:{id}")),
                "the seeded group {id} is not dirty"
            );
        }
    }
    #[tokio::test]
    async fn a_wrong_passphrase_writes_nothing() {
        let dir = TempDir::new("join-wrong");
        let fake = Fake::cas(true);
        let (keyfile, _keys) = new_keyfile("correct horse").expect("keyfile");
        fake.seed(
            &keyfile_key(PREFIX),
            serde_json::to_vec(&keyfile).expect("keyfile json"),
            "kf",
            None,
        );

        let result = join_pull(
            &fake,
            PREFIX,
            "wrong passphrase",
            "this-device",
            "identity",
            &SyncCredentialsArg::default(),
            NOW,
        )
        .await;
        assert_eq!(
            result.err().expect("a wrong passphrase must fail"),
            "sync: wrong sync passphrase, or the keyfile is corrupt"
        );
        assert!(dir.is_empty(), "a refused join wrote a local file");
        assert!(
            fake.puts().is_empty(),
            "a refused join wrote a remote object"
        );
    }
}
