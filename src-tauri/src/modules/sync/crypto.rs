//! Everything the remote sees is sealed here, and nothing here talks to the
//! remote.
//!
//! PURE, in the same sense `src-tauri/src/modules/sync/model.rs` is: no
//! network, no filesystem, no keychain. It takes a passphrase or a keyfile and
//! returns bytes.
//!
//! The chain, top to bottom:
//!
//! ```text
//! passphrase + random salt, Argon2id                     ->  key-encryption key
//! root key (32 random bytes), AES-256-GCM under the KEK  ->  wrapped, in the keyfile
//! root key, HKDF-SHA256 Expand with two labels           ->  data key, name key
//! data key, AES-256-GCM with a fresh nonce               ->  each record
//! name key, HMAC-SHA256                                  ->  each object name
//! ```
//!
//! ONE WRAPPED ROOT, TWO DERIVED SUBKEYS, rather than two independently
//! wrapped keys: the keyfile then holds one blob, and changing the passphrase
//! rewraps one thing. `ring::hkdf` rather than a hand-rolled HMAC
//! construction, because it is a named primitive in a dependency this crate
//! already pins, so there is no ad-hoc crypto for a reviewer to verify.
//!
//! WHY THE NAME KEY EXISTS AT ALL: an object's name is a path segment on
//! somebody else's storage. Deriving it from the record's `kind` and `id`
//! directly would publish the whole inventory's shape in the file listing.
//! HMAC under a key the remote does not have makes the listing opaque while
//! keeping the name deterministic, so two devices independently compute the
//! same name for the same record.
//!
//! NO PATHS HERE. The layout `<prefix>/v1/keyfile` and
//! `<prefix>/v1/obj/<name>` belongs to
//! `src-tauri/src/modules/sync/engine/layout.rs`, the sync layer above the
//! provider, which composes the key a provider receives; this module produces
//! the `<name>` half and the keyfile struct and builds no path. A provider in
//! `src-tauri/src/modules/sync/provider.rs` sees keys and bytes and has no idea
//! what a record is, so the `v1` segment (the wire format's version expressed in
//! the object namespace) cannot be its business.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use ring::{
    aead::NONCE_LEN,
    hkdf, hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::modules::aesgcm::{open_with_key, seal_with_key};
use crate::modules::vault::file::header_aad;
use crate::modules::vault::kdf::{check_params, derive_key, fresh_params, Argon2Params};

/// The keyfile's own version, DELIBERATELY SEPARATE from `WIRE_VERSION` in
/// `src-tauri/src/modules/sync/model.rs` even though both are 1 today.
///
/// Welding them together is a data-loss trapdoor. The root key exists in
/// exactly one place, `SyncKeyfile::wrapped`, so the day the ENVELOPE shape
/// changes for an envelope reason and `WIRE_VERSION` goes to 2, a shared
/// constant would have `open_keyfile` refuse every keyfile already written and
/// every object on every remote would become permanently undecryptable. The
/// two version the two things, and they move independently.
const KEYFILE_VERSION: u32 = 1;

/// The `format` string every keyfile this build writes carries, and the one
/// `open_keyfile` accepts. Anything else is a folder this app did not write.
const KEYFILE_FORMAT: &str = "subclave-sync";

/// The two HKDF labels. DISTINCT is the whole requirement, they are what make
/// the two subkeys independent, and they are spelled out rather than built
/// from a shared stem so a refactor cannot accidentally collapse them.
const DATA_LABEL: &[u8] = b"subclave-sync-record-key";
const NAME_LABEL: &[u8] = b"subclave-sync-object-name-key";

/// One message for every way opening can fail.
///
/// Wrong passphrase, a flipped byte, a truncated field, base64 that is not
/// base64: all the same sentence, because distinguishing them tells an
/// attacker which guess was closer and none of them is separately actionable.
/// `a_wrong_passphrase_and_a_corrupt_keyfile_are_indistinguishable` is what
/// notices when that stops being true.
const OPAQUE_FAILURE: &str = "sync: wrong sync passphrase, or the keyfile is corrupt";

