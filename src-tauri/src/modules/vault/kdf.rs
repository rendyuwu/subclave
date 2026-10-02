//! Argon2id key derivation and the shared KDF header.
//!
//! One KDF for the vault file, the sync keyfile and backups. Parameters over
//! the caps are refused BEFORE any derivation runs, so a hostile file cannot
//! hang the app with a 100 GiB memory request; every file is written with the
//! current defaults.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const DEFAULT_MEMORY_KIB: u32 = 65536;
pub const DEFAULT_ITERATIONS: u32 = 3;
pub const DEFAULT_PARALLELISM: u32 = 4;
pub const MAX_MEMORY_KIB: u32 = 262144;
pub const MAX_ITERATIONS: u32 = 10;
pub const MAX_PARALLELISM: u32 = 8;
pub const KDF_NAME: &str = "argon2id";
pub const SALT_LEN: usize = 16;
pub const KEY_LEN: usize = 32;

/// The KDF header stored in the vault file, the sync keyfile and backups.
/// `memoryKiB` is renamed explicitly: serde camelCase alone yields
/// `memoryKib`, which would change the on-disk bytes and the header AAD.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Argon2Params {
    pub name: String,
    #[serde(rename = "memoryKiB")]
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
    /// Base64, 16 bytes.
    pub salt: String,
}

/// Refuse a header before deriving from it. Called on every read path.
pub fn check_params(p: &Argon2Params) -> Result<(), String> {
    if p.name != KDF_NAME {
        return Err("vault: unsupported key derivation".to_string());
    }
    if p.memory_kib > MAX_MEMORY_KIB
        || p.iterations > MAX_ITERATIONS
        || p.parallelism > MAX_PARALLELISM
    {
        return Err("vault: kdf parameters exceed the accepted maximum".to_string());
    }
    Ok(())
}

/// Random salt, current defaults. Errors only if the system RNG is dead.
pub fn fresh_params() -> Result<Argon2Params, String> {
    let mut salt = [0u8; SALT_LEN];
    SystemRandom::new()
        .fill(&mut salt)
        .map_err(|_| "vault: random salt failed".to_string())?;
    Ok(Argon2Params {
        name: KDF_NAME.to_string(),
        memory_kib: DEFAULT_MEMORY_KIB,
        iterations: DEFAULT_ITERATIONS,
        parallelism: DEFAULT_PARALLELISM,
        salt: B64.encode(salt),
    })
}

/// Derive the 32-byte vault key. Runs [`check_params`] first, so the caps hold
/// on every caller.
pub fn derive_key(password: &str, params: &Argon2Params) -> Result<Zeroizing<[u8; 32]>, String> {
    check_params(params)?;
    let salt = B64
        .decode(&params.salt)
        .map_err(|_| "vault: key derivation failed".to_string())?;
    let argon = Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(
            params.memory_kib,
            params.iterations,
            params.parallelism,
            Some(KEY_LEN),
        )
        .map_err(|_| "vault: key derivation failed".to_string())?,
    );
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(password.as_bytes(), &salt, &mut *out)
        .map_err(|_| "vault: key derivation failed".to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Argon2Params {
        Argon2Params {
            name: KDF_NAME.into(),
            memory_kib: DEFAULT_MEMORY_KIB,
            iterations: DEFAULT_ITERATIONS,
            parallelism: DEFAULT_PARALLELISM,
            salt: B64.encode([0u8; SALT_LEN]),
        }
    }

    #[test]
    fn serde_uses_memorykib_spelling() {
        let json = serde_json::to_string(&params()).unwrap();
        assert!(json.contains("\"memoryKiB\":65536"), "{json}");
        assert!(!json.contains("memoryKib\""));
    }

    #[test]
    fn check_params_refuses_bad_name_and_over_caps() {
        assert_eq!(
            check_params(&Argon2Params {
                name: "pbkdf2".into(),
                ..params()
            })
            .unwrap_err(),
            "vault: unsupported key derivation"
        );
        assert_eq!(
            check_params(&Argon2Params {
                memory_kib: MAX_MEMORY_KIB + 1,
                ..params()
            })
            .unwrap_err(),
            "vault: kdf parameters exceed the accepted maximum"
        );
        assert_eq!(
            check_params(&Argon2Params {
                iterations: MAX_ITERATIONS + 1,
                ..params()
            })
            .unwrap_err(),
            "vault: kdf parameters exceed the accepted maximum"
        );
        assert_eq!(
            check_params(&Argon2Params {
                parallelism: MAX_PARALLELISM + 1,
                ..params()
            })
            .unwrap_err(),
            "vault: kdf parameters exceed the accepted maximum"
        );
        assert!(check_params(&params()).is_ok());
    }

    #[test]
    fn derive_key_is_deterministic_and_salt_sensitive() {
        let a = derive_key("pass", &params()).unwrap();
        let b = derive_key("pass", &params()).unwrap();
        assert_eq!(&*a, &*b);
        let mut other = params();
        other.salt = B64.encode([1u8; SALT_LEN]);
        let c = derive_key("pass", &other).unwrap();
        assert_ne!(&*a, &*c);
    }
}
