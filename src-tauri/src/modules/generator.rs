//! Password generator: uniform draw over the union of enabled sets by
//! rejection sampling, redrawn until every enabled set appears.
//!
//! The RNG is injectable: a plain `FnMut` filling a byte buffer, so the tests
//! drive it with a seeded deterministic generator and no trait is needed.
//! Production passes the OS CSPRNG.

use serde::{Deserialize, Serialize};

/// Characters removed by `exclude_ambiguous`: I, l, 1, O, 0. Symbols are
/// never filtered.
const AMBIGUOUS: [char; 5] = ['I', 'l', '1', 'O', '0'];

/// Not a protocol constant, just a product choice; this is the one const to change.
pub const SYMBOLS: &str = "!@#$%^&*()-_=+[]{};:,.?/";

/// The webview's password-generator settings, deserialized camelCase from the
/// `gen_password` command argument.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorOptions {
    pub length: u32,
    pub lower: bool,
    pub upper: bool,
    pub digits: bool,
    pub symbols: bool,
    pub exclude_ambiguous: bool,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        Self {
            length: 20,
            lower: true,
            upper: true,
            digits: true,
            symbols: true,
            exclude_ambiguous: false,
        }
    }
}

fn charset(options: &GeneratorOptions) -> Result<Vec<char>, String> {
    let mut set: Vec<char> = Vec::new();
    if options.lower {
        set.extend('a'..='z');
    }
    if options.upper {
        set.extend('A'..='Z');
    }
    if options.digits {
        set.extend('0'..='9');
    }
    if options.symbols {
        set.extend(SYMBOLS.chars());
    }
    if options.exclude_ambiguous {
        set.retain(|c| !AMBIGUOUS.contains(c));
    }
    if set.is_empty() {
        return Err("generator: pick at least one character set".to_string());
    }
    Ok(set)
}

/// Validate and generate. `length` is 8..=128. Each character is drawn
/// uniformly from the union of enabled sets by rejection sampling (`limit =
/// 256 - 256 % set_len`, bytes past it redrawn); the whole password is
/// redrawn until every enabled set appears, bounded so the loop cannot spin
/// forever on a hostile option combination.
pub fn generate(
    options: &GeneratorOptions,
    rng: &mut dyn FnMut(&mut [u8]),
) -> Result<String, String> {
    let length = options.length;
    if !(8..=128).contains(&length) {
        return Err("generator: length must be 8 to 128".to_string());
    }
    let set = charset(options)?;
    // One group per enabled set, restricted to what survived the ambiguous
    // filter. A group left empty by that filter cannot be satisfied and must
    // not gate the redraw loop.
    let required: Vec<Vec<char>> = {
        let mut groups: Vec<Vec<char>> = Vec::new();
        if options.lower {
            let g: Vec<char> = set
                .iter()
                .copied()
                .filter(|c| c.is_ascii_lowercase())
                .collect();
            if !g.is_empty() {
                groups.push(g);
            }
        }
        if options.upper {
            let g: Vec<char> = set
                .iter()
                .copied()
                .filter(|c| c.is_ascii_uppercase())
                .collect();
            if !g.is_empty() {
                groups.push(g);
            }
        }
        if options.digits {
            let g: Vec<char> = set.iter().copied().filter(|c| c.is_ascii_digit()).collect();
            if !g.is_empty() {
                groups.push(g);
            }
        }
        if options.symbols {
            let g: Vec<char> = set
                .iter()
                .copied()
                .filter(|c| SYMBOLS.contains(*c))
                .collect();
            if !g.is_empty() {
                groups.push(g);
            }
        }
        groups
    };
    let set_len = set.len() as u32;
    let limit = 256 - 256 % set_len;
    let mut bytes = vec![0u8; length as usize];
    for _attempt in 0..10_000 {
        for slot in bytes.iter_mut() {
            loop {
                let mut one = [0u8; 1];
                rng(&mut one);
                if u32::from(one[0]) < limit {
                    *slot = one[0];
                    break;
                }
            }
        }
        let password: String = bytes
            .iter()
            .map(|b| set[*b as usize % set_len as usize])
            .collect();
        if required
            .iter()
            .all(|group| password.chars().any(|c| group.contains(&c)))
        {
            return Ok(password);
        }
    }
    Err("generator: could not satisfy all character sets".to_string())
}