/// What a remote that is not a Subclave sync folder answers with, whether its
/// keyfile carries another `format` or does not parse as a keyfile at all.
///
/// `pub(crate)` because the sync engine refuses bytes that do not parse a
/// keyfile before it gets this far, and one spelling of the sentence is the
/// point.
pub(crate) const NOT_A_KEYFILE: &str = "sync: not a Subclave sync folder";

/// A keyfile written by a build that knows a later `KEYFILE_VERSION`. Named
/// rather than opaque because it depends on nothing the user typed, so it is
/// not a guessing oracle.
const NEWER_KEYFILE: &str = "sync: this keyfile was written by a newer Subclave";

/// A keyfile whose KDF parameters ride over the caps in
/// `src-tauri/src/modules/vault/kdf.rs`. Refused before any derivation runs,
/// so a hostile keyfile cannot hang the app with a huge Argon2id request.
const CAPS_FAILURE: &str = "sync: the keyfile's kdf parameters exceed the accepted maximum";

/// The two working keys, held only in memory and never serialized.
///
/// Both halves are 32 random-derived bytes, so SWAPPING them breaks nothing
/// observable while silently discarding the separation this design paid for.
/// That is what `each_path_uses_its_own_subkey` exists to pin.
///
/// NO `Debug`, deliberately: a derived one prints key material, and the one
/// place that would happen is a log line or a panic message written by
/// somebody who did not think about it.
pub struct SyncKeys {
    pub(crate) data: Zeroizing<[u8; 32]>,
    pub(crate) name: Zeroizing<[u8; 32]>,
}

/// What sits at the root of the remote, and the only thing a second device
/// needs besides the passphrase.
///
/// Every field is `pub` because the provider has to serialize this to the
/// remote verbatim. None of it is secret: the salt and nonce are public by
/// construction, and `wrapped` is the root key under a passphrase-derived key.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncKeyfile {
    /// Checked, not assumed. Every other keyfile check depends on it.
    pub format: String,
    pub v: u32,
    pub kdf: Argon2Params,
    /// Base64, 12 bytes.
    pub nonce: String,
    /// Base64: AES-256-GCM(KEK, 32-byte root key).
    pub wrapped: String,
}

/// One sealed record, as it is stored. The nonce is per record and public.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SealedRecord {
    pub nonce: String,
    pub ciphertext: String,
}

/// HKDF-Expand the root into one labelled subkey.
///
/// Expand rather than Extract-then-Expand because the root is already 32
/// uniformly random bytes from the system RNG, which is exactly the input
/// `Prk` wants. Extract would add a step that buys nothing here.
fn subkey(root: &[u8; 32], label: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    let prk = hkdf::Prk::new_less_safe(hkdf::HKDF_SHA256, root);
    let mut out = Zeroizing::new([0u8; 32]);
    prk.expand(&[label], hkdf::HKDF_SHA256)
        .and_then(|okm| okm.fill(&mut *out))
        .map_err(|_| "sync: key expansion failed".to_string())?;
    Ok(out)
}

/// The two subkeys the root expands into, the only working keys this module
/// ever holds.
pub fn expand_root(root: &[u8; 32]) -> Result<SyncKeys, String> {
    Ok(SyncKeys {
        data: subkey(root, DATA_LABEL)?,
        name: subkey(root, NAME_LABEL)?,
    })
}

/// The keyfile header as associated data: the shared header AAD
/// ([`header_aad`]) over the keyfile's own format and version.
fn kek_aad(kdf: &Argon2Params) -> Vec<u8> {
    header_aad(KEYFILE_FORMAT, KEYFILE_VERSION, kdf)
}

