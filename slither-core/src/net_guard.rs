//! SSRF protection: block requests to loopback, private, link-local, and other
//! non-public address ranges.
//!
//! Slither accepts arbitrary URLs from the CLI, the REST API, and MCP tool calls.
//! Without a guard, `crawl http://169.254.169.254/…` or `http://127.0.0.1:…`
//! turns the crawler into a server-side request forgery primitive that can read
//! internal services and cloud metadata endpoints. This module is the single
//! source of truth for "is this host allowed as a crawl/webhook target?".
//!
//! Enforcement points:
//! - [`Fetcher::fetch`](crate::crawler::fetcher::Fetcher) — the chokepoint every
//!   HTTP request passes through, so it covers seeds, discovered links, robots,
//!   sitemaps, and every redirect hop.
//! - Webhook dispatch validation.
//!
//! Set `SLITHER_ALLOW_PRIVATE_TARGETS=1` to allow private/internal targets for
//! legitimate intranet audits.

use std::net::{IpAddr, Ipv4Addr};

/// Returns true when the operator has explicitly opted into crawling
/// private/internal targets via `SLITHER_ALLOW_PRIVATE_TARGETS=1`.
pub fn private_targets_allowed() -> bool {
    matches!(
        std::env::var("SLITHER_ALLOW_PRIVATE_TARGETS")
            .ok()
            .as_deref(),
        Some("1") | Some("true") | Some("TRUE")
    )
}

/// True if `ip` is not a routable public address and should be blocked as an
/// SSRF target. Covers loopback, private (RFC 1918), link-local (incl. the
/// 169.254.169.254 cloud metadata endpoint), unspecified, broadcast,
/// documentation, CGNAT (100.64.0.0/10), IPv6 ULA (fc00::/7), IPv6 link-local
/// (fe80::/10), and IPv4-mapped IPv6 forms of all the above.
pub fn is_blocked_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_v4(&mapped);
            }
            // Transition mechanisms carry an IPv4 address inside the v6 form; a
            // literal like 2002:7f00:1:: would otherwise reach 127.0.0.1
            // through a 6to4 relay while looking like an ordinary public v6.
            if let Some(embedded) = embedded_ipv4(v6) {
                if is_blocked_v4(&embedded) {
                    return true;
                }
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 unique local
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        }
    }
}

/// Extract an IPv4 address embedded in an IPv6 transition form, if any:
/// 6to4 (`2002::/16`), Teredo (`2001:0::/32`, where the client address is
/// stored bitwise-inverted), and NAT64 (`64:ff9b::/96`).
fn embedded_ipv4(v6: &std::net::Ipv6Addr) -> Option<Ipv4Addr> {
    let s = v6.segments();
    let join = |hi: u16, lo: u16| Ipv4Addr::from(((hi as u32) << 16) | lo as u32);

    if s[0] == 0x2002 {
        return Some(join(s[1], s[2])); // 6to4
    }
    if s[0] == 0x2001 && s[1] == 0x0000 {
        return Some(join(!s[6], !s[7])); // Teredo client, obfuscated
    }
    if s[0] == 0x0064 && s[1] == 0xff9b && s[2] == 0 && s[3] == 0 && s[4] == 0 && s[5] == 0 {
        return Some(join(s[6], s[7])); // NAT64 well-known prefix
    }
    None
}

fn is_blocked_v4(v4: &Ipv4Addr) -> bool {
    let o = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_documentation()
        || (o[0] == 100 && (64..128).contains(&o[1])) // 100.64.0.0/10 CGNAT
}

/// True if a host string is an obvious loopback name that should be blocked
/// without a DNS lookup.
pub fn is_loopback_name(host: &str) -> bool {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost") || h.to_ascii_lowercase().ends_with(".localhost")
}

/// Validate that a URL is safe to request: an http(s) scheme with a host that
/// does not resolve to a blocked address range. Returns `Ok(())` when allowed.
///
/// Fails closed: an unresolvable host is treated as blocked so a name that
/// resolves to a private IP cannot slip through on a transient DNS error.
/// A no-op (always `Ok`) when `SLITHER_ALLOW_PRIVATE_TARGETS` is set.
pub async fn check_url_allowed(url: &str) -> Result<(), String> {
    if private_targets_allowed() {
        return Ok(());
    }

    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL {url}: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => return Err(format!("unsupported URL scheme '{other}' for {url}")),
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| format!("URL has no host: {url}"))?;

    if is_loopback_name(host) {
        return Err(blocked_msg(url, host));
    }

    // A literal IP host: check directly, no DNS.
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = stripped.parse::<IpAddr>() {
        if is_blocked_ip(&ip) {
            return Err(blocked_msg(url, host));
        }
        return Ok(());
    }

    // Resolve and check every returned address.
    let port = parsed.port_or_known_default().unwrap_or(0);
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("cannot resolve host '{host}' for {url}: {e}"))?;

    let mut any = false;
    for addr in addrs {
        any = true;
        if is_blocked_ip(&addr.ip()) {
            return Err(blocked_msg(url, host));
        }
    }
    if !any {
        return Err(format!("host '{host}' did not resolve to any address"));
    }
    Ok(())
}