#[tauri::command]
pub async fn gen_password(options: GeneratorOptions) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use ring::rand::SecureRandom as _;
        let random = ring::rand::SystemRandom::new();
        generate(&options, &mut |buf| {
            // A dead system RNG kills the process's guarantees wholesale, so
            // the panic is the honest failure; it cannot fire in practice.
            random.fill(buf).expect("generator: rng failed")
        })
    })
    .await
    .map_err(|e| format!("generator: task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::modules::test_rng::xorshift;

    fn defaults() -> GeneratorOptions {
        GeneratorOptions::default()
    }

    /// Chi-square uniformity at p >= 0.001 over lowercase-only draws,
    /// length 128, 10,000 passwords. df = 25, critical value 52.62.
    #[test]
    fn lowercase_draws_pass_chi_square() {
        let options = GeneratorOptions {
            length: 128,
            lower: true,
            upper: false,
            digits: false,
            symbols: false,
            exclude_ambiguous: false,
        };
        let mut rng = xorshift(0xA11CE);
        let mut counts = [0u64; 26];
        for _ in 0..10_000 {
            let pw = generate(&options, &mut rng).unwrap();
            for c in pw.chars() {
                counts[(c as u8 - b'a') as usize] += 1;
            }
        }
        let n: u64 = counts.iter().sum();
        let expected = n as f64 / 26.0;
        let stat: f64 = counts
            .iter()
            .map(|&o| (o as f64 - expected).powi(2) / expected)
            .sum();
        assert!(
            stat < 52.62,
            "chi-square {stat} >= 52.62: counts uneven {counts:?}"
        );
    }

    #[test]
    fn every_default_password_contains_all_four_sets() {
        let mut rng = xorshift(0xB0B);
        for _ in 0..10_000 {
            let pw = generate(&defaults(), &mut rng).unwrap();
            assert!(pw.chars().any(|c| c.is_ascii_lowercase()));
            assert!(pw.chars().any(|c| c.is_ascii_uppercase()));
            assert!(pw.chars().any(|c| c.is_ascii_digit()));
            assert!(pw.chars().any(|c| SYMBOLS.contains(c)));
            assert_eq!(pw.chars().count(), 20);
        }
    }

    #[test]
    fn exclude_ambiguous_removes_exactly_the_five() {
        let options = GeneratorOptions {
            length: 128,
            lower: true,
            upper: true,
            digits: true,
            symbols: false,
            exclude_ambiguous: true,
        };
        let mut rng = xorshift(0xC0FFEE);
        for _ in 0..1_000 {
            let pw = generate(&options, &mut rng).unwrap();
            assert!(!pw.chars().any(|c| AMBIGUOUS.contains(&c)), "{pw}");
        }
    }

    #[test]
    fn empty_union_and_bad_length_error() {
        let empty = GeneratorOptions {
            length: 20,
            lower: false,
            upper: false,
            digits: false,
            symbols: false,
            exclude_ambiguous: false,
        };
        assert_eq!(
            generate(&empty, &mut xorshift(1)).unwrap_err(),
            "generator: pick at least one character set"
        );
        assert_eq!(
            generate(
                &GeneratorOptions {
                    length: 7,
                    ..Default::default()
                },
                &mut xorshift(1)
            )
            .unwrap_err(),
            "generator: length must be 8 to 128"
        );
        assert_eq!(
            generate(
                &GeneratorOptions {
                    length: 129,
                    ..Default::default()
                },
                &mut xorshift(1)
            )
            .unwrap_err(),
            "generator: length must be 8 to 128"
        );
    }
}
