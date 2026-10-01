//! Managed state for the browser integration: the socket listener's refusal
//! message, the one pending pairing request, and the live connection handles.
//!
//! The struct is a thin `Arc` handle so a per-connection task can own a clone
//! without borrowing the app handle.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{oneshot, Notify};

use crate::modules::lockext::lock_or_recover;

/// The listener refusal the Settings UI shows. The status command's row shape
/// has no slot of its own, so it rides each family row as `listenError`.
#[derive(Clone, Default)]
pub(crate) struct BrowserState(Arc<Inner>);

#[derive(Default)]
struct Inner {
    listen_error: Mutex<Option<String>>,
    pending: Mutex<Option<PendingPairing>>,
    conns: Mutex<HashMap<u64, ConnHandle>>,
    next_conn: AtomicU64,
}

/// One in-flight `associate`. `respond` carries the Allow/Deny answer from the
/// pairing dialog; dropping it (a timeout) is a denial too. The browser name,
/// profile and code are handed to the dialog at `emit_pairing_request` time,
/// so they are not kept here.
pub(crate) struct PendingPairing {
    pub request_id: String,
    pub respond: oneshot::Sender<bool>,
}

/// A live connection's shared bits. `close` is notified by `close_all` (lock)
/// and `close_client` (revoke); the per-connection loop also watches it.
pub(crate) struct ConnHandle {
    pub client_id: Mutex<Option<String>>,
    pub close: Arc<Notify>,
}

impl BrowserState {
    /// Record the listener refusal the status command reports. `start` calls
    /// this at most once; a second call replaces the message.
    pub(crate) fn set_listen_error(&self, message: impl Into<String>) {
        *lock_or_recover(&self.0.listen_error) = Some(message.into());
    }

    pub(crate) fn listen_error(&self) -> Option<String> {
        lock_or_recover(&self.0.listen_error).clone()
    }

    /// Try to take the one pending-pairing slot. `false` means one is already
    /// in flight, which `associate` answers with `busy`.
    pub(crate) fn begin_pairing(&self, pending: PendingPairing) -> bool {
        let mut guard = lock_or_recover(&self.0.pending);
        if guard.is_some() {
            return false;
        }
        *guard = Some(pending);
        true
    }

    /// Clear the slot if it still holds `request_id`, so a late responder
    /// cannot clear a newer request.
    pub(crate) fn end_pairing(&self, request_id: &str) {
        let mut guard = lock_or_recover(&self.0.pending);
        if guard.as_ref().is_some_and(|p| p.request_id == request_id) {
            *guard = None;
        }
    }

    /// Answer the pending pairing named by `request_id`. An unknown or stale
    /// id is a no-op.
    pub(crate) fn answer_pairing(&self, request_id: &str, accept: bool) {
        let pending = {
            let mut guard = lock_or_recover(&self.0.pending);
            match guard.as_ref() {
                Some(p) if p.request_id == request_id => guard.take(),
                _ => None,
            }
        };
        if let Some(pending) = pending {
            let _ = pending.respond.send(accept);
        }
    }

    /// Drop the pending pairing without answering it (a lock cancels the
    /// dialog's request).
    pub(crate) fn cancel_pairing(&self) {
        *lock_or_recover(&self.0.pending) = None;
    }

    /// The request id of an in-flight pairing, if any. Tests use this to wait
    /// for `associate` to register its request before answering it.
    #[cfg(test)]
    pub(crate) fn pending_request_id(&self) -> Option<String> {
        lock_or_recover(&self.0.pending)
            .as_ref()
            .map(|p| p.request_id.clone())
    }

    /// Register a connection and return its id and close handle.
    pub(crate) fn register_conn(&self) -> (u64, Arc<Notify>) {
        let id = self.0.next_conn.fetch_add(1, Ordering::SeqCst);
        let close = Arc::new(Notify::new());
        lock_or_recover(&self.0.conns).insert(
            id,
            ConnHandle {
                client_id: Mutex::new(None),
                close: close.clone(),
            },
        );
        (id, close)
    }

    pub(crate) fn remove_conn(&self, id: u64) {
        lock_or_recover(&self.0.conns).remove(&id);
    }

    pub(crate) fn set_conn_client(&self, id: u64, client_id: &str) {
        if let Some(handle) = lock_or_recover(&self.0.conns).get(&id) {
            *lock_or_recover(&handle.client_id) = Some(client_id.to_string());
        }
    }

    /// Notify every live connection (a lock).
    pub(crate) fn close_all(&self) {
        for handle in lock_or_recover(&self.0.conns).values() {
            handle.close.notify_one();
        }
    }

    /// Notify only the connections authenticated as `client_id` (a revoke).
    pub(crate) fn close_client(&self, client_id: &str) {
        for handle in lock_or_recover(&self.0.conns).values() {
            if lock_or_recover(&handle.client_id).as_deref() == Some(client_id) {
                handle.close.notify_one();
            }
        }
    }
}
