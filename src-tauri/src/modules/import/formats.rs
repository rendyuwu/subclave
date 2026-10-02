//! CSV parsing for the four supported exports (KeePassXC, Bitwarden, Chrome,
//! Firefox): header detection and the row-to-entry mapping. No state.

use serde::{Deserialize, Serialize};
use url::Url;

use crate::modules::vault::model::{host_of, CustomField, Entry, EntryUrl, MatchMode, ROOT_ID};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CsvFormat {
    Auto,
    Keepassxc,
    Bitwarden,
    Chrome,
    Firefox,
}

/// `Auto` takes the first format whose required columns are all present.
/// Firefox before Chrome: a Firefox header with a `name` column also holds
/// every Chrome column, while `httprealm` is Firefox's alone.
const DETECT_ORDER: [CsvFormat; 4] = [
    CsvFormat::Bitwarden,
    CsvFormat::Keepassxc,
    CsvFormat::Firefox,
    CsvFormat::Chrome,
];

fn required(format: CsvFormat) -> &'static [&'static str] {
    match format {
        CsvFormat::Auto => &[],
        CsvFormat::Keepassxc => &["title", "username", "password", "url"],
        CsvFormat::Bitwarden => &["name", "login_uri", "login_username", "login_password"],
        CsvFormat::Chrome => &["name", "url", "username", "password"],
        CsvFormat::Firefox => &["url", "username", "password", "httprealm"],
    }
}

/// One data record mapped to an entry. `recycled` marks a KeePassXC row from
/// the Recycle Bin, which the preview flags.
pub(crate) struct ParsedRow {
    pub(crate) entry: Entry,
    pub(crate) recycled: bool,
}

/// Parse `bytes` as `format` (or detect it), returning the resolved format
/// (never `Auto`) and one row per data record, in file order.
pub(crate) fn parse_csv(
    bytes: &[u8],
    format: CsvFormat,
) -> Result<(CsvFormat, Vec<ParsedRow>), String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "import: the file is not UTF-8 text".to_string())?;
    let read_err = |e: csv::Error| format!("import: could not read the CSV file: {e}");
    // `csv` drops a leading UTF-8 BOM itself (`a_bom_prefixed_file_parses`).
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());
    let header: Vec<String> = reader
        .headers()
        .map_err(read_err)?
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .collect();
    let has = |col: &&str| header.iter().any(|h| h == col);
    let format = match format {
        CsvFormat::Auto => DETECT_ORDER
            .into_iter()
            .find(|f| required(*f).iter().all(has))
            .ok_or_else(|| {
                "import: the columns match no supported format; pick the format".to_string()
            })?,
        explicit => {
            if let Some(col) = required(explicit).iter().find(|c| !has(c)) {
                return Err(format!(
                    "import: the file has no \"{col}\" column for that format"
                ));
            }
            explicit
        }
    };
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(read_err)?;
        // First occurrence of a name wins; a missing column or a short
        // record reads as empty.
        let get = |name: &str| {
            header
                .iter()
                .position(|h| h == name)
                .and_then(|i| record.get(i))
                .unwrap_or("")
        };
        rows.push(map_row(format, get));
    }
    Ok((format, rows))
}

fn map_row<'r>(format: CsvFormat, get: impl Fn(&str) -> &'r str) -> ParsedRow {
    let (title, username, password, uris, notes, totp, favorite, fields) = match format {
        CsvFormat::Keepassxc => (
            get("title"),
            get("username"),
            get("password"),
            vec![get("url")],
            get("notes"),
            get("totp"),
            false,
            "",
        ),
        CsvFormat::Bitwarden => (
            get("name"),
            get("login_username"),
            get("login_password"),
            get("login_uri").split(',').collect(),
            get("notes"),
            get("login_totp"),
            get("favorite").trim() == "1",
            get("fields"),
        ),
        CsvFormat::Chrome => (
            get("name"),
            get("username"),
            get("password"),
            vec![get("url")],
            get("note"),
            "",
            false,
            "",
        ),
        CsvFormat::Firefox | CsvFormat::Auto => (
            "",
            get("username"),
            get("password"),
            vec![get("url")],
            "",
            "",
            false,
            "",
        ),
    };
    let recycled = format == CsvFormat::Keepassxc && {
        let group = get("group").trim();
        group == "Recycle Bin" || group.ends_with("/Recycle Bin")
    };

    let urls: Vec<EntryUrl> = uris
        .into_iter()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(|u| EntryUrl {
            url: normalize_url(u),
            match_mode: MatchMode::Domain,
        })
        .collect();
    let mut title = title.trim().to_string();
    if title.is_empty() {
        title = urls
            .first()
            .and_then(|u| host_of(&u.url))
            .unwrap_or_default();
    }

    let mut custom_fields = Vec::new();
    for line in fields.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() {
            continue;
        }
        let (name, value) = match line.split_once(": ") {
            Some((name, value)) if !name.trim().is_empty() => (name.trim(), value),
            Some((_, value)) => ("Field", value),
            None => ("Field", line),
        };
        push_field(&mut custom_fields, name, value);
    }
    let totp = totp_of(totp.trim(), &mut custom_fields);

    ParsedRow {
        entry: Entry {
            id: uuid::Uuid::new_v4().to_string(),
            group_id: ROOT_ID.to_string(),
            title,
            username: username.trim().to_string(),
            password: password.to_string(),
            urls,
            notes: notes.to_string(),
            totp,
            custom_fields,
            tags: vec![],
            icon: None,
            color: None,
            favorite,
            expires_at: None,
            trashed_from: None,
            created_at: 0,
            updated_at: 0,
            history: vec![],
            last_used_at: None,
        },
        recycled,
    }
}

