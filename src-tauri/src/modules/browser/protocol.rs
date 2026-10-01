//! The browser wire contract: the request/response shapes and the error codes.
//!
//! `extension/src/lib/protocol.ts` mirrors every literal here, and the
//! browser-verify script under scripts/ checks the two stay equal.

use serde::{Deserialize, Serialize};

/// The protocol version carried in `v`. A mismatch is [`NmError::Version`].
pub const PROTOCOL_VERSION: u32 = 1;

pub const ACTION_STATUS: &str = "status";
pub const ACTION_FOCUS_APP: &str = "focus-app";
pub const ACTION_ASSOCIATE: &str = "associate";
pub const ACTION_HELLO: &str = "hello";
pub const ACTION_AUTH: &str = "auth";
pub const ACTION_GET_LOGINS: &str = "get-logins";
pub const ACTION_GET_CREDENTIAL: &str = "get-credential";
pub const ACTION_SAVE_LOGIN: &str = "save-login";
pub const ACTION_GENERATE_PASSWORD: &str = "generate-password";

#[derive(Deserialize, Debug)]
pub struct NmRequest {
    pub v: u32,
    pub id: String,
    pub action: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// The ten error codes. The serde tag renders each as the kebab-case string in
/// `protocol.ts`, and the test below pins all ten.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(tag = "code", rename_all = "kebab-case")]
pub enum NmError {
    AppNotRunning,
    VaultLocked,
    NotAssociated,
    AuthFailed,
    PairingDenied,
    Busy,
    NoMatch,
    BadRequest,
    TooLarge,
    Version,
}

impl NmError {
    pub fn message(self) -> &'static str {
        match self {
            NmError::AppNotRunning => "Subclave is not running",
            NmError::VaultLocked => "Subclave is locked",
            NmError::NotAssociated => "Pair with Subclave first",
            NmError::AuthFailed => "Authentication failed",
            NmError::PairingDenied => "Pairing was denied",
            NmError::Busy => "Another pairing is in progress",
            NmError::NoMatch => "No matching login",
            NmError::BadRequest => "Invalid request",
            NmError::TooLarge => "Request too large",
            NmError::Version => "Unsupported protocol version",
        }
    }
}

/// The kebab-case code string of an error. The enum's internal tag is the one
/// author of the literal; this pulls it back out so the error object nests it
/// as a string rather than a second object.
fn error_code(error: NmError) -> serde_json::Value {
    serde_json::to_value(error)
        .ok()
        .and_then(|value| value.get("code").cloned())
        .unwrap_or(serde_json::Value::Null)
}

/// A success envelope. Built here so the version and the `ok` discriminant
/// have one author.
pub fn ok_response(id: &str, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "v": PROTOCOL_VERSION,
        "id": id,
        "ok": true,
        "result": result,
    })
}

/// An error envelope.
pub fn err_response(id: &str, error: NmError) -> serde_json::Value {
    serde_json::json!({
        "v": PROTOCOL_VERSION,
        "id": id,
        "ok": false,
        "error": { "code": error_code(error), "message": error.message() },
    })
}

/// An error envelope carrying a caller-supplied message (the vault's own text
/// for a refused write, which the extension surfaces verbatim).
pub fn err_response_text(id: &str, error: NmError, message: &str) -> serde_json::Value {
    serde_json::json!({
        "v": PROTOCOL_VERSION,
        "id": id,
        "ok": false,
        "error": { "code": error_code(error), "message": message },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_code_renders_its_literal() {
        let cases = [
            (NmError::AppNotRunning, "app-not-running"),
            (NmError::VaultLocked, "vault-locked"),
            (NmError::NotAssociated, "not-associated"),
            (NmError::AuthFailed, "auth-failed"),
            (NmError::PairingDenied, "pairing-denied"),
            (NmError::Busy, "busy"),
            (NmError::NoMatch, "no-match"),
            (NmError::BadRequest, "bad-request"),
            (NmError::TooLarge, "too-large"),
            (NmError::Version, "version"),
        ];
        for (error, literal) in cases {
            let value = serde_json::to_value(error).unwrap();
            assert_eq!(value["code"], literal);
        }
    }

    #[test]
    fn responses_carry_the_envelope_version() {
        let ok = ok_response("1", serde_json::json!({ "locked": false }));
        assert_eq!(ok["v"], PROTOCOL_VERSION);
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["result"]["locked"], false);
        let err = err_response("2", NmError::NotAssociated);
        assert_eq!(err["v"], PROTOCOL_VERSION);
        assert_eq!(err["ok"], false);
        assert_eq!(err["error"]["code"], "not-associated");
        assert_eq!(err["error"]["message"], "Pair with Subclave first");
    }

    #[test]
    fn requests_deserialize_with_and_without_params() {
        let with: NmRequest =
            serde_json::from_str(r#"{"v":1,"id":"a","action":"status","params":{"x":1}}"#).unwrap();
        assert_eq!(with.action, "status");
        assert_eq!(with.params["x"], 1);
        let without: NmRequest =
            serde_json::from_str(r#"{"v":1,"id":"b","action":"focus-app"}"#).unwrap();
        assert!(without.params.is_null());
    }
}
