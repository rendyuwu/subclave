//! The browser socket server: the accept loop, the peer check, the per
//! connection state machine and the dispatch table.
//!
//! On Unix the listener is a `tokio::net::UnixListener` in a private
//! directory; on Windows it is a named pipe whose DACL grants only the current
//! user. The listener refuses to serve rather than panicking, recording the
//! reason in `BrowserState::listen_error` for the Settings UI.

use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use tauri::Manager;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::modules::browser::actions::{self, Failure};
use crate::modules::browser::auth;
use crate::modules::browser::host::{Host, TauriHost};
use crate::modules::browser::pairing;
use crate::modules::browser::protocol::{self, NmError, NmRequest, PROTOCOL_VERSION};
use crate::modules::browser::state::BrowserState;
use crate::modules::vault::state::{commit, now_ms, VaultState, LOCKED_ERR};
use subclave_proxy::frame::{self, FrameRead, MAX_REQUEST_FRAME, MAX_RESPONSE_FRAME};

/// Concurrent connections. A 17th is closed immediately.
const MAX_CONNECTIONS: usize = 16;

// ---- Dispatch seam ----
//
// The accept loop holds a `ConnEnv`, so a test can drive it without a Tauri
// runtime: production resolves the managed state per message, a test hands in
// leaked references.

/// The boxed future a [`ConnEnv`] dispatch returns. A named alias keeps the
/// signatures readable.
pub(crate) type BoxReplyFuture<'a> = std::pin::Pin<Box<dyn Future<Output = Reply> + Send + 'a>>;

pub(crate) trait ConnEnv: Send + Sync + 'static {
    fn dispatch<'a>(&'a self, conn: &'a mut Conn, req: &'a NmRequest) -> BoxReplyFuture<'a>;
}

struct AppEnv {
    app: tauri::AppHandle,
    host: Arc<dyn Host>,
}

impl ConnEnv for AppEnv {
    fn dispatch<'a>(&'a self, conn: &'a mut Conn, req: &'a NmRequest) -> BoxReplyFuture<'a> {
        // The vault-touching actions run here, on the async runtime, and their
        // `commit` (seal plus one file write) blocks the worker for the few
        // milliseconds it takes. That is deliberate: every commit is
        // serialized by the vault's own `save_lock` anyway, and every path here
        // needs a paired, authenticated client, so the 16-connection cap cannot
        // be turned into a runtime stall by an unauthenticated peer. Moving the
        // handlers to `spawn_blocking` would also mean giving up the
        // `&VaultState`/`&dyn Host` signatures that keep them unit-testable
        // without a Tauri runtime.
        Box::pin(async move {
            let state = self.app.state::<BrowserState>();
            let vault = self.app.state::<VaultState>();
            let dir = self.host.app_data_dir();
            dispatch(conn, req, &state, &vault, self.host.as_ref(), &dir).await
        })
    }
}

// ---- Per-connection state ----

/// A connection's authentication state.
pub(crate) struct Conn {
    pub id: u64,
    /// Set once `auth` succeeds.
    client_id: Option<String>,
    /// Set by `hello`, consumed by `auth`.
    hello: Option<Hello>,
}

struct Hello {
    client_id: String,
    app_nonce: Vec<u8>,
    ext_nonce: Vec<u8>,
}

impl Conn {
    pub(crate) fn new(id: u64) -> Self {
        Self {
            id,
            client_id: None,
            hello: None,
        }
    }
}

/// A response plus whether the connection must close after it is written.
pub(crate) struct Reply {
    pub value: Value,
    pub close: bool,
}

impl Reply {
    fn ok(value: Value) -> Self {
        Self {
            value,
            close: false,
        }
    }

    fn err(value: Value) -> Self {
        Self {
            value,
            close: false,
        }
    }

    fn closing(value: Value) -> Self {
        Self { value, close: true }
    }

    fn failure(id: &str, failure: Failure) -> Self {
        let value = match failure.message {
            Some(message) => protocol::err_response_text(id, failure.code, &message),
            None => protocol::err_response(id, failure.code),
        };
        Self::err(value)
    }
}

