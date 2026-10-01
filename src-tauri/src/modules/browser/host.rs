//! The seam between the browser action handlers and Tauri.
//!
//! Actions take `&dyn Host`, so they unit-test with a recording fake and no
//! Tauri runtime (there is no `tauri::test` and no `[dev-dependencies]` here).

use std::path::PathBuf;

use tauri::{Emitter, Manager};

use crate::modules::events;

pub(crate) trait Host: Send + Sync {
    fn app_version(&self) -> String;
    fn app_data_dir(&self) -> PathBuf;
    fn focus_app(&self);
    fn emit_pairing_request(&self, request_id: &str, browser: &str, profile: &str, code: &str);
    fn emit_vault_changed(&self, ids: &[String], origin: &str);
}

pub(crate) struct TauriHost(pub tauri::AppHandle);

impl Host for TauriHost {
    fn app_version(&self) -> String {
        self.0.package_info().version.to_string()
    }

    fn app_data_dir(&self) -> PathBuf {
        self.0
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::new())
    }

    fn focus_app(&self) {
        if let Some(window) = self.0.get_webview_window("main") {
            crate::windows::reveal(&window);
        }
    }

    fn emit_pairing_request(&self, request_id: &str, browser: &str, profile: &str, code: &str) {
        let _ = self.0.emit(
            events::PAIRING_REQUEST,
            serde_json::json!({
                "requestId": request_id,
                "browser": browser,
                "profileName": profile,
                "code": code,
            }),
        );
    }

    fn emit_vault_changed(&self, ids: &[String], origin: &str) {
        let _ = self.0.emit(
            events::VAULT_CHANGED,
            serde_json::json!({ "ids": ids, "origin": origin }),
        );
    }
}
