//! URL matching for the browser integration.
//!
//! Pure functions: the extension never decides a match, it sends the page URL
//! and Rust applies the rules here. The scheme family, the port and the
//! entry's own `match` mode decide a match; the table tests below are the
//! specification.

use url::Url;

use crate::modules::vault::model::MatchMode;

/// The page URL as the browser reported it, parsed. `None` is not parseable.
pub fn parse_page(raw: &str) -> Option<Url> {
    Url::parse(raw).ok()
}

/// Entry URLs are stored as typed; a bare `github.com/path` is read as https.
pub fn parse_entry(raw: &str) -> Option<Url> {
    match Url::parse(raw) {
        Ok(url) => Some(url),
        Err(_) => Url::parse(&format!("https://{raw}")).ok(),
    }
}

fn host_eq(page: &Url, entry: &Url) -> bool {
    match (page.host_str(), entry.host_str()) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

/// Rule 3: page `https` accepts entry `http` or `https`; page `http` accepts
/// only entry `http`. Anything else never matches.
fn scheme_ok(page: &Url, entry: &Url) -> bool {
    match page.scheme() {
        "https" => matches!(entry.scheme(), "http" | "https"),
        "http" => entry.scheme() == "http",
        _ => false,
    }
}

/// Rule 4: an entry with an explicit port must equal the page's effective
/// port; an entry without one matches the page's scheme default only.
fn port_ok(page: &Url, entry: &Url) -> bool {
    match entry.port() {
        Some(port) => page.port_or_known_default() == Some(port),
        None => page.port().is_none(),
    }
}

/// Registrable domain via `psl::domain_str`, including the private section.
/// `None` for an IP, `localhost`, a single-label host, or a host that is
/// itself a public suffix (`github.io`), and the caller then compares hosts
/// exactly.
pub fn registrable(host: &str) -> Option<String> {
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return None;
    }
    let lower = host.to_ascii_lowercase();
    psl::domain_str(&lower).map(|domain| domain.to_string())
}

fn domain_ok(page: &Url, entry: &Url) -> bool {
    match (
        page.host_str().and_then(registrable),
        entry.host_str().and_then(registrable),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => host_eq(page, entry),
    }
}

/// `/` matches everything; otherwise the entry path is a prefix of the page
/// path ending on a segment boundary (`/app` matches `/app` and `/app/x`, not
/// `/apple`).
fn path_prefix(entry_path: &str, page_path: &str) -> bool {
    if entry_path == "/" {
        return true;
    }
    if !page_path.starts_with(entry_path) {
        return false;
    }
    if page_path.len() == entry_path.len() {
        return true;
    }
    entry_path.ends_with('/') || page_path.as_bytes()[entry_path.len()] == b'/'
}

/// Full rule set: scheme, port, and the entry's own match mode. Query and
/// fragment are ignored.
pub fn matches(page: &Url, entry: &Url, mode: MatchMode) -> bool {
    if !scheme_ok(page, entry) {
        return false;
    }
    match mode {
        MatchMode::Exact => {
            page.scheme() == entry.scheme()
                && host_eq(page, entry)
                && port_ok(page, entry)
                && path_prefix(entry.path(), page.path())
        }
        MatchMode::Host => host_eq(page, entry) && port_ok(page, entry),
        MatchMode::Domain => domain_ok(page, entry) && port_ok(page, entry),
    }
}

/// The inline rule: same host, same acceptable port, same scheme family,
/// ignoring the match mode and the path. What `get-credential { via: "inline" }`
/// and the `scope: "host"` filter use.
pub fn same_host(page: &Url, entry: &Url) -> bool {
    scheme_ok(page, entry) && host_eq(page, entry) && port_ok(page, entry)
}