// ---- Start / listener preparation ----

/// Start the browser server. Called from `setup()`. A refusal is recorded, not
/// panicked: the app keeps running and Settings shows the reason.
pub(crate) fn start(app: tauri::AppHandle) {
    let host: Arc<dyn Host> = Arc::new(TauriHost(app.clone()));
    let state = (*app.state::<BrowserState>()).clone();
    let env: Arc<dyn ConnEnv> = Arc::new(AppEnv {
        app: app.clone(),
        host,
    });
    let Some(address) = subclave_proxy::path::socket_address() else {
        state.set_listen_error("the browser socket path could not be resolved");
        return;
    };

    // The endpoint is created inside the async runtime, not here: `setup()`
    // runs on the main thread with no Tokio reactor parked on it, and binding
    // a `tokio::net::UnixListener` registers the fd with the reactor and
    // panics ("there is no reactor running") when it is called outside one.
    #[cfg(unix)]
    {
        let Some(dir) = subclave_proxy::path::socket_dir() else {
            state.set_listen_error("the browser socket directory could not be resolved");
            return;
        };
        tauri::async_runtime::spawn(async move {
            match prepare_listener(&address, &dir) {
                Ok(listener) => accept_loop(listener, env, state).await,
                Err(message) => state.set_listen_error(message),
            }
        });
    }
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn(async move {
            match prepare_pipe(&address) {
                Ok(first) => accept_loop(first, env, state).await,
                Err(message) => state.set_listen_error(message),
            }
        });
    }
}

/// Notify every live connection and drop the pending pairing (a lock).
pub(crate) fn close_all(app: &tauri::AppHandle) {
    let state = (*app.state::<BrowserState>()).clone();
    state.close_all();
    state.cancel_pairing();
}

/// Notify only the connections authenticated as `client_id` (a revoke).
pub(crate) fn close_client(app: &tauri::AppHandle, client_id: &str) {
    app.state::<BrowserState>().close_client(client_id);
}

/// Prepare the Unix endpoint: create the private directory, verify its owner
/// and mode, clear a stale socket, and bind.
#[cfg(unix)]
pub(crate) fn prepare_listener(
    address: &Path,
    dir: &Path,
) -> Result<tokio::net::UnixListener, String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("the browser socket directory could not be created: {e}"))?;

    let meta = std::fs::metadata(dir)
        .map_err(|e| format!("the browser socket directory could not be read: {e}"))?;
    let euid = unsafe { libc::geteuid() };
    if meta.uid() != euid || meta.permissions().mode() & 0o777 != 0o700 {
        return Err("the socket directory is not private to your user".to_string());
    }

    if address.exists() {
        // A connect that succeeds means a live instance owns the socket; a
        // refused one leaves a stale file to remove.
        match std::os::unix::net::UnixStream::connect(address) {
            Ok(_) => {
                return Err("another Subclave instance already holds the browser socket".to_string())
            }
            Err(_) => {
                std::fs::remove_file(address)
                    .map_err(|e| format!("the stale browser socket could not be removed: {e}"))?;
            }
        }
    }

    let listener = tokio::net::UnixListener::bind(address).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            "another Subclave instance already holds the browser socket".to_string()
        } else {
            format!("the browser socket could not be bound: {e}")
        }
    })?;
    std::fs::set_permissions(address, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("the browser socket permissions could not be set: {e}"))?;
    Ok(listener)
}

// ---- Accept loops ----

