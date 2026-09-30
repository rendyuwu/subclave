//! TOTP: parse `otpauth://` URIs, compute RFC 6238 codes.
//!
//! `ring` HMAC plus [`data_encoding`] base32 is the whole dependency surface;
//! `totp-rs` would bring a QR stack for nothing. Errors carry the `totp:`
//! prefix and are shown verbatim by the editor when it refuses a URI.

use data_encoding::{BASE32, BASE32_NOPAD};
use ring::hmac::{self, HMAC_SHA1_FOR_LEGACY_USE_ONLY, HMAC_SHA256, HMAC_SHA512};
use tauri::Manager;
use url::Url;

#[derive(Clone, Debug, PartialEq)]
pub struct TotpUri {
    pub secret: Vec<u8>,
    pub algo: TotpAlgo,
    pub digits: u32,
    pub period: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TotpAlgo {
    Sha1,
    Sha256,
    Sha512,
}

impl TotpAlgo {
    fn hmac(self) -> hmac::Algorithm {
        match self {
            // RFC 4226 fixes SHA-1 for TOTP, and `ring` exposes no other
            // SHA-1 HMAC name than this legacy constant.
            // (`HMAC_SHA1_FOR_LEGACY_USE_ONLY`, ring 0.17, TotpAlgo::hmac.)
            TotpAlgo::Sha1 => HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            TotpAlgo::Sha256 => HMAC_SHA256,
            TotpAlgo::Sha512 => HMAC_SHA512,
        }
    }
}

/// Parse an otpauth URI: scheme `otpauth`, host `totp`, a required base32
/// `secret`, `algorithm` (default SHA1), `digits` (default 6, 6..=8) and
/// `period` (default 30, positive).
pub fn parse(uri: &str) -> Result<TotpUri, String> {
    let url = Url::parse(uri).map_err(|_| "totp: not an otpauth URI".to_string())?;
    if url.scheme() != "otpauth" || url.host_str() != Some("totp") {
        return Err("totp: not an otpauth URI".to_string());
    }
    let secret_param = url
        .query_pairs()
        .find(|(k, _)| k == "secret")
        .map(|(_, v)| v.to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "totp: not an otpauth URI".to_string())?;
    // Decode with the padded alphabet first (the spec's URIs carry `=`), then
    // the unpadded one on the uppercased, padding-stripped bytes: a stripped
    // padded input can end on a length that is not a multiple of 8, which the
    // padded spec rejects and `BASE32_NOPAD` accepts, and a lowercase secret
    // must not be refused just because the fallback forgot to uppercase it.
    let upper = secret_param.to_uppercase();
    let secret = BASE32
        .decode(upper.as_bytes())
        .or_else(|_| BASE32_NOPAD.decode(upper.trim_end_matches('=').as_bytes()))
        .map_err(|_| "totp: bad base32 secret".to_string())?;
    let algo = match url
        .query_pairs()
        .find(|(k, _)| k == "algorithm")
        .map(|(_, v)| v.to_uppercase())
    {
        None => TotpAlgo::Sha1,
        Some(a) => match a.as_str() {
            "SHA1" => TotpAlgo::Sha1,
            "SHA256" => TotpAlgo::Sha256,
            "SHA512" => TotpAlgo::Sha512,
            other => return Err(format!("totp: unknown algorithm \"{other}\"")),
        },
    };
    let digits = match url
        .query_pairs()
        .find(|(k, _)| k == "digits")
        .map(|(_, v)| v.parse::<u32>())
    {
        None => 6,
        Some(Ok(d)) => {
            if !(6..=8).contains(&d) {
                return Err("totp: digits must be 6 to 8".to_string());
            }
            d
        }
        Some(Err(_)) => return Err("totp: digits must be 6 to 8".to_string()),
    };
    let period = match url
        .query_pairs()
        .find(|(k, _)| k == "period")
        .map(|(_, v)| v.parse::<u32>())
    {
        None => 30,
        Some(Ok(p)) if p > 0 => p,
        _ => return Err("totp: period must be positive".to_string()),
    };
    Ok(TotpUri {
        secret,
        algo,
        digits,
        period,
    })
}

/// The HOTP value per RFC 4226: HMAC over the big-endian counter, dynamic
/// truncation, `digits` zero-padded.
pub fn code(t: &TotpUri, unix_seconds: u64) -> String {
    let counter = (unix_seconds / u64::from(t.period)).to_be_bytes();
    let key = hmac::Key::new(t.algo.hmac(), &t.secret);
    let digest = hmac::sign(&key, &counter);
    let offset = (digest.as_ref()[digest.as_ref().len() - 1] & 0x0f) as usize;
    let bin = u32::from_be_bytes([
        digest.as_ref()[offset] & 0x7f,
        digest.as_ref()[offset + 1],
        digest.as_ref()[offset + 2],
        digest.as_ref()[offset + 3],
    ]);
    let modulus = 10u64.pow(t.digits);
    format!(
        "{:0width$}",
        bin as u64 % modulus,
        width = t.digits as usize
    )
}

/// Seconds until the current code expires.
pub fn remaining(t: &TotpUri, unix_seconds: u64) -> u32 {
    t.period - (unix_seconds % u64::from(t.period)) as u32
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TotpCode {
    pub code: String,
    pub period: u32,
    pub remaining: u32,
}

/// The current code for one entry, for the detail pane. The URI itself is
/// only reachable through `vault_entry_reveal`.
#[tauri::command]
pub async fn totp_code(app: tauri::AppHandle, id: String) -> Result<TotpCode, String> {
    let task_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = task_app.state::<crate::modules::vault::VaultState>();
        totp_code_inner(&state, &id)
    })
    .await
    .map_err(|e| format!("totp: task failed: {e}"))?;
    crate::modules::vault::drain_auto_lock(&app);
    result
}

