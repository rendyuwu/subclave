//! Pairing and the authentication handshake primitives.
//!
//! Every literal in this file is mirrored by `extension/src/lib/auth.ts`, and
//! the pinned vectors in both test suites make a drift redden both at once.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ring::hmac;

/// Domain-separation contexts for the two HMAC proofs, and the pairing code.
pub const PAIR_CONTEXT: &[u8] = b"subclave-pair-v1";
pub const APP_CONTEXT: &[u8] = b"subclave-app-v1";
pub const EXT_CONTEXT: &[u8] = b"subclave-ext-v1";

/// The six-digit code the user compares. First 4 bytes of
/// `SHA-256(PAIR_CONTEXT || pairNonce)` as a big-endian u32, mod 1,000,000,
/// zero-padded to six digits.
pub fn pairing_code(pair_nonce: &[u8]) -> String {
    let mut input = Vec::with_capacity(PAIR_CONTEXT.len() + pair_nonce.len());
    input.extend_from_slice(PAIR_CONTEXT);
    input.extend_from_slice(pair_nonce);
    let digest = ring::digest::digest(&ring::digest::SHA256, &input);
    let bytes = digest.as_ref();
    let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 1_000_000;
    format!("{value:06}")
}

fn mac(secret: &[u8], context: &[u8], first: &[u8], second: &[u8]) -> [u8; 32] {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let mut input = Vec::with_capacity(context.len() + first.len() + second.len());
    input.extend_from_slice(context);
    input.extend_from_slice(first);
    input.extend_from_slice(second);
    let tag = hmac::sign(&key, &input);
    let mut out = [0u8; 32];
    out.copy_from_slice(tag.as_ref());
    out
}

/// The app's proof over `APP_CONTEXT || extNonce || appNonce`.
pub fn app_proof(secret: &[u8], ext_nonce: &[u8], app_nonce: &[u8]) -> [u8; 32] {
    mac(secret, APP_CONTEXT, ext_nonce, app_nonce)
}

/// The extension's proof over `EXT_CONTEXT || appNonce || extNonce`. The app
/// sends it (as `app_proof`) and verifies the extension's; this direction is
/// kept for the parity vectors `auth.ts` shares.
#[allow(dead_code)]
pub fn ext_proof(secret: &[u8], app_nonce: &[u8], ext_nonce: &[u8]) -> [u8; 32] {
    mac(secret, EXT_CONTEXT, app_nonce, ext_nonce)
}

/// Constant-time verification of [`app_proof`]. Unused in the channel (the
/// app only verifies the extension's direction); kept for parity.
#[allow(dead_code)]
pub fn verify_app_proof(secret: &[u8], ext_nonce: &[u8], app_nonce: &[u8], proof: &[u8]) -> bool {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let mut input = Vec::with_capacity(APP_CONTEXT.len() + ext_nonce.len() + app_nonce.len());
    input.extend_from_slice(APP_CONTEXT);
    input.extend_from_slice(ext_nonce);
    input.extend_from_slice(app_nonce);
    hmac::verify(&key, &input, proof).is_ok()
}

/// Constant-time verification of [`ext_proof`].
pub fn verify_ext_proof(secret: &[u8], app_nonce: &[u8], ext_nonce: &[u8], proof: &[u8]) -> bool {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let mut input = Vec::with_capacity(EXT_CONTEXT.len() + app_nonce.len() + ext_nonce.len());
    input.extend_from_slice(EXT_CONTEXT);
    input.extend_from_slice(app_nonce);
    input.extend_from_slice(ext_nonce);
    hmac::verify(&key, &input, proof).is_ok()
}

/// 32 random bytes from the system RNG, standard base64.
pub fn random_nonce_b64() -> String {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .expect("browser: system rng failed");
    STANDARD.encode(bytes)
}

/// Decode a base64 nonce, requiring exactly 32 bytes. `None` is a bad request.
pub fn nonce_bytes(b64: &str) -> Option<Vec<u8>> {
    let bytes = STANDARD.decode(b64).ok()?;
    (bytes.len() == 32).then_some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_code_matches_the_pinned_vector() {
        assert_eq!(pairing_code(&[0u8; 32]), "645339");
    }

    #[test]
    fn pairing_code_is_always_six_digits() {
        for seed in 0u8..64 {
            let mut nonce = [0u8; 32];
            nonce[0] = seed;
            nonce[31] = seed.wrapping_mul(7);
            let code = pairing_code(&nonce);
            assert_eq!(code.len(), 6, "code {code} from seed {seed}");
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn hmac_parity_vectors() {
        let app = app_proof(&[1u8; 32], &[2u8; 32], &[3u8; 32]);
        assert_eq!(
            hex(&app),
            "eabf7b81f143227129cca156b66e04d3d46c2f24b605c595b5f3062b858e5d8e"
        );
        let ext = ext_proof(&[1u8; 32], &[3u8; 32], &[2u8; 32]);
        assert_eq!(
            hex(&ext),
            "e82b14c434f1915828d86889b56358cea216b521862ee312595f251f9ed7d159"
        );
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn verify_accepts_the_right_proof_only() {
        let secret = [1u8; 32];
        let ext_nonce = [2u8; 32];
        let app_nonce = [3u8; 32];
        assert!(verify_app_proof(
            &secret,
            &ext_nonce,
            &app_nonce,
            &app_proof(&secret, &ext_nonce, &app_nonce)
        ));
        assert!(verify_ext_proof(
            &secret,
            &app_nonce,
            &ext_nonce,
            &ext_proof(&secret, &app_nonce, &ext_nonce)
        ));

        // A one-byte-different MAC.
        let mut tampered = app_proof(&secret, &ext_nonce, &app_nonce);
        tampered[0] ^= 1;
        assert!(!verify_app_proof(
            &secret, &ext_nonce, &app_nonce, &tampered
        ));
        // A truncated MAC.
        let full = app_proof(&secret, &ext_nonce, &app_nonce);
        assert!(!verify_app_proof(
            &secret,
            &ext_nonce,
            &app_nonce,
            &full[..16]
        ));
        // The proof over the wrong message order.
        assert!(!verify_app_proof(
            &secret,
            &app_nonce,
            &ext_nonce,
            &app_proof(&secret, &ext_nonce, &app_nonce)
        ));
        // The other direction's proof.
        assert!(!verify_app_proof(
            &secret,
            &ext_nonce,
            &app_nonce,
            &ext_proof(&secret, &app_nonce, &ext_nonce)
        ));
    }

    #[test]
    fn nonce_bytes_requires_exactly_32_bytes() {
        let good = random_nonce_b64();
        assert_eq!(nonce_bytes(&good).map(|b| b.len()), Some(32));
        assert_eq!(nonce_bytes("not base64!"), None);
        assert_eq!(nonce_bytes(&STANDARD.encode([0u8; 16])), None);
        assert_eq!(nonce_bytes(""), None);
    }
}