/// Keep a URL that already parses with a host. Otherwise try it with
/// `https://` in front (so `example.org:443` is not read as scheme
/// `example.org`), and fall back to the text as written.
fn normalize_url(s: &str) -> String {
    let has_host = |u: &str| Url::parse(u).is_ok_and(|p| p.host_str().is_some());
    if has_host(s) {
        return s.to_string();
    }
    let prefixed = format!("https://{s}");
    if has_host(&prefixed) {
        prefixed
    } else {
        s.to_string()
    }
}

/// The stored otpauth URI for a TOTP cell: an `otpauth://` URI as given, a
/// bare base32 secret wrapped the way the editor's `totpUriFromInput` does.
/// A value that does not parse is kept as a hidden `TOTP` custom field.
fn totp_of(raw: &str, fields: &mut Vec<CustomField>) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let is_base32 = raw.chars().all(|c| {
        c.is_ascii_alphabetic() || ('2'..='7').contains(&c) || c == '=' || c.is_whitespace()
    });
    let candidate = if !raw.starts_with("otpauth://") && is_base32 {
        let secret: String = raw
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '=')
            .map(|c| c.to_ascii_uppercase())
            .collect();
        format!("otpauth://totp/Subclave?secret={secret}")
    } else {
        raw.to_string()
    };
    if crate::modules::totp::parse(&candidate).is_ok() {
        return Some(candidate);
    }
    push_field(fields, "TOTP", raw);
    None
}

