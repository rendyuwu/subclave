//! Shared fixtures for the browser module's tests.

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;

use parking_lot::Mutex;

use crate::modules::browser::host::Host;

/// One recorded Host callback.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Recorded {
    PairingRequest {
        request_id: String,
        browser: String,
        profile: String,
        code: String,
    },
    VaultChanged {
        ids: Vec<String>,
        origin: String,
    },
}

/// A `Host` that records what the actions asked for and resolves the app data
/// dir to a test temp directory.
pub(crate) struct TestHost {
    pub dir: PathBuf,
    pub version: String,
    pub focus_calls: AtomicUsize,
    pub events: Mutex<Vec<Recorded>>,
}

impl TestHost {
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            version: "0.1.0-test".to_string(),
            focus_calls: AtomicUsize::new(0),
            events: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn events(&self) -> Vec<Recorded> {
        self.events.lock().clone()
    }

    pub(crate) fn focus_count(&self) -> usize {
        self.focus_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Host for TestHost {
    fn app_version(&self) -> String {
        self.version.clone()
    }

    fn app_data_dir(&self) -> PathBuf {
        self.dir.clone()
    }

    fn focus_app(&self) {
        self.focus_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    fn emit_pairing_request(&self, request_id: &str, browser: &str, profile: &str, code: &str) {
        self.events.lock().push(Recorded::PairingRequest {
            request_id: request_id.to_string(),
            browser: browser.to_string(),
            profile: profile.to_string(),
            code: code.to_string(),
        });
    }

    fn emit_vault_changed(&self, ids: &[String], origin: &str) {
        self.events.lock().push(Recorded::VaultChanged {
            ids: ids.to_vec(),
            origin: origin.to_string(),
        });
    }
}