fn totp_code_inner(
    state: &crate::modules::vault::VaultState,
    id: &str,
) -> Result<TotpCode, String> {
    let guard = state.access()?;
    let unlocked = guard
        .as_ref()
        .ok_or_else(|| crate::modules::vault::LOCKED_ERR.to_string())?;
    let entry = unlocked
        .payload
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| "vault: no such entry".to_string())?;
    let uri = entry
        .totp
        .clone()
        .ok_or_else(|| "vault: no TOTP on this entry".to_string())?;
    let parsed = parse(&uri)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Ok(TotpCode {
        code: code(&parsed, now),
        period: parsed.period,
        remaining: remaining(&parsed, now),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The RFC 6238 test vectors: ASCII seeds, the 8-digit column for all
    /// three algorithms, and the 6-digit forms.
    fn uri(seed: &[u8], algo: &str) -> String {
        format!(
            "otpauth://totp/Test?secret={}&algorithm={}&digits=8",
            BASE32.encode(seed),
            algo
        )
    }

    /// The RFC 6238 test vectors: ASCII seeds of 20/32/64 bytes, the 8-digit
    /// column for all three algorithms. The table gives one column per seed,
    /// so each seed carries its own expected list.
    fn vectors(algo: &str, expected_per_seed: &[[&str; 6]]) {
        let seeds: [&[u8]; 3] = [
            b"12345678901234567890",
            b"12345678901234567890123456789012",
            b"1234567890123456789012345678901234567890123456789012345678901234",
        ];
        let times = [
            59u64,
            1111111109,
            1111111111,
            1234567890,
            2000000000,
            20000000000,
        ];
        for (seed, expected) in seeds.iter().zip(expected_per_seed) {
            let parsed = parse(&uri(seed, algo)).unwrap();
            assert_eq!(parsed.digits, 8);
            for (time, want) in times.iter().zip(expected) {
                assert_eq!(
                    code(&parsed, *time),
                    *want,
                    "{algo} seed {} t={}",
                    seed.len(),
                    time
                );
            }
        }
    }

    #[test]
    fn rfc6238_sha1_vectors() {
        vectors(
            "SHA1",
            &[
                [
                    "94287082", "07081804", "14050471", "89005924", "69279037", "65353130",
                ],
                [
                    "97599872", "82138967", "32201283", "23012961", "26931087", "03573920",
                ],
                [
                    "14779409", "36110091", "18372631", "33973530", "50155042", "50487110",
                ],
            ],
        );
    }

    #[test]
    fn rfc6238_sha256_vectors() {
        vectors(
            "SHA256",
            &[
                [
                    "32247374", "34756375", "74584430", "42829826", "78428693", "24142410",
                ],
                [
                    "46119246", "68084774", "67062674", "91819424", "90698825", "77737706",
                ],
                [
                    "73786473", "24171431", "93080941", "61384964", "13269708", "24124209",
                ],
            ],
        );
    }

    #[test]
    fn rfc6238_sha512_vectors() {
        vectors(
            "SHA512",
            &[
                [
                    "69342147", "63049338", "54380122", "76671578", "56464532", "69481994",
                ],
                [
                    "53754366", "81199770", "70247269", "62618035", "11046892", "83136826",
                ],
                [
                    "90693936", "25091201", "99943326", "93441116", "38618901", "47863826",
                ],
            ],
        );
    }

    #[test]
    fn six_digit_forms_match_the_vector_low_digits() {
        // The 6-digit column of the same vectors, leading zeros kept: the
        // low six digits of the 8-digit value per seed and time.
        let cases = [
            ("SHA1", 59u64, "287082"),
            ("SHA1", 1111111109, "081804"),
            ("SHA1", 1111111111, "050471"),
            ("SHA1", 1234567890, "005924"),
            ("SHA1", 2000000000, "279037"),
            ("SHA1", 20000000000, "353130"),
            ("SHA256", 59, "247374"),
            ("SHA256", 1111111109, "756375"),
            ("SHA256", 1111111111, "584430"),
            ("SHA256", 1234567890, "829826"),
            ("SHA256", 2000000000, "428693"),
            ("SHA256", 20000000000, "142410"),
            ("SHA512", 59, "342147"),
            ("SHA512", 1111111109, "049338"),
            ("SHA512", 1111111111, "380122"),
            ("SHA512", 1234567890, "671578"),
            ("SHA512", 2000000000, "464532"),
            ("SHA512", 20000000000, "481994"),
        ];
        for (algo, time, want) in cases {
            let parsed = parse(&format!(
                "otpauth://totp/T?secret={}&algorithm={}&digits=6",
                BASE32.encode(b"12345678901234567890"),
                algo
            ))
            .unwrap();
            assert_eq!(code(&parsed, time), want, "{algo} t={time}");
        }
    }

    #[test]
    fn parser_errors() {
        assert_eq!(
            parse("https://totp/x").unwrap_err(),
            "totp: not an otpauth URI"
        );
        let bad_b32 = "otpauth://totp/T?secret=!!!notbase32!!!";
        assert_eq!(parse(bad_b32).unwrap_err(), "totp: bad base32 secret");
        let no_secret = "otpauth://totp/T?algorithm=SHA1";
        assert_eq!(parse(no_secret).unwrap_err(), "totp: not an otpauth URI");
        let empty_secret = "otpauth://totp/T?secret=&algorithm=SHA1";
        assert_eq!(parse(empty_secret).unwrap_err(), "totp: not an otpauth URI");
        let bad_algo = format!(
            "otpauth://totp/T?secret={}&algorithm=MD5",
            BASE32.encode(b"abc")
        );
        assert_eq!(
            parse(&bad_algo).unwrap_err(),
            "totp: unknown algorithm \"MD5\""
        );
        let few = format!("otpauth://totp/T?secret={}&digits=5", BASE32.encode(b"abc"));
        assert_eq!(parse(&few).unwrap_err(), "totp: digits must be 6 to 8");
        let many = format!("otpauth://totp/T?secret={}&digits=9", BASE32.encode(b"abc"));
        assert_eq!(parse(&many).unwrap_err(), "totp: digits must be 6 to 8");
        let zero = format!("otpauth://totp/T?secret={}&period=0", BASE32.encode(b"abc"));
        assert_eq!(parse(&zero).unwrap_err(), "totp: period must be positive");
    }

    #[test]
    fn defaults_and_remaining() {
        let parsed = parse(&format!(
            "otpauth://totp/T?secret={}",
            BASE32.encode(b"abc")
        ))
        .unwrap();
        assert_eq!(parsed.algo, TotpAlgo::Sha1);
        assert_eq!(parsed.digits, 6);
        assert_eq!(parsed.period, 30);
        assert_eq!(remaining(&parsed, 0), 30);
        assert_eq!(remaining(&parsed, 45), 15);
        // Unpadded base32 decodes too. GEZDGNBV is the unpadded form of
        // "12345".
        let unpadded = parse("otpauth://totp/T?secret=GEZDGNBV").unwrap();
        assert_eq!(unpadded.secret, b"12345".to_vec());
        // A lowercase secret whose stripped length is not a multiple of 8
        // reaches the NOPAD fallback and must decode there.
        let lowercase = parse("otpauth://totp/T?secret=gezdgnbvgy3tqojqgezdgnbvgy").unwrap();
        assert_eq!(lowercase.secret, b"1234567890123456".to_vec());
    }
}