/// A reqwest DNS resolver that applies [`is_blocked_ip`] to the addresses the
/// connection will actually use.
///
/// [`check_url_allowed`] alone is a time-of-check/time-of-use gap: it resolves
/// the host, vets the addresses, and then reqwest resolves the name *again* when
/// it opens the socket. An attacker serving a low-TTL record that answers public
/// on the first lookup and private on the second lands the request on an
/// internal target. Installing this resolver on the client closes the gap,
/// because the addresses vetted here are the ones handed to the connector.
#[derive(Debug, Default, Clone)]
pub struct GuardedResolver;

impl reqwest::dns::Resolve for GuardedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();

            if private_targets_allowed() {
                let addrs: Vec<std::net::SocketAddr> =
                    tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
                return Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs);
            }

            if is_loopback_name(&host) {
                return Err(resolver_blocked(&host));
            }

            let addrs: Vec<std::net::SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();

            if addrs.is_empty() {
                return Err(format!("host '{host}' did not resolve to any address").into());
            }

            // Any blocked address fails the whole resolution rather than being
            // filtered out: a name answering with a mix of public and private
            // addresses is the rebinding pattern itself.
            if addrs.iter().any(|a| is_blocked_ip(&a.ip())) {
                return Err(resolver_blocked(&host));
            }

            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

fn resolver_blocked(host: &str) -> Box<dyn std::error::Error + Send + Sync> {
    format!(
        "refusing to connect to '{host}': it resolves to a private/loopback \
         address. Set SLITHER_ALLOW_PRIVATE_TARGETS=1 to allow internal targets."
    )
    .into()
}

fn blocked_msg(url: &str, host: &str) -> String {
    format!(
        "refusing to request {url}: host '{host}' resolves to a private/loopback \
         address. Set SLITHER_ALLOW_PRIVATE_TARGETS=1 to allow internal targets."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_loopback_and_private_v4() {
        assert!(is_blocked_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"10.0.0.5".parse().unwrap()));
        assert!(is_blocked_ip(&"192.168.1.1".parse().unwrap()));
        assert!(is_blocked_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_blocked_ip(&"169.254.169.254".parse().unwrap())); // cloud metadata
        assert!(is_blocked_ip(&"100.64.0.1".parse().unwrap())); // CGNAT
        assert!(is_blocked_ip(&"0.0.0.0".parse().unwrap()));
    }

    #[test]
    fn allows_public_v4() {
        assert!(!is_blocked_ip(&"93.184.216.34".parse().unwrap())); // example.com
        assert!(!is_blocked_ip(&"8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn blocks_v6_loopback_ula_linklocal_and_mapped() {
        assert!(is_blocked_ip(&"::1".parse().unwrap()));
        assert!(is_blocked_ip(&"fd00::1".parse().unwrap())); // ULA
        assert!(is_blocked_ip(&"fe80::1".parse().unwrap())); // link-local
        assert!(is_blocked_ip(&"::ffff:127.0.0.1".parse().unwrap())); // mapped loopback
        assert!(!is_blocked_ip(
            &"2606:2800:220:1:248:1893:25c8:1946".parse().unwrap()
        ));
    }

    /// Transition forms that tunnel to a private IPv4 must be blocked; the
    /// same forms carrying a public IPv4 must still be allowed.
    #[test]
    fn blocks_ipv4_embedded_in_v6_transition_forms() {
        // 6to4 (2002::/16) wrapping 127.0.0.1 and 169.254.169.254.
        assert!(is_blocked_ip(&"2002:7f00:1::".parse().unwrap()));
        assert!(is_blocked_ip(&"2002:a9fe:a9fe::".parse().unwrap()));
        // 6to4 wrapping a public address stays allowed (93.184.216.34).
        assert!(!is_blocked_ip(&"2002:5db8:d822::".parse().unwrap()));

        // NAT64 well-known prefix wrapping 10.0.0.1.
        assert!(is_blocked_ip(&"64:ff9b::a00:1".parse().unwrap()));

        // Teredo stores the client address inverted: !127.0.0.1 == 128.255.255.254.
        assert!(is_blocked_ip(&"2001:0:0:0:0:0:80ff:fffe".parse().unwrap()));
    }

    #[test]
    fn loopback_names() {
        assert!(is_loopback_name("localhost"));
        assert!(is_loopback_name("app.localhost"));
        assert!(!is_loopback_name("example.com"));
    }

    #[tokio::test]
    async fn check_url_blocks_literal_private() {
        assert!(check_url_allowed("http://127.0.0.1:8080/").await.is_err());
        assert!(
            check_url_allowed("http://169.254.169.254/latest/meta-data/")
                .await
                .is_err()
        );
        assert!(check_url_allowed("http://[::1]/").await.is_err());
        assert!(check_url_allowed("http://localhost:3000/").await.is_err());
    }

    /// The resolver is the TOCTOU-free enforcement point: it must refuse a name
    /// that resolves to a blocked address, since these are the exact addresses
    /// the connector will dial.
    #[tokio::test]
    async fn guarded_resolver_blocks_loopback_names() {
        use reqwest::dns::Resolve;
        use std::str::FromStr;

        let resolver = GuardedResolver;
        let name = reqwest::dns::Name::from_str("localhost").unwrap();
        assert!(resolver.resolve(name).await.is_err());
    }

    #[tokio::test]
    async fn check_url_rejects_bad_scheme() {
        assert!(check_url_allowed("ftp://example.com/").await.is_err());
        assert!(check_url_allowed("file:///etc/passwd").await.is_err());
    }
}
