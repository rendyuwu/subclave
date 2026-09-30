//! The vault file: seal, open, save, load.
//!
//! `<app data>/subclave-vault.json`, one JSON object whose ciphertext is the
//! whole [`VaultPayload`] under AES-256-GCM. The header (`format`, `v`, `kdf`)
//! rides as the GCM associated data, so changing any header byte fails the
//! open. Writes are atomic with a `.bak` twin; loading falls back to the
//! `.bak` only when the primary is unreadable, and never writes.

use std::path::Path;

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::modules::aesgcm;
use crate::modules::fs::atomic;
use crate::modules::vault::kdf::{check_params, derive_key, Argon2Params};
use crate::modules::vault::model::VaultPayload;

pub const VAULT_FILE_NAME: &str = "subclave-vault.json";
pub const FORMAT: &str = "subclave-vault";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VaultFile {
    pub format: String,
    pub v: u32,
    pub kdf: Argon2Params,
    /// Base64, 12 bytes, fresh on every save.
    pub nonce: String,
    /// Base64, AES-256-GCM ciphertext plus the 16-byte tag.
    pub ciphertext: String,
}

/// The header as associated data: compact JSON of the format, version and KDF
/// parameters, in declaration order, so the bytes are stable on seal and open.
fn aad(v: u32, kdf: &Argon2Params) -> Vec<u8> {
    #[derive(Serialize)]
    struct HeaderAad<'a> {
        format: &'static str,
        v: u32,
        kdf: &'a Argon2Params,
    }
    serde_json::to_vec(&HeaderAad {
        format: FORMAT,
        v,
        kdf,
    })
    .expect("header AAD serialization")
}

/// An opened vault. The key is returned with the payload because every later
/// save needs it, and deriving it a second time would double the unlock cost.
pub struct OpenVault {
    pub payload: VaultPayload,
    pub key: Zeroizing<[u8; 32]>,
    pub kdf: Argon2Params,
}

pub fn seal_payload(
    payload: &VaultPayload,
    key: &[u8; 32],
    kdf: &Argon2Params,
) -> Result<VaultFile, String> {
    let json = Zeroizing::new(serde_json::to_vec(payload).map_err(|e| format!("vault: {e}"))?);
    let (nonce, ciphertext) = aesgcm::seal_with_key(key, &aad(FORMAT_VERSION, kdf), &json)?;
    Ok(VaultFile {
        format: FORMAT.to_string(),
        v: FORMAT_VERSION,
        kdf: kdf.clone(),
        nonce: B64.encode(nonce),
        ciphertext: B64.encode(ciphertext),
    })
}

/// Open a vault file. Order of checks: format, version, KDF caps, key
/// derivation, GCM open, payload parse. Wrong password, tampered ciphertext
/// and truncation share ONE message: telling them apart tells an attacker
/// which guess was closer.
pub fn open_file(file: &VaultFile, password: &str) -> Result<OpenVault, String> {
    if file.format != FORMAT {
        return Err("vault: not a subclave vault file".to_string());
    }
    if file.v > FORMAT_VERSION {
        return Err("vault: this vault was written by a newer Subclave".to_string());
    }
    check_params(&file.kdf)?;
    let key = derive_key(password, &file.kdf)?;
    let nonce_bytes: [u8; 12] = B64
        .decode(&file.nonce)
        .ok()
        .and_then(|n| <[u8; 12]>::try_from(n).ok())
        .ok_or_else(|| "vault: wrong master password, or the vault file is corrupt".to_string())?;
    let ciphertext = B64
        .decode(&file.ciphertext)
        .map_err(|_| "vault: wrong master password, or the vault file is corrupt".to_string())?;
    let plain = aesgcm::open_with_key(
        &key,
        &aad(file.v, &file.kdf),
        &nonce_bytes,
        ciphertext,
        "vault",
    )
    .map_err(|_| "vault: wrong master password, or the vault file is corrupt".to_string())?;
    let payload: VaultPayload = serde_json::from_slice(&plain)
        .map_err(|_| "vault: vault payload is corrupt".to_string())?;
    Ok(OpenVault {
        payload,
        key,
        kdf: file.kdf.clone(),
    })
}

/// Lenient peek of just `format` and `v`. Used by [`load_vault`] so a
/// newer-version file with a changed shape still answers the
/// newer-Subclave refusal instead of falling through to the `.bak`.
pub fn peek_version(bytes: &[u8]) -> Option<(String, u32)> {
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let format = value.get("format")?.as_str()?.to_string();
    let v = value.get("v")?.as_u64()?;
    u32::try_from(v).ok().map(|v| (format, v))
}

pub fn vault_file_bytes(file: &VaultFile) -> Result<Vec<u8>, String> {
    serde_json::to_vec(file).map_err(|e| format!("vault: {e}"))
}