/// Mint a brand new root key and wrap it under `passphrase`.
///
/// Returns the root beside the keyfile. A caller that has to PERSIST the root
/// cannot recover it from [`SyncKeys`], whose halves are expanded subkeys, so
/// re-opening what this just wrote would spend a second KDF run for nothing.
///
/// An empty passphrase is refused HERE rather than in a caller, so the
/// guarantee holds no matter which caller reaches this.
pub fn new_keyfile_with_root(
    passphrase: &str,
) -> Result<(SyncKeyfile, Zeroizing<[u8; 32]>), String> {
    if passphrase.is_empty() {
        return Err("sync: a passphrase is required".into());
    }
    let kdf = fresh_params()?;
    let mut root = Zeroizing::new([0u8; 32]);
    SystemRandom::new()
        .fill(&mut *root)
        .map_err(|_| "sync: random root key failed".to_string())?;

    let kek = derive_key(passphrase, &kdf)?;
    let (nonce, wrapped) = seal_with_key(&kek, &kek_aad(&kdf), &root[..])?;

    Ok((
        SyncKeyfile {
            format: KEYFILE_FORMAT.into(),
            v: KEYFILE_VERSION,
            kdf,
            nonce: B64.encode(nonce),
            wrapped: B64.encode(&wrapped),
        },
        root,
    ))
}

/// [`new_keyfile_with_root`] with the root expanded into its two subkeys, for a
/// caller that is about to use them rather than store the root.
pub fn new_keyfile(passphrase: &str) -> Result<(SyncKeyfile, SyncKeys), String> {
    let (keyfile, root) = new_keyfile_with_root(passphrase)?;
    Ok((keyfile, expand_root(&root)?))
}

/// Unwrap the root key out of a keyfile.
///
/// `format`, `v` and the KDF caps are checked BEFORE anything is derived, and
/// those three are the only failures here that name what went wrong: an
/// unreadable keyfile is a "this is not ours" or "this device runs an older
/// build" message, and none of them depends on the passphrase, so none is a
/// guessing oracle. Every other failure below (wrong passphrase, tampered
/// `wrapped`, malformed base64, a bad salt) is `OPAQUE_FAILURE`.
pub fn open_keyfile_root(
    kf: &SyncKeyfile,
    passphrase: &str,
) -> Result<Zeroizing<[u8; 32]>, String> {
    if kf.format != KEYFILE_FORMAT {
        return Err(NOT_A_KEYFILE.to_string());
    }
    if kf.v != KEYFILE_VERSION {
        return Err(NEWER_KEYFILE.to_string());
    }
    check_params(&kf.kdf).map_err(|_| CAPS_FAILURE.to_string())?;

    let nonce = decode_nonce(&kf.nonce)?;
    let wrapped = B64.decode(&kf.wrapped).map_err(|_| OPAQUE_FAILURE)?;

    let kek = derive_key(passphrase, &kf.kdf).map_err(|_| OPAQUE_FAILURE)?;
    let root: [u8; 32] = open_with_key(&kek, &kek_aad(&kf.kdf), &nonce, wrapped, "sync")
        .map_err(|_| OPAQUE_FAILURE)?
        .as_slice()
        .try_into()
        .map_err(|_| OPAQUE_FAILURE)?;
    Ok(Zeroizing::new(root))
}

/// [`open_keyfile_root`] with the root expanded into its two subkeys.
pub fn open_keyfile(kf: &SyncKeyfile, passphrase: &str) -> Result<SyncKeys, String> {
    let root = open_keyfile_root(kf, passphrase)?;
    expand_root(&root)
}

fn decode_nonce(encoded: &str) -> Result<[u8; NONCE_LEN], String> {
    B64.decode(encoded)
        .ok()
        .and_then(|b| <[u8; NONCE_LEN]>::try_from(b.as_slice()).ok())
        .ok_or_else(|| OPAQUE_FAILURE.to_string())
}