/// Add a hidden field (CSV drops Bitwarden's hidden flag), suffixing the name
/// with ` 2`, ` 3`, ... while it clashes case-insensitively.
fn push_field(fields: &mut Vec<CustomField>, name: &str, value: &str) {
    let mut candidate = name.to_string();
    let mut n = 2;
    while fields
        .iter()
        .any(|f| f.name.eq_ignore_ascii_case(&candidate))
    {
        candidate = format!("{name} {n}");
        n += 1;
    }
    fields.push(CustomField {
        name: candidate,
        value: value.to_string(),
        hidden: true,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEEPASSXC: &str = "\"Group\",\"Title\",\"Username\",\"Password\",\"URL\",\"Notes\",\"TOTP\",\"Icon\",\"Last Modified\",\"Created\"\n\
\"Root/Email\",\"Mail\",\"alice\",\"pw-a\",\"https://mail.example.com\",\"note a\",\"otpauth://totp/x?secret=JBSWY3DPEHPK3PXP\",\"0\",\"2024-01-01T00:00:00Z\",\"2024-01-01T00:00:00Z\"\n\
\"Root/Recycle Bin\",\"Old\",\"bob\",\"pw-b\",\"https://old.example.com\",\"\",\"\",\"0\",\"\",\"\"\n\
\"Recycle Bin\",\"Older\",\"carol\",\"pw-c\",\"\",\"\",\"\",\"0\",\"\",\"\"\n";

    const BITWARDEN: &str = "folder,favorite,type,name,notes,fields,reprompt,login_uri,login_username,login_password,login_totp\n\
Work,1,login,Bank,bank note,\"PIN: 1234\nPIN: 5678\nloose line\",0,\"https://bank.example.com, bank.example.org\",dave,pw-d,jbsw y3dp ehpk 3pxp\n\
,,login,Bad totp,,TOTP: kept,0,https://x.example.com,erin,pw-e,not-a-secret!\n";

    const CHROME: &str = "name,url,username,password,note\n\
Twitter,twitter.com,frank,pw-f,chrome note\n";

    const FIREFOX: &str = "\"url\",\"username\",\"password\",\"httpRealm\",\"formActionOrigin\",\"guid\",\"timeCreated\",\"timeLastUsed\",\"timePasswordChanged\"\n\
\"example.org:443\",\"gina\",\"pw-g\",,\"https://example.org\",\"{abc}\",\"1\",\"1\",\"1\"\n";

    /// A Firefox-shaped header that also carries every Chrome column.
    const AMBIGUOUS: &str = "name,url,username,password,httprealm\n\
Site,https://amb.example.com,hank,pw-h,\n";

    fn parse(text: &str, format: CsvFormat) -> (CsvFormat, Vec<ParsedRow>) {
        parse_csv(text.as_bytes(), format).unwrap()
    }

    fn refusal(bytes: &[u8], format: CsvFormat) -> String {
        parse_csv(bytes, format).err().unwrap()
    }

    fn hidden(name: &str, value: &str) -> CustomField {
        CustomField {
            name: name.into(),
            value: value.into(),
            hidden: true,
        }
    }

    #[test]
    fn each_real_header_is_detected() {
        assert_eq!(parse(KEEPASSXC, CsvFormat::Auto).0, CsvFormat::Keepassxc);
        assert_eq!(parse(BITWARDEN, CsvFormat::Auto).0, CsvFormat::Bitwarden);
        assert_eq!(parse(CHROME, CsvFormat::Auto).0, CsvFormat::Chrome);
        assert_eq!(parse(FIREFOX, CsvFormat::Auto).0, CsvFormat::Firefox);
    }

    #[test]
    fn an_ambiguous_header_detects_as_firefox_and_the_override_wins() {
        let (format, rows) = parse(AMBIGUOUS, CsvFormat::Auto);
        assert_eq!(format, CsvFormat::Firefox);
        assert_eq!(rows[0].entry.title, "amb.example.com");
        let (format, rows) = parse(AMBIGUOUS, CsvFormat::Chrome);
        assert_eq!(format, CsvFormat::Chrome);
        assert_eq!(rows[0].entry.title, "Site");
    }

    #[test]
    fn an_override_missing_a_required_column_names_it() {
        assert_eq!(
            refusal(CHROME.as_bytes(), CsvFormat::Firefox),
            "import: the file has no \"httprealm\" column for that format"
        );
    }

    #[test]
    fn keepassxc_rows_map_and_flag_the_recycle_bin() {
        let (_, rows) = parse(KEEPASSXC, CsvFormat::Auto);
        let e = &rows[0].entry;
        assert_eq!(
            (e.title.as_str(), e.username.as_str(), e.password.as_str()),
            ("Mail", "alice", "pw-a")
        );
        assert_eq!(e.urls[0].url, "https://mail.example.com");
        assert_eq!(e.notes, "note a");
        assert_eq!(
            e.totp.as_deref(),
            Some("otpauth://totp/x?secret=JBSWY3DPEHPK3PXP")
        );
        assert_eq!(
            rows.iter().map(|r| r.recycled).collect::<Vec<_>>(),
            [false, true, true]
        );
    }

    #[test]
    fn bitwarden_rows_map_fields_uris_and_totp() {
        let (_, rows) = parse(BITWARDEN, CsvFormat::Auto);
        let e = &rows[0].entry;
        assert!(e.favorite);
        assert_eq!(
            e.urls.iter().map(|u| u.url.as_str()).collect::<Vec<_>>(),
            ["https://bank.example.com", "https://bank.example.org"]
        );
        assert_eq!(
            e.custom_fields,
            [
                hidden("PIN", "1234"),
                hidden("PIN 2", "5678"),
                hidden("Field", "loose line"),
            ]
        );
        assert_eq!(
            e.totp.as_deref(),
            Some("otpauth://totp/Subclave?secret=JBSWY3DPEHPK3PXP")
        );
        assert!(!rows[1].entry.favorite);
        assert_eq!(rows[1].entry.totp, None);
        // The unreadable TOTP joins the name-clash suffixing like any field.
        assert_eq!(
            rows[1].entry.custom_fields,
            [hidden("TOTP", "kept"), hidden("TOTP 2", "not-a-secret!")]
        );
        assert!(!rows[0].recycled);
    }

    #[test]
    fn firefox_titles_from_the_host_and_schemeless_urls_gain_https() {
        let (_, rows) = parse(FIREFOX, CsvFormat::Auto);
        assert_eq!(rows[0].entry.urls[0].url, "https://example.org:443");
        assert_eq!(rows[0].entry.title, "example.org");
        assert_eq!(rows[0].entry.notes, "");
        let (_, rows) = parse(CHROME, CsvFormat::Auto);
        assert_eq!(rows[0].entry.urls[0].url, "https://twitter.com");
        assert_eq!(rows[0].entry.title, "Twitter");
        assert_eq!(rows[0].entry.notes, "chrome note");
    }

    #[test]
    fn a_bom_prefixed_file_parses() {
        let (format, rows) = parse(&format!("\u{feff}{CHROME}"), CsvFormat::Auto);
        assert_eq!(format, CsvFormat::Chrome);
        assert_eq!(rows[0].entry.username, "frank");
    }

    #[test]
    fn bad_bytes_and_unknown_headers_are_refused() {
        assert_eq!(
            refusal(b"name,url\n\xff\xfe\n", CsvFormat::Auto),
            "import: the file is not UTF-8 text"
        );
        assert_eq!(
            refusal(b"a,b,c\n1,2,3\n", CsvFormat::Auto),
            "import: the columns match no supported format; pick the format"
        );
    }
}
