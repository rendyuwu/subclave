//! The one date format a listing spells its modification time in.

use super::super::sigv4;

/// Unix MILLISECONDS from the date format a listing's modification time is
/// spelled in, or `None`.
///
/// A DIFFERENT FORMAT FROM THE OTHER BACKEND'S, and reusing that one's parser
/// here would return `None` on every row rather than a wrong instant - which
/// sounds harmless and is not. A row with no stamp is never a candidate for the
/// expiry pass, so expired tombstones would never be pruned from a remote and
/// would accumulate without bound, and an expired remote tombstone would be
/// applied here where the other backend reports it for the user to resolve.
/// Two devices sharing one prefix would then disagree about a delete.
///
/// ONLY THE FIXED-WIDTH FORM IS READ, which is a deliberate narrowing of the
/// general HTTP rule that a recipient accepts three date formats. This value is
/// not an HTTP header field: the protocol defines it as the fixed-width form
/// and names no other, so a compliant generator has one spelling. Against that,
/// the two obsolete parsers are some twenty-five lines whose failure mode is
/// `None` - the conservative direction, since a row with no stamp is never
/// mistaken for expired. If a real server is ever found emitting one, this is
/// additive.
///
/// Shares the era arithmetic in
/// `src-tauri/src/modules/sync/providers/sigv4.rs` rather than carrying a
/// second copy of it.
pub fn parse_http_date(s: &str) -> Option<u64> {
    let s = s.trim();
    let b = s.as_bytes();
    // `Sun, 06 Nov 1994 08:49:37 GMT` and nothing else: every separator is at a
    // fixed offset and the whole thing is exactly this long.
    if b.len() != 29
        || b[3] != b','
        || b[4] != b' '
        || b[7] != b' '
        || b[11] != b' '
        || b[16] != b' '
        || b[19] != b':'
        || b[22] != b':'
        || b[25] != b' '
        || &s[26..] != "GMT"
    {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<u32> {
        let part = s.get(from..to)?;
        if !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    // The day name is not checked. It is redundant with the date beside it, and
    // a recipient is told to ignore it rather than to validate it.
    let day = num(5, 7)?;
    let month = match &s[8..11] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year = num(12, 16)? as i64;
    let hour = num(17, 19)?;
    let minute = num(20, 22)?;
    let second = num(23, 25)?;
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let secs = sigv4::days_from_civil(year, month, day) * 86_400
        + hour as i64 * 3600
        + minute as i64 * 60
        + second as i64;
    if secs < 0 {
        return None;
    }
    Some(secs as u64 * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- the date ---------------------------------------------------------

    #[test]
    fn the_listing_date_format_parses_to_milliseconds() {
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777_000)
        );
        assert_eq!(
            parse_http_date("Mon, 12 Jan 1998 09:25:56 GMT"),
            Some(884_597_156_000)
        );
    }

    #[test]
    fn the_obsolete_date_formats_and_a_nonsense_one_are_refused() {
        for bad in [
            // The two obsolete spellings, deliberately not read.
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
            // A zone that is not the one the format fixes.
            "Sun, 06 Nov 1994 08:49:37 UTC",
            "Sun, 06 Nov 1994 08:49:37 +0000",
            "Sun, 06 Xyz 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:6a GMT",
            "",
        ] {
            assert_eq!(parse_http_date(bad), None, "{bad} was read as a date");
        }
    }
}