/// Atomic write of the primary, then the `.bak`. On Unix both land at mode
/// 0o600 (the staging temp is opened with the mode before any byte is
/// written); Windows has no equivalent and takes plain writes.
pub fn save_vault(dir: &Path, file: &VaultFile) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("vault: {e}"))?;
    let bytes = vault_file_bytes(file)?;
    let primary = dir.join(VAULT_FILE_NAME);
    let bak = dir.join(format!("{VAULT_FILE_NAME}.bak"));
    #[cfg(unix)]
    let write = |p: &Path| atomic::atomic_write_mode(p, &bytes, 0o600);
    #[cfg(not(unix))]
    let write = |p: &Path| atomic::atomic_write(p, &bytes);
    write(&primary).map_err(|e| format!("vault: {e}"))?;
    write(&bak).map_err(|e| format!("vault: {e}"))?;
    Ok(())
}

/// Cheap sanity on a parsed file, before it is trusted as "readable": the
/// nonce must decode to 12 bytes and the ciphertext must hold at least the
/// GCM tag. Anything deeper (wrong password, tampered bytes) needs the key
/// and stays with `open_file`.
fn structurally_readable(file: &VaultFile) -> bool {
    let nonce_ok = B64
        .decode(&file.nonce)
        .map(|n| n.len() == 12)
        .unwrap_or(false);
    let ciphertext_ok = B64
        .decode(&file.ciphertext)
        .map(|c| c.len() >= 16)
        .unwrap_or(false);
    nonce_ok && ciphertext_ok
}

/// Read the primary, falling back to the `.bak` when the primary is unreadable.
/// The bool is `from_bak`. Never writes anything.
///
/// A primary whose version is above the current one is refused without
/// touching the `.bak`: a newer Subclave wrote it, and opening a stale backup
/// would silently roll the vault back.
///
/// The fallback is structural only. A primary that parses but whose GCM tag
/// fails (bit rot, tampering) is indistinguishable from a wrong password
/// without the key, so `load_vault` cannot detect it and returns the file; the
/// failure surfaces at [`open_file`], and the restore flow, which knows the
/// password, is the recovery path. Deriving a key here to tell the two apart
/// would double the unlock cost for a case `open_file` already reports.
pub fn load_vault(dir: &Path) -> Result<(VaultFile, bool), String> {
    const NEWER: &str = "vault: this vault was written by a newer Subclave";
    let primary_path = dir.join(VAULT_FILE_NAME);
    if let Ok(bytes) = std::fs::read(&primary_path) {
        match serde_json::from_slice::<VaultFile>(&bytes) {
            Ok(file) => {
                if file.v > FORMAT_VERSION {
                    return Err(NEWER.to_string());
                }
                if structurally_readable(&file) {
                    return Ok((file, false));
                }
                // The primary cannot be opened; the `.bak` may be whole.
            }
            Err(_) => {
                if let Some((format, v)) = peek_version(&bytes) {
                    if format == FORMAT && v > FORMAT_VERSION {
                        return Err(NEWER.to_string());
                    }
                }
            }
        }
    }
    let bak_path = dir.join(format!("{VAULT_FILE_NAME}.bak"));
    if let Ok(bytes) = std::fs::read(&bak_path) {
        match serde_json::from_slice::<VaultFile>(&bytes) {
            Ok(file) => return Ok((file, true)),
            Err(_) => {
                if let Some((format, v)) = peek_version(&bytes) {
                    if format == FORMAT && v > FORMAT_VERSION {
                        return Err(NEWER.to_string());
                    }
                }
            }
        }
    }
    Err("vault: no readable vault file".to_string())
}