#[cfg(unix)]
pub(crate) async fn accept_loop(
    listener: tokio::net::UnixListener,
    env: Arc<dyn ConnEnv>,
    state: BrowserState,
) {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(accepted) => accepted,
            // A persistent accept error (EMFILE, say) must not spin this loop;
            // a short pause also makes a transient one cheap.
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                continue;
            }
        };
        // Over the cap the connection is closed immediately rather than left
        // queued in the listen backlog: a stalled client must never make the
        // whole channel look dead, and the holders are the peers the peer-uid
        // check alone cannot reap.
        let permit = match Arc::clone(&semaphore).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                drop(stream);
                continue;
            }
        };
        if !peer_is_self(&stream) {
            drop(stream);
            drop(permit);
            continue;
        }
        let env = Arc::clone(&env);
        let state = state.clone();
        tauri::async_runtime::spawn(async move {
            handle_conn(stream, env, state).await;
            drop(permit);
        });
    }
}

/// The peer's uid must be ours, or the connection is closed before any byte is
/// read.
#[cfg(unix)]
fn peer_is_self(stream: &tokio::net::UnixStream) -> bool {
    match stream.peer_cred() {
        Ok(cred) => cred.uid() == unsafe { libc::geteuid() },
        Err(_) => false,
    }
}

// ---- Per-connection frame loop ----

