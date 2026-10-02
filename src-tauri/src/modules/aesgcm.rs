//! AES-256-GCM under a key the caller already holds.
//!
//! A leaf on purpose: the vault file, the sync keyfile (a later module) and the
//! backup format all need the same seal/open pair, and none of them should
//! inherit the others' dependencies. This module takes the 32 key bytes and
//! nothing else. Key derivation lives above it.
//!
//! The nonce is drawn inside [`seal_with_key`] rather than taken as a parameter
//! so no caller can supply a constant one, and each key instance seals exactly
//! one message (see [`OneNonce`]). Every authenticated failure in
//! [`open_with_key`] is reported with ONE message on purpose: distinguishing
//! wrong key from tampered ciphertext tells an attacker which guess was closer,
//! and none of them is separately actionable for the user.

use ring::{
    aead::{self, Aad, BoundKey, Nonce, NonceSequence, UnboundKey, AES_256_GCM, NONCE_LEN},
    error::Unspecified,
    rand::{SecureRandom, SystemRandom},
};
use zeroize::Zeroizing;

/// `ring`'s sealing API consumes a nonce sequence; each message is sealed under
/// its own key INSTANCE, so the sequence yields the single random nonce and
/// then refuses. Refusing matters: reusing a nonce under the same key breaks
/// GCM completely. The scope is one use per instance, not one use per key: a
/// caller that seals many messages under one long-lived key is responsible for
/// a fresh nonce per message.
pub(crate) struct OneNonce(Option<[u8; NONCE_LEN]>);

impl NonceSequence for OneNonce {
    fn advance(&mut self) -> Result<Nonce, Unspecified> {
        self.0
            .take()
            .map(Nonce::assume_unique_for_key)
            .ok_or(Unspecified)
    }
}

/// Seal `plaintext` under `key` with a freshly drawn random nonce and `aad` as
/// the associated data. Returns the nonce and `ciphertext || tag`.
///
/// Failures here are unreachable in practice (a dead system RNG), so they carry
/// no caller prefix: there is nothing for a user to tell apart.
pub(crate) fn seal_with_key(
    key: &[u8; 32],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<([u8; NONCE_LEN], Vec<u8>), String> {
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| "aes-gcm: random nonce failed".to_string())?;

    let unbound = UnboundKey::new(&AES_256_GCM, key).map_err(|_| "aes-gcm: bad key".to_string())?;
    let mut sealing = aead::SealingKey::new(unbound, OneNonce(Some(nonce)));

    // seal_in_place_append_tag appends the 16-byte auth tag, so `buf` ends up
    // as ciphertext||tag - which is exactly what open_in_place expects back.
    let mut buf = plaintext.to_vec();
    sealing
        .seal_in_place_append_tag(Aad::from(aad), &mut buf)
        .map_err(|_| "aes-gcm: encryption failed".to_string())?;
    Ok((nonce, buf))
}

/// Open what [`seal_with_key`] produced. `buf` is `ciphertext || tag`.
///
/// Returns the plaintext wrapped in [`Zeroizing`] with no extra copy: after
/// `open_in_place` succeeds, `buf` holds the plaintext in its prefix, so the
/// buffer itself is truncated and wrapped rather than copied out.
pub(crate) fn open_with_key(
    key: &[u8; 32],
    aad: &[u8],
    nonce: &[u8; NONCE_LEN],
    mut buf: Vec<u8>,
    prefix: &str,
) -> Result<Zeroizing<Vec<u8>>, String> {
    let unbound = UnboundKey::new(&AES_256_GCM, key).map_err(|_| format!("{prefix}: bad key"))?;
    let mut opening = aead::OpeningKey::new(unbound, OneNonce(Some(*nonce)));
    let plain = opening
        .open_in_place(Aad::from(aad), &mut buf)
        .map_err(|_| format!("{prefix}: wrong passphrase, or the file is corrupt"))?;
    let plain_len = plain.len();
    buf.truncate(plain_len);
    Ok(Zeroizing::new(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];

    #[test]
    fn roundtrip_with_and_without_aad() {
        for aad in [&b""[..], &b"header bytes"[..]] {
            let (nonce, sealed) = seal_with_key(&KEY, aad, b"secret payload").unwrap();
            let opened = open_with_key(&KEY, aad, &nonce, sealed, "test").unwrap();
            assert_eq!(&*opened, b"secret payload");
        }
    }

    #[test]
    fn tampering_the_aad_fails_open() {
        let (nonce, sealed) = seal_with_key(&KEY, b"header-a", b"secret").unwrap();
        let err = open_with_key(&KEY, b"header-b", &nonce, sealed, "test").unwrap_err();
        assert!(err.contains("wrong passphrase, or the file is corrupt"));
    }

    #[test]
    fn wrong_key_gives_the_one_opaque_message() {
        let other = [9u8; 32];
        let (nonce, sealed) = seal_with_key(&KEY, b"", b"secret").unwrap();
        let err = open_with_key(&other, b"", &nonce, sealed, "vault").unwrap_err();
        assert_eq!(err, "vault: wrong passphrase, or the file is corrupt");
    }
}