/// Seal one record's bytes under the data key.
///
/// A FRESH RANDOM NONCE PER CALL, drawn inside
/// `src-tauri/src/modules/aesgcm.rs`. `OneNonce` guarantees one use per key
/// INSTANCE and cannot guarantee this: the same data key seals every record in
/// the inventory, so a constant nonce here would be reuse across the whole
/// inventory rather than within one file.
pub fn seal_record(keys: &SyncKeys, plaintext: &[u8]) -> Result<SealedRecord, String> {
    let (nonce, buf) = seal_with_key(&keys.data, &[], plaintext)?;
    Ok(SealedRecord {
        nonce: B64.encode(nonce),
        ciphertext: B64.encode(&buf),
    })
}

/// Open what `seal_record` produced. AAD is empty: the record's identity is
/// the object name it is stored under.
pub fn open_record(keys: &SyncKeys, sealed: &SealedRecord) -> Result<Zeroizing<Vec<u8>>, String> {
    let nonce = decode_nonce(&sealed.nonce)?;
    let buf = B64.decode(&sealed.ciphertext).map_err(|_| OPAQUE_FAILURE)?;
    open_with_key(&keys.data, &[], &nonce, buf, "sync")
}

/// The remote's name for one record: `hex(HMAC-SHA256(name_key, kind:id))`.
///
/// Deterministic, so two devices name the same record the same way without
/// talking; keyed, so the name reveals neither the kind nor the id to whoever
/// can list the storage.
///
/// Hex rather than base64 because the result is a path segment, and base64's
/// alphabet includes `/`. The encoding is the `hex` crate's, which
/// `sigv4::hex` in `src-tauri/src/modules/sync/providers/sigv4.rs` also calls.
///
/// The `:` is a real separator only because NO `kind` CONTAINS ONE. The two in
/// use are `entry` and `group`, and both live in
/// `src-tauri/src/modules/sync/model.rs`. Without that,
/// `("a:b", "c")` and `("a", "b:c")` would name one object.
pub fn object_name(keys: &SyncKeys, kind: &str, id: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, &keys.name[..]);
    let tag = hmac::sign(&key, format!("{kind}:{id}").as_bytes());
    hex::encode(tag.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::vault::kdf::{MAX_ITERATIONS, MAX_MEMORY_KIB, MAX_PARALLELISM};

    /// One keyfile plus its keys, for tests that do not care about the
    /// passphrase.
    fn fresh() -> (SyncKeyfile, SyncKeys) {
        new_keyfile("correct horse").expect("new keyfile")
    }

    #[test]
    fn the_same_plaintext_seals_differently_every_time() {
        // A constant nonce under one data key would be reuse across the WHOLE
        // inventory, not within one file, which is why this is asserted here
        // and not left to `OneNonce`.
        let (_, keys) = fresh();
        let a = seal_record(&keys, b"the same record").unwrap();
        let b = seal_record(&keys, b"the same record").unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
        assert_eq!(
            open_record(&keys, &a).unwrap().as_slice(),
            b"the same record"
        );
        assert_eq!(
            open_record(&keys, &b).unwrap().as_slice(),
            b"the same record"
        );
    }

    #[test]
    fn the_two_subkeys_differ() {
        // An HKDF label typo collapses them, and every other test in this file
        // still passes. Reach, stated honestly: this also passes for any
        // construction where the two differ, including one that never calls
        // HKDF at all.
        let (_, keys) = fresh();
        assert_ne!(&*keys.data, &*keys.name);
    }

    #[test]
    fn each_path_uses_its_own_subkey() {
        // Built by hand with known distinct halves, because swapping the two
        // inside this module passes every other test here unchanged.
        let straight = SyncKeys {
            data: Zeroizing::new([1u8; 32]),
            name: Zeroizing::new([2u8; 32]),
        };
        let swapped = SyncKeys {
            data: Zeroizing::new([2u8; 32]),
            name: Zeroizing::new([1u8; 32]),
        };
        // Only the DATA key is allowed to open a record sealed under it.
        let sealed = seal_record(&straight, b"payload").unwrap();
        assert!(open_record(&swapped, &sealed).is_err());
        // Only the NAME key is allowed to name an object.
        assert_ne!(
            object_name(&straight, "entry", "e-1"),
            object_name(&swapped, "entry", "e-1")
        );
    }

    #[test]
    fn a_keyfile_reopened_with_its_passphrase_yields_the_same_keys() {
        // The second-device path: nothing travels but the keyfile and the
        // passphrase, and both halves have to come back identical or a record
        // sealed on one device is unreadable on the other.
        let (kf, first) = fresh();
        let second = open_keyfile(&kf, "correct horse").unwrap();
        let sealed = seal_record(&first, b"shared record").unwrap();
        assert_eq!(
            open_record(&second, &sealed).unwrap().as_slice(),
            b"shared record"
        );
        assert_eq!(
            object_name(&first, "group", "g-1"),
            object_name(&second, "group", "g-1")
        );
    }

    #[test]
    fn a_record_does_not_open_under_a_different_keyfile() {
        let (_, mine) = fresh();
        let (_, theirs) = fresh();
        let sealed = seal_record(&mine, b"mine").unwrap();
        assert!(open_record(&theirs, &sealed).is_err());
    }

    #[test]
    fn a_wrong_passphrase_and_a_corrupt_keyfile_are_indistinguishable() {
        // `.err()` rather than `.unwrap_err()`, which would need `SyncKeys` to
        // be `Debug`, see the struct for why it deliberately is not.
        let (kf, _) = fresh();
        let wrong = open_keyfile(&kf, "not the passphrase")
            .err()
            .expect("a wrong passphrase must fail");

        let mut corrupt = kf.clone();
        let mut raw = B64.decode(&corrupt.wrapped).unwrap();
        raw[0] ^= 0x01;
        corrupt.wrapped = B64.encode(&raw);
        let tampered = open_keyfile(&corrupt, "correct horse")
            .err()
            .expect("a corrupt keyfile must fail");

        assert_eq!(wrong, tampered, "the two failures are distinguishable");
        // And it says `sync`, not `vault`: the prefix is what separates this
        // path's message from the vault file's.
        assert!(wrong.starts_with("sync:"), "unexpected error: {wrong}");
        assert!(!wrong.contains("vault"), "unexpected error: {wrong}");
    }

    #[test]
    fn an_unknown_format_is_refused_by_name() {
        let (mut kf, _) = fresh();
        kf.format = "some-other-tool".into();
        let err = open_keyfile(&kf, "correct horse")
            .err()
            .expect("an unknown format must fail");
        assert_eq!(err, NOT_A_KEYFILE);
        // The message names the product, so the user knows what it expected.
        assert!(err.contains("Subclave"), "unexpected error: {err}");
    }

    #[test]
    fn a_newer_keyfile_is_refused_before_the_kdf() {
        let (mut newer, _) = fresh();
        newer.v = KEYFILE_VERSION + 1;
        // A WRONG passphrase is the proof that the KDF never ran: had it run,
        // this would be OPAQUE_FAILURE.
        assert_eq!(
            open_keyfile(&newer, "not the passphrase").err(),
            Some(NEWER_KEYFILE.to_string())
        );
        assert_eq!(
            open_keyfile(&newer, "correct horse").err(),
            Some(NEWER_KEYFILE.to_string())
        );
    }

    #[test]
    fn kdf_params_over_the_caps_are_refused_before_the_kdf() {
        // Argon2id parameters arrive from storage and nothing has authenticated
        // them by the time the KDF runs, so an unbounded request is a hang per
        // pull for whoever can write to the remote.
        let (kf, _) = fresh();

        let mut too_much_memory = kf.clone();
        too_much_memory.kdf.memory_kib = MAX_MEMORY_KIB + 1;
        assert_eq!(
            open_keyfile(&too_much_memory, "not the passphrase").err(),
            Some(CAPS_FAILURE.to_string())
        );

        let mut too_many_iterations = kf.clone();
        too_many_iterations.kdf.iterations = MAX_ITERATIONS + 1;
        assert_eq!(
            open_keyfile(&too_many_iterations, "not the passphrase").err(),
            Some(CAPS_FAILURE.to_string())
        );

        let mut too_much_parallelism = kf.clone();
        too_much_parallelism.kdf.parallelism = MAX_PARALLELISM + 1;
        assert_eq!(
            open_keyfile(&too_much_parallelism, "not the passphrase").err(),
            Some(CAPS_FAILURE.to_string())
        );

        // The same wrong passphrase against the real parameters reaches the
        // KDF and comes back opaque, so the caps path is not simply refusing
        // everything.
        assert_eq!(
            open_keyfile(&kf, "not the passphrase").err(),
            Some(OPAQUE_FAILURE.to_string())
        );
        assert!(open_keyfile(&kf, "correct horse").is_ok());
    }

    #[test]
    fn a_sealed_record_leaks_none_of_its_plaintext() {
        // Needle discipline: every needle is a run of five or more characters
        // drawn ENTIRELY from the base64 alphabet. A needle carrying a `.`
        // could only fire if the encoding itself broke, which reads as
        // coverage without being any, and a short run turns up in base64
        // output by chance.
        let needles = ["vpsalpha", "svcdeploy", "hunter2", "54321"];
        let plain = r#"{"id":"e-1","title":"vpsalpha","url":"vpsalpha.example.com","port":54321,"user":"svcdeploy","password":"hunter2"}"#;
        // Negative assertions pass for free when a needle is simply absent, so
        // a dropped field would read as coverage instead of a hole.
        for needle in needles {
            assert!(
                plain.contains(needle),
                "{needle} is missing from the fixture"
            );
        }

        let (_, keys) = fresh();
        let sealed = seal_record(&keys, plain.as_bytes()).unwrap();
        // Collected rather than asserted one at a time, so a leak names every
        // needle it exposed instead of stopping at the first.
        let leaked: Vec<&str> = needles
            .into_iter()
            .filter(|n| sealed.ciphertext.contains(n) || sealed.nonce.contains(n))
            .collect();
        assert!(
            leaked.is_empty(),
            "readable in the sealed record: {leaked:?}"
        );
        assert_eq!(
            open_record(&keys, &sealed).unwrap().as_slice(),
            plain.as_bytes()
        );
    }

    #[test]
    fn an_object_name_hides_what_it_was_built_from() {
        // NEEDLE DISCIPLINE, and here it cuts the other way from the sealed
        // record's. An object name is HEX, so a needle carrying any character
        // outside `[0-9a-f]` cannot appear in one no matter what the function
        // does, and asserting it reads as coverage without being any. So the
        // fixture's kind and id are drawn entirely from the hex alphabet and
        // are long enough not to turn up by chance.
        let (_, keys) = fresh();
        let (kind, id) = ("decaf", "deadbeefcafe");
        let name = object_name(&keys, kind, id);
        assert!(!name.contains(kind), "the kind leaked into {name}");
        assert!(!name.contains(id), "the id leaked into {name}");

        // Deterministic, or two devices would file one record twice.
        assert_eq!(name, object_name(&keys, kind, id));
        // Both halves are bound in, or two records would share one name. The
        // separator is what makes that true for every pair, since no `kind` in
        // use carries a colon.
        assert_ne!(name, object_name(&keys, kind, "deadbeefcaf"));
        assert_ne!(name, object_name(&keys, "decafe", id));
        // And keyed, or the storage's listing would be the same for everybody.
        let (_, other) = fresh();
        assert_ne!(name, object_name(&other, kind, id));
    }

    #[test]
    fn an_empty_passphrase_is_refused() {
        assert!(new_keyfile("").is_err());
    }
}
