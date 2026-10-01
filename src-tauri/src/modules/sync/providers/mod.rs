//! The backends that sit behind `SyncProvider` in
//! `src-tauri/src/modules/sync/provider.rs`.
//!
//! One per file, each reachable only through `build` in that module, so adding
//! a backend is a new file here plus one arm there and touches nothing else.
//! `src-tauri/src/modules/sync/providers/sigv4.rs` is shared signing rather
//! than a backend of its own: any other service speaking the same signature
//! scheme would use it without a second copy. The two backends are directory
//! modules; `src-tauri/src/modules/sync/providers/http.rs` holds the HTTP shell
//! they share, since a request already decided is a request whichever protocol
//! built it, and `src-tauri/src/modules/sync/providers/xml_tree.rs` holds the
//! XML scan they share, since the walk does not depend on the element names the
//! two look up.
//!
//! THE SSRF GUARD LIVES HERE rather than in a module of its own, because its
//! only callers are the two backends below: a file whose whole contents are one
//! guard three files away is indirection for its own sake. Both providers call
//! it from the guard that runs before their first request.

pub mod http;
pub mod s3;
pub mod sigv4;
pub mod webdav;
pub mod xml_tree;

/// Install the process-wide TLS crypto provider, once.
///
/// reqwest is built with its `rustls-no-provider` feature, which deliberately
/// installs nothing, so every `Client::builder().build()` panics until the
/// process names a provider. `ring` is the provider the rest of this crate's
/// graph already compiles rustls with, so naming it here adds no crate and
/// changes no handshake.
///
/// Called from every provider that builds a client rather than from
/// `src-tauri/src/lib.rs`, because a unit test builds one too.
pub(crate) fn ensure_crypto_provider() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        // A provider already installed is the same one, so the refusal is not
        // a failure.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// True for the IPv4/IPv6 link-local ranges (169.254.0.0/16 and fe80::/10,
/// including IPv4-mapped IPv6). This is the SSRF-sensitive range that fronts the
/// cloud instance metadata service. Defined once so every caller checks the same
/// byte ranges.
pub(crate) fn ip_is_link_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => {
            (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6.to_ipv4().map(|m| m.is_link_local()).unwrap_or(false)
        }
    }
}

/// True for the cloud-metadata hostnames that resolve into the link-local range.
/// Expects an already-lowercased host.
pub(crate) fn is_metadata_hostname(host: &str) -> bool {
    host == "metadata.google.internal" || host == "metadata"
}

/// Rejects a sync endpoint aimed at the cloud instance metadata service or the
/// IPv4/IPv6 link-local ranges, the classic SSRF target used to steal cloud
/// credentials. The host is resolved first so a hostname pointing into that
/// space is caught as well. Loopback and private LAN ranges stay allowed: a
/// self-hosted WebDAV server or MinIO instance on the local network is a
/// supported target, and the endpoint here is one the user typed into Settings.
///
/// The refusal vocabulary is the contract, not an implementation detail: a
/// policy refusal starts with `blocked:`, while a parse error or a name that
/// did not resolve does not. The providers read that prefix to tell a refusal
/// (never retry) from a dropped network (retry), so the prefix is load-bearing.
pub(crate) async fn reject_metadata_ssrf(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|_| "invalid url".to_string())?;
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!("blocked: unsupported url scheme '{scheme}'"));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "url has no host".to_string())?
        .to_string();
    let host_l = host.to_ascii_lowercase();
    // These hostnames resolve into the metadata range; refuse them by name too.
    if is_metadata_hostname(&host_l) {
        return Err("blocked: cloud metadata endpoint".to_string());
    }
    // `Url::host_str` spells an IPv6 literal with its brackets, and neither
    // `Ipv6Addr::from_str` nor a resolver accepts those, so a `[fe80::1]`
    // endpoint would come back as a resolution failure rather than reaching
    // the range check below - a bypass that reads like a dropped network.
    // Bracket stripping is safe here because a domain name cannot contain one.
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = parsed.port_or_known_default().unwrap_or(443);
    // DNS resolution blocks; run it off the async runtime, then inspect every
    // candidate address. IP literals resolve to themselves (no DNS lookup).
    let ips = tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        (host.as_str(), port)
            .to_socket_addrs()
            .map(|it| it.map(|a| a.ip()).collect::<Vec<std::net::IpAddr>>())
    })
    .await
    .map_err(|e| format!("dns task failed: {e}"))?
    .map_err(|e| format!("dns resolve failed: {e}"))?;
    for ip in ips {
        if ip_is_link_local(ip) {
            return Err("blocked: link-local / cloud-metadata address".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_metadata_hostname_is_refused_by_name_before_any_lookup() {
        // Answered from the hostname alone, so this holds with no DNS at all.
        for url in [
            "http://metadata.google.internal/",
            "http://metadata.google.internal/latest/meta-data/",
            "https://METADATA.google.internal/dav",
        ] {
            let err = reject_metadata_ssrf(url)
                .await
                .expect_err("a metadata host must be refused");
            assert!(err.starts_with("blocked:"), "{url} gave {err}");
        }
    }

    #[tokio::test]
    async fn a_link_local_literal_is_refused_without_a_lookup() {
        // An IP literal resolves to itself, so the blocking task answers
        // without touching a resolver.
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://[fe80::1]/",
        ] {
            let err = reject_metadata_ssrf(url)
                .await
                .expect_err("a link-local literal must be refused");
            assert!(err.starts_with("blocked:"), "{url} gave {err}");
        }
    }

    #[tokio::test]
    async fn loopback_and_private_lan_addresses_stay_allowed() {
        // The intended targets for the self-hosted backends.
        for url in [
            "http://127.0.0.1:8099/dav",
            "http://192.168.1.10:9000/bucket",
            "http://[::1]:8099/dav",
        ] {
            reject_metadata_ssrf(url)
                .await
                .unwrap_or_else(|e| panic!("{url} must be allowed, got {e}"));
        }
    }

    #[tokio::test]
    async fn a_bad_url_is_not_a_policy_refusal() {
        // No `blocked:` prefix, so the providers classify these as anything
        // but `Blocked`.
        for url in ["not a url", ""] {
            let err = reject_metadata_ssrf(url).await.expect_err("must refuse");
            assert!(!err.starts_with("blocked:"), "{url} gave {err}");
        }
        let err = reject_metadata_ssrf("ftp://dav.example")
            .await
            .expect_err("a non-http scheme must be refused");
        assert!(err.starts_with("blocked:"), "{err}");
    }

    #[test]
    fn the_link_local_check_matches_the_ranges_it_names() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        for (ip, want) in [
            (IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)), true),
            (IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), false),
            (IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), false),
            (IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)), true),
            (
                IpAddr::V6(Ipv6Addr::new(0xfec0, 0, 0, 0, 0, 0, 0, 1)),
                false,
            ),
            (IpAddr::V6(Ipv6Addr::LOCALHOST), false),
        ] {
            assert_eq!(ip_is_link_local(ip), want, "{ip}");
        }
    }
}