/// The `<scheme>://host[:port]` of a page URL, for the URL an extension fill
/// appends to an entry.
pub fn origin(page: &Url) -> String {
    match (page.host_str(), page.port()) {
        (Some(host), Some(port)) => format!("{}://{}:{}", page.scheme(), host, port),
        (Some(host), None) => format!("{}://{}", page.scheme(), host),
        (None, _) => page.as_str().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(raw: &str) -> Url {
        parse_page(raw).expect("test page url")
    }

    fn entry(raw: &str) -> Url {
        parse_entry(raw).expect("test entry url")
    }

    #[test]
    fn https_entry_is_not_offered_on_http() {
        assert!(!matches(
            &page("http://example.com"),
            &entry("https://example.com"),
            MatchMode::Domain
        ));
        assert!(matches(
            &page("https://example.com"),
            &entry("http://example.com"),
            MatchMode::Domain
        ));
    }

    #[test]
    fn subdomain_matches_domain_but_not_host() {
        let p = page("https://accounts.github.com/login");
        assert!(matches(&p, &entry("github.com"), MatchMode::Domain));
        assert!(!matches(&p, &entry("github.com"), MatchMode::Host));
        assert!(matches(&p, &entry("accounts.github.com"), MatchMode::Host));
    }

    #[test]
    fn public_suffix_hosts_never_domain_match() {
        let a = page("https://a.github.io/x");
        let b = entry("https://b.github.io/y");
        assert!(!matches(&a, &b, MatchMode::Domain));
        assert!(matches(
            &page("https://b.github.io"),
            &entry("https://b.github.io"),
            MatchMode::Domain
        ));
        // A bare public suffix never equals a subdomain.
        assert!(!matches(
            &page("https://a.github.io"),
            &entry("https://github.io"),
            MatchMode::Domain
        ));
    }

    #[test]
    fn ips_and_localhost_never_domain_match() {
        assert!(!matches(
            &page("https://127.0.0.1"),
            &entry("https://127.0.0.2"),
            MatchMode::Domain
        ));
        assert!(matches(
            &page("https://127.0.0.1"),
            &entry("https://127.0.0.1"),
            MatchMode::Domain
        ));
        assert!(!matches(
            &page("http://localhost:3000/a"),
            &entry("http://localhost:4000/a"),
            MatchMode::Domain
        ));
        assert_eq!(registrable("127.0.0.1"), None);
        assert!(!matches(
            &page("https://192.168.0.1"),
            &entry("https://10.0.0.1"),
            MatchMode::Domain
        ));
    }

    #[test]
    fn exact_mode_path_prefix_boundaries() {
        let p_app = page("https://example.com/app");
        let p_app_x = page("https://example.com/app/x");
        let p_apple = page("https://example.com/apple");
        let p_root = page("https://example.com/anything");
        assert!(matches(
            &p_app,
            &entry("https://example.com/app"),
            MatchMode::Exact
        ));
        assert!(matches(
            &p_app_x,
            &entry("https://example.com/app"),
            MatchMode::Exact
        ));
        assert!(!matches(
            &p_apple,
            &entry("https://example.com/app"),
            MatchMode::Exact
        ));
        assert!(matches(
            &p_root,
            &entry("https://example.com/"),
            MatchMode::Exact
        ));
    }

    #[test]
    fn explicit_ports_must_agree() {
        assert!(!matches(
            &page("https://example.com"),
            &entry("https://example.com:8443"),
            MatchMode::Domain
        ));
        assert!(!matches(
            &page("https://example.com:8443"),
            &entry("https://example.com"),
            MatchMode::Domain
        ));
        assert!(matches(
            &page("https://example.com:8443"),
            &entry("https://example.com:8443"),
            MatchMode::Domain
        ));
    }

    #[test]
    fn bare_entry_urls_are_read_as_https() {
        assert_eq!(entry("github.com/path").scheme(), "https");
        assert_eq!(entry("github.com/path").host_str(), Some("github.com"));
        assert!(parse_entry("not a url").is_none());
    }

    #[test]
    fn same_host_ignores_mode_and_path() {
        let p = page("https://accounts.github.com/login");
        assert!(same_host(&p, &entry("https://accounts.github.com/other")));
        assert!(!same_host(&p, &entry("https://github.com")));
        assert!(!same_host(
            &page("http://accounts.github.com"),
            &entry("https://accounts.github.com")
        ));
        assert!(same_host(&page("https://x.com"), &entry("http://x.com")));
    }

    #[test]
    fn origin_carries_the_scheme_and_port() {
        assert_eq!(
            origin(&page("https://example.com/a/b?q=1")),
            "https://example.com"
        );
        assert_eq!(
            origin(&page("http://example.com:8080/a")),
            "http://example.com:8080"
        );
    }
}
