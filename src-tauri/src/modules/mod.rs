pub mod aesgcm;
pub mod backup;
pub mod browser;
pub mod clipboard;
pub mod events;
pub mod fs;
pub mod generator;
pub mod import;
pub mod lockext;
pub mod prefs;
pub mod strength;
pub mod sync;
pub mod totp;
pub mod tray;
pub mod vault;

/// Shared test-only fixtures.
#[cfg(test)]
pub(crate) mod test_rng {
    /// Deterministic xorshift64* fill, seeded per test. Used by the generator
    /// and the history-merge property tests.
    pub(crate) fn xorshift(seed: u64) -> impl FnMut(&mut [u8]) {
        let mut state = seed | 1;
        move |buf: &mut [u8]| {
            for slot in buf.iter_mut() {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                *slot = (state.wrapping_mul(0x2545F4914F6CDD1D) >> 56) as u8;
            }
        }
    }
}
