//! Password strength via zxcvbn: score 0..=4 plus the matcher's warning.

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Strength {
    /// 0..=4 from zxcvbn's estimate.
    pub score: u8,
    pub warning: Option<String>,
}

/// The mapping from zxcvbn's estimate to the webview shape.
fn strength_of(password: &str) -> Strength {
    let estimate = zxcvbn::zxcvbn(password, &[]);
    Strength {
        score: u8::from(estimate.score()),
        warning: estimate
            .feedback()
            .and_then(|f| f.warning())
            .map(|w| w.to_string()),
    }
}

/// An empty password scores 0 rather than erroring: the meter renders before
/// the user has typed anything.
#[tauri::command]
pub async fn gen_strength(password: String) -> Result<Strength, String> {
    tauri::async_runtime::spawn_blocking(move || Ok(strength_of(&password)))
        .await
        .map_err(|e| format!("strength: task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_stays_inside_the_reported_range() {
        for password in [
            "password",
            "Tr0ub4dor&3",
            "correct horse battery staple zebra 42!",
        ] {
            let strength = strength_of(password);
            assert!(
                strength.score <= 4,
                "{password:?} scored {}",
                strength.score
            );
        }
    }

    #[test]
    fn empty_password_scores_zero_without_a_warning() {
        // The meter renders before the user has typed anything.
        assert_eq!(strength_of("").score, 0);
        assert_eq!(strength_of("").warning, None);
    }
}