async fn handle_conn<S>(stream: S, env: Arc<dyn ConnEnv>, state: BrowserState)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (id, close) = state.register_conn();
    let mut conn = Conn::new(id);
    let (mut reader, mut writer) = tokio::io::split(stream);

    loop {
        let frame = tokio::select! {
            _ = close.notified() => break,
            frame = read_request(&mut reader) => frame,
        };
        match frame {
            Ok(FrameRead::Eof) => break,
            Ok(FrameRead::Frame(bytes)) => {
                let reply = match serde_json::from_slice::<NmRequest>(&bytes) {
                    Err(_) => Reply::err(protocol::err_response(
                        &frame::request_id(&bytes),
                        NmError::BadRequest,
                    )),
                    Ok(req) => env.dispatch(&mut conn, &req).await,
                };
                if write_reply(&mut writer, &reply).await.is_err() {
                    break;
                }
                if reply.close {
                    break;
                }
            }
            Ok(FrameRead::OverCap { id, desynced }) => {
                if writer.write_all(&frame::warn_too_large(&id)).await.is_err() {
                    break;
                }
                if desynced {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    state.remove_conn(id);
    let _ = writer.shutdown().await;
}

/// Async frame read over the shared codec. The pure helpers (`decode_len`,
/// `request_id`) come from `subclave_proxy`, so there is one framing.
async fn read_request<R: AsyncRead + Unpin>(reader: &mut R) -> std::io::Result<FrameRead> {
    let mut header = [0u8; 4];
    let mut read = 0;
    while read < 4 {
        let n = reader.read(&mut header[read..]).await?;
        if n == 0 {
            if read == 0 {
                return Ok(FrameRead::Eof);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated frame header",
            ));
        }
        read += n;
    }
    let len = frame::decode_len(header) as usize;
    if len <= MAX_REQUEST_FRAME {
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).await?;
        Ok(FrameRead::Frame(buf))
    } else {
        let prefix_len = len.min(4 * 1024);
        let mut prefix = vec![0u8; prefix_len];
        reader.read_exact(&mut prefix).await?;
        let id = frame::request_id(&prefix);
        if len <= frame::MAX_STREAM_FRAME {
            let mut remaining = len - prefix_len;
            let mut scratch = [0u8; 8192];
            while remaining > 0 {
                let take = remaining.min(scratch.len());
                reader.read_exact(&mut scratch[..take]).await?;
                remaining -= take;
            }
            Ok(FrameRead::OverCap {
                id,
                desynced: false,
            })
        } else {
            Ok(FrameRead::OverCap { id, desynced: true })
        }
    }
}

async fn write_reply<W: AsyncWrite + Unpin>(writer: &mut W, reply: &Reply) -> std::io::Result<()> {
    let body = serde_json::to_vec(&reply.value).unwrap_or_else(|_| b"{}".to_vec());
    if body.len() > MAX_RESPONSE_FRAME {
        let id = reply.value.get("id").and_then(|v| v.as_str()).unwrap_or("");
        return writer.write_all(&frame::warn_too_large(id)).await;
    }
    writer
        .write_all(&frame::encode_len(body.len() as u32))
        .await?;
    writer.write_all(&body).await
}

// ---- State machine and dispatch ----

fn vault_is_locked(vault: &VaultState) -> bool {
    match vault.access() {
        Ok(guard) => guard.is_none(),
        Err(_) => true,
    }
}

/// The error precedence: `status`/`focus-app` anywhere, everything else
/// locked first, then association, then the handshake, then the actions.
pub(crate) async fn dispatch(
    conn: &mut Conn,
    req: &NmRequest,
    state: &BrowserState,
    vault: &VaultState,
    host: &dyn Host,
    dir: &Path,
) -> Reply {
    let action = req.action.as_str();
    if req.v != PROTOCOL_VERSION {
        return Reply::err(protocol::err_response(&req.id, NmError::Version));
    }
    if action == protocol::ACTION_STATUS {
        let locked = vault_is_locked(vault);
        return Reply::ok(protocol::ok_response(
            &req.id,
            serde_json::json!({
                "locked": locked,
                "appVersion": host.app_version(),
                "protocol": PROTOCOL_VERSION,
            }),
        ));
    }
    if action == protocol::ACTION_FOCUS_APP {
        host.focus_app();
        return Reply::ok(protocol::ok_response(&req.id, serde_json::json!({})));
    }
    if vault_is_locked(vault) {
        return Reply::err(protocol::err_response(&req.id, NmError::VaultLocked));
    }

    let authed = conn.client_id.is_some();
    let greeted = conn.hello.is_some();
    // Before `hello` an unlisted action is not-associated; after it, the
    // connection is mid-handshake and anything unrecognized is a bad request.
    let unlisted = if greeted {
        NmError::BadRequest
    } else {
        NmError::NotAssociated
    };

    match action {
        protocol::ACTION_ASSOCIATE if !authed => {
            match pairing::associate(state, host, vault, dir, &req.id, &req.params).await {
                Ok(result) => Reply::ok(protocol::ok_response(&req.id, result)),
                Err(failure) => Reply::failure(&req.id, failure),
            }
        }
        protocol::ACTION_HELLO if !authed && !greeted => handle_hello(conn, req, vault),
        protocol::ACTION_AUTH if !authed && greeted => handle_auth(conn, req, vault, state, dir),
        protocol::ACTION_GET_LOGINS if authed => {
            to_reply(req, actions::get_logins(vault, &req.params))
        }
        protocol::ACTION_CHECK_LOGIN if authed => {
            to_reply(req, actions::check_login(vault, &req.params))
        }
        protocol::ACTION_GET_CREDENTIAL if authed => {
            to_reply(req, actions::get_credential(vault, host, dir, &req.params))
        }
        protocol::ACTION_SAVE_LOGIN if authed => {
            to_reply(req, actions::save_login(vault, host, dir, &req.params))
        }
        protocol::ACTION_GENERATE_PASSWORD if authed => {
            to_reply(req, actions::generate_password(dir, &req.params))
        }
        _ => Reply::err(protocol::err_response(&req.id, unlisted)),
    }
}

fn to_reply(req: &NmRequest, result: Result<Value, Failure>) -> Reply {
    match result {
        Ok(result) => Reply::ok(protocol::ok_response(&req.id, result)),
        Err(failure) => Reply::failure(&req.id, failure),
    }
}

/// Look up the client and start the handshake.
fn handle_hello(conn: &mut Conn, req: &NmRequest, vault: &VaultState) -> Reply {
    let Some(client_id) = req.params.get("clientId").and_then(|v| v.as_str()) else {
        return Reply::err(protocol::err_response(&req.id, NmError::BadRequest));
    };
    let Some(ext_nonce_b64) = req.params.get("extNonce").and_then(|v| v.as_str()) else {
        return Reply::err(protocol::err_response(&req.id, NmError::BadRequest));
    };
    let Some(ext_nonce) = auth::nonce_bytes(ext_nonce_b64) else {
        return Reply::err(protocol::err_response(&req.id, NmError::BadRequest));
    };

    let secret = match client_secret(vault, client_id) {
        Ok(secret) => secret,
        Err(error) => return Reply::err(protocol::err_response(&req.id, error)),
    };

    let app_nonce_b64 = auth::random_nonce_b64();
    let Some(app_nonce) = auth::nonce_bytes(&app_nonce_b64) else {
        return Reply::err(protocol::err_response(&req.id, NmError::BadRequest));
    };
    let proof = auth::app_proof(&secret, &ext_nonce, &app_nonce);
    conn.hello = Some(Hello {
        client_id: client_id.to_string(),
        app_nonce,
        ext_nonce,
    });
    Reply::ok(protocol::ok_response(
        &req.id,
        serde_json::json!({
            "appNonce": app_nonce_b64,
            "appProof": base64_encode(&proof),
        }),
    ))
}

/// Verify the extension's proof, stamp `lastSeenAt`, and authenticate the
/// connection. A mismatch closes it.
fn handle_auth(
    conn: &mut Conn,
    req: &NmRequest,
    vault: &VaultState,
    state: &BrowserState,
    dir: &Path,
) -> Reply {
    let Some(proof_b64) = req.params.get("extProof").and_then(|v| v.as_str()) else {
        return Reply::closing(protocol::err_response(&req.id, NmError::AuthFailed));
    };
    let Some(proof) = decode_b64(proof_b64) else {
        return Reply::closing(protocol::err_response(&req.id, NmError::AuthFailed));
    };
    let Some(hello) = conn.hello.as_ref() else {
        return Reply::err(protocol::err_response(&req.id, NmError::BadRequest));
    };
    let client_id = hello.client_id.clone();

    let secret = match client_secret(vault, &client_id) {
        Ok(secret) => secret,
        Err(error) => return Reply::err(protocol::err_response(&req.id, error)),
    };
    if !auth::verify_ext_proof(&secret, &hello.app_nonce, &hello.ext_nonce, &proof) {
        return Reply::closing(protocol::err_response(&req.id, NmError::AuthFailed));
    }

    if let Err(message) = stamp_last_seen(vault, dir, &client_id, now_ms()) {
        return Reply::failure(&req.id, Failure::message(NmError::BadRequest, message));
    }
    conn.client_id = Some(client_id.clone());
    state.set_conn_client(conn.id, &client_id);
    Reply::ok(protocol::ok_response(&req.id, serde_json::json!({})))
}

/// The stored secret for `client_id`, decoded. `not-associated` when unknown.
fn client_secret(vault: &VaultState, client_id: &str) -> Result<Vec<u8>, NmError> {
    let guard = vault.access().map_err(|_| NmError::VaultLocked)?;
    let unlocked = guard.as_ref().ok_or(NmError::VaultLocked)?;
    let client = unlocked
        .payload
        .device
        .browser_clients
        .iter()
        .find(|c| c.id == client_id)
        .ok_or(NmError::NotAssociated)?;
    decode_b64(&client.secret).ok_or(NmError::NotAssociated)
}

/// Stamp `lastSeenAt` on one client and commit it. Not a history version.
fn stamp_last_seen(
    vault: &VaultState,
    dir: &Path,
    client_id: &str,
    now: u64,
) -> Result<(), String> {
    {
        let mut guard = vault.access()?;
        let unlocked = guard.as_mut().ok_or_else(|| LOCKED_ERR.to_string())?;
        let client = unlocked
            .payload
            .device
            .browser_clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or_else(|| "browser: no such client".to_string())?;
        client.last_seen_at = Some(now);
    }
    commit(vault, dir)
}

fn decode_b64(b64: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(b64).ok()
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

// ---- Windows: the named pipe endpoint ----

#[cfg(windows)]
pub(crate) fn prepare_pipe(
    _address: &Path,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer, String> {
    let name = subclave_proxy::path::pipe_name()
        .ok_or_else(|| "the browser pipe name could not be resolved".to_string())?;
    let attrs =
        PipeSecurity::new().map_err(|e| format!("the pipe DACL could not be built: {e}"))?;
    create_pipe_instance(&name, true, &attrs)
}

#[cfg(windows)]
fn create_pipe_instance(
    name: &str,
    first: bool,
    attrs: &PipeSecurity,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer, String> {
    use tokio::net::windows::named_pipe::{PipeMode, ServerOptions};

    let mut options = ServerOptions::new();
    options
        .pipe_mode(PipeMode::Byte)
        .first_pipe_instance(first)
        // Belt and braces: the DACL already excludes remote clients.
        .reject_remote_clients(true);
    // Built here so it outlives the call that reads it.
    let mut security = attrs.attributes();
    let result = unsafe {
        options.create_with_security_attributes_raw(
            name,
            std::ptr::addr_of_mut!(security).cast::<core::ffi::c_void>(),
        )
    };
    match result {
        Ok(server) => Ok(server),
        Err(error) => {
            use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
            if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
                Err("another Subclave instance already holds the browser pipe".to_string())
            } else {
                Err(format!("the browser pipe could not be created: {error}"))
            }
        }
    }
}

/// A `SECURITY_ATTRIBUTES` whose DACL grants only the current user full
/// access, built once and reused for every pipe instance.
#[cfg(windows)]
struct PipeSecurity {
    /// The SDDL-derived security descriptor, held as an address so this struct
    /// stays `Send`: the accept loop runs on the async runtime, and a raw
    /// pointer field would make its future `!Send` and unspawnable.
    descriptor: usize,
}

#[cfg(windows)]
impl PipeSecurity {
    fn new() -> Result<Self, String> {
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };

        let sid = subclave_proxy::client::current_user_sid_string()
            .ok_or_else(|| "the user SID could not be read".to_string())?;
        let sddl = format!("D:P(A;;GA;;;{sid})");
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor: *mut core::ffi::c_void = std::ptr::null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || descriptor.is_null() {
            return Err("the pipe DACL could not be converted".to_string());
        }
        Ok(Self {
            descriptor: descriptor as usize,
        })
    }

    /// A `SECURITY_ATTRIBUTES` pointing at the descriptor. Built on every call
    /// so the struct holds no pointer; the caller keeps the returned value
    /// alive for as long as `CreateNamedPipe` reads it.
    fn attributes(&self) -> windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor as *mut core::ffi::c_void,
            bInheritHandle: 0,
        }
    }
}