/// Read and parse the `.bak`, `None` when it is absent or does not parse.
/// The unlock path retries with this when the primary parses but fails to
/// open, which `load_vault` cannot see without the key.
pub(crate) fn read_bak(dir: &Path) -> Option<VaultFile> {
    let bytes = std::fs::read(dir.join(format!("{VAULT_FILE_NAME}.bak"))).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::vault::kdf::fresh_params;
    use crate::modules::vault::model::{DeviceState, Entry, ROOT_ID};

    const CANARY: &str = "SUBCLAVE-CANARY-7f3a";

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subclave-vault-{tag}-{}-{:?}",
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

    fn payload_with_canary() -> VaultPayload {
        VaultPayload {
            entries: vec![Entry {
                id: "e1".into(),
                group_id: ROOT_ID.into(),
                title: "Canary".into(),
                username: "u".into(),
                password: CANARY.into(),
                urls: vec![],
                notes: String::new(),
                totp: None,
                custom_fields: vec![],
                tags: vec![],
                icon: None,
                color: None,
                favorite: false,
                expires_at: None,
                trashed_from: None,
                created_at: 1,
                updated_at: 1,
                history: vec![],
                last_used_at: None,
            }],
            groups: vec![],
            tombstones: vec![],
            device: DeviceState::default(),
        }
    }

    /// One derivation per call at full default cost; the cheap-params variant
    /// below is what most tests use.
    fn seal_default(dir: &TempDir) -> (VaultFile, [u8; 32]) {
        let kdf = fresh_params().unwrap();
        let key = derive_key("master-pw", &kdf).unwrap();
        let file = seal_payload(&payload_with_canary(), &key, &kdf).unwrap();
        save_vault(&dir.0, &file).unwrap();
        (file, *key)
    }

    fn seal_cheap(dir: &TempDir) -> (VaultFile, [u8; 32], Argon2Params) {
        let mut kdf = fresh_params().unwrap();
        kdf.memory_kib = 8192;
        kdf.iterations = 1;
        let key = derive_key("master-pw", &kdf).unwrap();
        let file = seal_payload(&payload_with_canary(), &key, &kdf).unwrap();
        save_vault(&dir.0, &file).unwrap();
        (file, *key, kdf)
    }

    #[test]
    fn saved_header_carries_the_default_kdf_fields() {
        let dir = TempDir::new("header");
        let _ = seal_default(&dir);
        let bytes = std::fs::read(dir.0.join(VAULT_FILE_NAME)).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\"format\":\"subclave-vault\""), "{text}");
        assert!(text.contains("\"v\":1"), "{text}");
        assert!(text.contains("\"memoryKiB\":65536"), "{text}");
        assert!(text.contains("\"iterations\":3"), "{text}");
        assert!(text.contains("\"parallelism\":4"), "{text}");
        // A memoryKib (lowercase b) regression would corrupt the on-disk
        // format and the AAD bytes; the memoryKiB assertion above is what
        // catches it, and this one fails if both spellings ever appear.
        assert!(!text.contains("memoryKib"), "{text}");
    }

    #[test]
    fn no_file_under_the_dir_holds_the_canary() {
        let dir = TempDir::new("canary");
        let _ = seal_default(&dir);
        let mut count = 0;
        for entry in std::fs::read_dir(&dir.0).unwrap().flatten() {
            let bytes = std::fs::read(entry.path()).unwrap();
            assert!(
                !bytes.windows(CANARY.len()).any(|w| w == CANARY.as_bytes()),
                "canary found in {}",
                entry.path().display()
            );
            count += 1;
        }
        assert!(
            count >= 2,
            "expected the vault file and its .bak, found {count}"
        );
    }

    #[test]
    fn changing_the_header_version_breaks_the_aad_binding() {
        let dir = TempDir::new("aad");
        let (file, _key, _kdf) = seal_cheap(&dir);
        // v = 0 passes the format and version pre-checks and derives the same
        // key, so the only difference is the AAD bytes. The failure is the
        // binding proof.
        let mut tampered = file.clone();
        tampered.v = 0;
        assert!(open_file(&tampered, "master-pw").is_err());
        // And the honest file still opens.
        assert!(open_file(&file, "master-pw").is_ok());
    }

    #[test]
    fn newer_version_refuses_and_leaves_the_file_byte_identical() {
        let dir = TempDir::new("newer");
        let (file, _key, _kdf) = seal_cheap(&dir);
        let mut newer = file.clone();
        newer.v = 2;
        let bytes = vault_file_bytes(&newer).unwrap();
        std::fs::write(dir.0.join(VAULT_FILE_NAME), &bytes).unwrap();
        let before = std::fs::read(dir.0.join(VAULT_FILE_NAME)).unwrap();
        let err = match open_file(&newer, "master-pw") {
            Err(e) => e,
            Ok(_) => panic!("a v2 file must refuse to open"),
        };
        assert_eq!(err, "vault: this vault was written by a newer Subclave");
        assert_eq!(std::fs::read(dir.0.join(VAULT_FILE_NAME)).unwrap(), before);
        // load_vault refuses on the peek too, and never falls back to .bak.
        assert_eq!(
            load_vault(&dir.0).unwrap_err(),
            "vault: this vault was written by a newer Subclave"
        );
    }

    #[test]
    fn wrong_password_gets_the_one_opaque_message() {
        let dir = TempDir::new("wrongpw");
        let (file, _key, _kdf) = seal_cheap(&dir);
        assert_eq!(
            open_file(&file, "not-the-password").err().unwrap(),
            "vault: wrong master password, or the vault file is corrupt"
        );
    }

    #[test]
    fn broken_primary_falls_back_to_bak() {
        let dir = TempDir::new("bak");
        let (file, _key, _kdf) = seal_cheap(&dir);
        // Valid JSON, broken ciphertext field: the strict parse passes, the
        // base64 sanity check does not, so load_vault falls back to the .bak.
        let mut broken = file.clone();
        broken.ciphertext = "not-base64!!".into();
        std::fs::write(
            dir.0.join(VAULT_FILE_NAME),
            vault_file_bytes(&broken).unwrap(),
        )
        .unwrap();
        let (loaded, from_bak) = load_vault(&dir.0).unwrap();
        assert!(from_bak);
        assert_eq!(loaded.ciphertext, file.ciphertext);
        let opened = open_file(&loaded, "master-pw").unwrap();
        assert_eq!(opened.payload.entries[0].password, CANARY);
    }

    #[test]
    fn salt_decodes_to_sixteen_bytes() {
        let dir = TempDir::new("salt");
        let (file, _key, _kdf) = seal_cheap(&dir);
        assert_eq!(B64.decode(&file.kdf.salt).unwrap().len(), 16);
    }
}