#[cfg(windows)]
impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::LocalFree(self.descriptor as *mut core::ffi::c_void);
        }
    }
}

#[cfg(windows)]
pub(crate) async fn accept_loop(
    mut server: tokio::net::windows::named_pipe::NamedPipeServer,
    env: Arc<dyn ConnEnv>,
    state: BrowserState,
) {
    use tokio::net::windows::named_pipe::NamedPipeServer;

    let name = match subclave_proxy::path::pipe_name() {
        Some(name) => name,
        None => return,
    };
    let attrs = match PipeSecurity::new() {
        Ok(attrs) => attrs,
        Err(_) => return,
    };
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    loop {
        if server.connect().await.is_err() {
            break;
        }
        let connected = server;
        // The listening instance must exist before the next accept, so a
        // failure to create one ends the loop.
        server = match create_pipe_instance(&name, false, &attrs) {
            Ok(next) => next,
            Err(_) => break,
        };
        let permit = match Arc::clone(&semaphore).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                // Over the cap: close the connection immediately.
                drop(connected);
                continue;
            }
        };
        let env = Arc::clone(&env);
        let state = state.clone();
        tauri::async_runtime::spawn(async move {
            handle_conn::<NamedPipeServer>(connected, env, state).await;
            drop(permit);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::fs::PermissionsExt;

    use crate::modules::browser::test_util::TestHost;
    use crate::modules::vault::test_util::TempDir;

    /// A `ConnEnv` over leaked references, so the accept loop runs without a
    /// Tauri app.
    struct TestEnv {
        state: &'static BrowserState,
        vault: &'static VaultState,
        host: Arc<TestHost>,
    }

    impl ConnEnv for TestEnv {
        fn dispatch<'a>(&'a self, conn: &'a mut Conn, req: &'a NmRequest) -> BoxReplyFuture<'a> {
            Box::pin(async move {
                let dir = self.host.app_data_dir();
                dispatch(conn, req, self.state, self.vault, self.host.as_ref(), &dir).await
            })
        }
    }

    fn run_dir(tag: &str) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new(tag);
        let sock_dir = dir.0.join("run");
        std::fs::create_dir_all(&sock_dir).unwrap();
        std::fs::set_permissions(&sock_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let address = sock_dir.join("browser.sock");
        (dir, address)
    }

    async fn write_framed<S: AsyncWrite + Unpin>(stream: &mut S, payload: &[u8]) {
        stream
            .write_all(&frame::encode_len(payload.len() as u32))
            .await
            .unwrap();
        stream.write_all(payload).await.unwrap();
    }

    async fn read_body<S: AsyncRead + Unpin>(stream: &mut S) -> Value {
        match read_request(stream).await.unwrap() {
            FrameRead::Frame(bytes) => serde_json::from_slice(&bytes).unwrap(),
            other => panic!("expected a frame, got {other:?}"),
        }
    }

    /// A directory not owned by us or not mode 0700 refuses to serve; the
    /// default `TempDir` mode (0755) is exactly that case.
    #[tokio::test]
    async fn socket_directory_mode_is_enforced() {
        let dir = TempDir::new("sockmode");
        let loose = dir.0.join("loose");
        std::fs::create_dir_all(&loose).unwrap();
        std::fs::set_permissions(&loose, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            prepare_listener(&loose.join("browser.sock"), &loose).unwrap_err(),
            "the socket directory is not private to your user"
        );

        let strict = dir.0.join("strict");
        std::fs::create_dir_all(&strict).unwrap();
        std::fs::set_permissions(&strict, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(prepare_listener(&strict.join("browser.sock"), &strict).is_ok());
    }

    #[tokio::test]
    async fn socket_status_and_over_cap() {
        let (dir, address) = run_dir("sock");
        let listener = prepare_listener(&address, address.parent().unwrap()).unwrap();
        let state: &'static BrowserState = Box::leak(Box::new(BrowserState::default()));
        let vault: &'static VaultState = Box::leak(Box::new(VaultState::default()));
        let host = Arc::new(TestHost::new(dir.0.clone()));
        let env: Arc<dyn ConnEnv> = Arc::new(TestEnv { state, vault, host });
        tauri::async_runtime::spawn(accept_loop(listener, env, state.clone()));

        let mut stream = tokio::net::UnixStream::connect(&address).await.unwrap();

        write_framed(&mut stream, br#"{"v":1,"id":"1","action":"status"}"#).await;
        let status = read_body(&mut stream).await;
        assert_eq!(status["ok"], true);
        assert_eq!(status["result"]["locked"], true);
        assert_eq!(status["result"]["protocol"], PROTOCOL_VERSION);

        // A 70 KiB request is over the cap: a too-large frame carrying the id.
        let mut big = br#"{"v":1,"id":"big","action":"get-logins","params":{"pad":""#.to_vec();
        big.resize(70 * 1024, b'x');
        big.extend_from_slice(br#""}}"#);
        assert!(big.len() > MAX_REQUEST_FRAME);
        write_framed(&mut stream, &big).await;
        let reply = read_body(&mut stream).await;
        assert_eq!(reply["id"], "big");
        assert_eq!(reply["error"]["code"], "too-large");

        // The stream is still in sync: an unknown action answers normally.
        write_framed(&mut stream, br#"{"v":1,"id":"2","action":"nope"}"#).await;
        let reply = read_body(&mut stream).await;
        assert_eq!(reply["id"], "2");
        assert_eq!(reply["error"]["code"], "vault-locked");
    }
}
