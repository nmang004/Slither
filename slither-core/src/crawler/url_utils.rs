use anyhow::{Context, Result};
use url::Url;

/// Uppercase the hex digits of every percent-encoding triplet.
///
/// RFC 3986 §6.2.2.1: the hex digits in a triplet are case-insensitive and
/// should be normalized to uppercase, so `%c3%a9` and `%C3%A9` are the same
/// URI. Without this the two spellings became separate visited-set keys, so one
/// resource was fetched twice and then reported against itself as duplicate
/// content — including the heavily-weighted `exact_duplicate` check.
fn normalize_percent_case(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            out.push('%');
            out.push(bytes[i + 1].to_ascii_uppercase() as char);
            out.push(bytes[i + 2].to_ascii_uppercase() as char);
            i += 3;
        } else {
            // Not a triplet: copy the character whole so multi-byte sequences
            // survive intact.
            let ch_len = utf8_len(bytes[i]);
            let end = (i + ch_len).min(bytes.len());
            out.push_str(&s[i..end]);
            i = end;
        }
    }
    out
}

/// Byte length of the UTF-8 character starting with this lead byte.
fn utf8_len(lead: u8) -> usize {
    match lead {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        // Continuation or invalid byte: advance one so we always make progress.
        _ => 1,
    }
}

/// Normalize a URL for deduplication:
/// - Lowercase the host
/// - Remove default ports (80 for http, 443 for https)
/// - Remove fragments
/// - Sort query parameters (without decoding them)
/// - Uppercase percent-encoding triplets
/// - Remove trailing slash (except for root path)
pub fn normalize_url(raw: &str) -> Result<String> {
    let mut parsed = Url::parse(raw).with_context(|| format!("Invalid URL: {}", raw))?;

    // Remove fragment
    parsed.set_fragment(None);

    // Remove default ports
    if let Some(port) = parsed.port() {
        let is_default = (parsed.scheme() == "https" && port == 443)
            || (parsed.scheme() == "http" && port == 80);
        if is_default {
            parsed.set_port(None).ok();
        }
    }

    // Sort query parameters by their raw text.
    //
    // The previous implementation round-tripped through `query_pairs()`, which
    // percent-decodes as lossy UTF-8. Every byte sequence that is not valid
    // UTF-8 — a legacy ISO-8859-1 or Shift_JIS query, say — collapsed to
    // U+FFFD and re-encoded to `%EF%BF%BD`, so `?x=%E9` and `?x=%E8` became the
    // same key and the second URL was silently never crawled. It also rewrote
    // `?flag` to `?flag=`, which many frameworks treat as a different request.
    // Sorting the raw `k=v` text is enough to make ordering canonical and keeps
    // every byte exactly as the site wrote it.
    if let Some(query) = parsed.query() {
        if !query.is_empty() {
            let mut pairs: Vec<&str> = query.split('&').collect();
            pairs.sort_unstable();
            let sorted = pairs.join("&");
            parsed.set_query(Some(&sorted));
        }
    }

    let mut result = normalize_percent_case(parsed.as_str());

    // Remove trailing slash (but not for root path like "https://example.com/")
    if result.ends_with('/') && parsed.path() != "/" {
        result.pop();
    }

    Ok(result)
}

/// Check if two URLs belong to the same site: same host *and* same port.
///
/// Returns false if either URL has no host (e.g. data: or mailto: URLs).
///
/// The port is part of the origin (RFC 6454). Comparing the host alone meant a
/// staging app on `localhost:3000` that linked to a different app on
/// `localhost:8080` treated the second app as part of the same site — spending
/// the crawl budget on a foreign origin, merging both applications into one
/// health score and one link graph, and never fetching that origin's robots.txt.
/// The scheme is deliberately *not* compared, so an `http://` link on an
/// otherwise-HTTPS site is still treated as internal (it normally redirects).
pub fn is_same_domain(url_a: &str, url_b: &str) -> Result<bool> {
    let a = Url::parse(url_a).with_context(|| format!("Invalid URL: {}", url_a))?;
    let b = Url::parse(url_b).with_context(|| format!("Invalid URL: {}", url_b))?;
    match (a.host_str(), b.host_str()) {
        (Some(ah), Some(bh)) => {
            Ok(ah == bh && a.port_or_known_default() == b.port_or_known_default())
        }
        _ => Ok(false),
    }
}

/// Check if a URL is crawlable (http or https scheme).
pub fn is_crawlable_url(url: &str) -> bool {
    if url.starts_with('#')
        || url.starts_with("mailto:")
        || url.starts_with("tel:")
        || url.starts_with("javascript:")
        || url.starts_with("data:")
        || url.starts_with("ftp:")
    {
        return false;
    }

    match Url::parse(url) {
        Ok(parsed) => matches!(parsed.scheme(), "http" | "https"),
        Err(_) => false,
    }
}

/// Resolve a potentially relative URL against a base URL.
pub fn resolve_url(base: &str, relative: &str) -> Result<String> {
    let base_url = Url::parse(base).with_context(|| format!("Invalid base URL: {}", base))?;
    let resolved = base_url
        .join(relative)
        .with_context(|| format!("Failed to resolve '{}' against '{}'", relative, base))?;
    Ok(resolved.to_string())
}

#[cfg(test)]
mod normalization_tests {
    use super::{is_same_domain, normalize_url};

    /// RFC 3986 §6.2.2.1: the hex digits of a percent-encoding triplet are
    /// case-insensitive. Treating the two spellings as different URLs fetched
    /// one resource twice and then reported it as duplicate content with itself.
    #[test]
    fn percent_encoding_case_is_normalized() {
        let upper = normalize_url("https://example.com/caf%C3%A9").unwrap();
        let lower = normalize_url("https://example.com/caf%c3%a9").unwrap();
        assert_eq!(upper, lower);
        assert!(
            upper.ends_with("%C3%A9"),
            "should fold to uppercase: {upper}"
        );
    }

    /// Query strings must survive byte-for-byte. Round-tripping them through a
    /// lossy UTF-8 decode collapsed every non-UTF-8 byte to U+FFFD, so distinct
    /// legacy-encoded URLs became one key and the second was never crawled.
    #[test]
    fn non_utf8_query_bytes_stay_distinct() {
        let a = normalize_url("https://example.com/a?x=%E9").unwrap();
        let b = normalize_url("https://example.com/a?x=%E8").unwrap();
        assert_ne!(a, b, "distinct legacy-encoded queries must not collide");
        assert!(a.contains("%E9"), "original bytes must be preserved: {a}");
    }

    /// `?flag` and `?flag=` are different query strings to many frameworks.
    #[test]
    fn valueless_query_keys_are_preserved() {
        let bare = normalize_url("https://example.com/u?flag").unwrap();
        let empty = normalize_url("https://example.com/u?flag=").unwrap();
        assert!(bare.ends_with("?flag"), "got {bare}");
        assert_ne!(bare, empty);
    }

    /// Ordering is still canonical, so the same query written two ways dedupes.
    #[test]
    fn query_parameters_are_sorted() {
        let a = normalize_url("https://example.com/p?b=2&a=1").unwrap();
        let b = normalize_url("https://example.com/p?a=1&b=2").unwrap();
        assert_eq!(a, b);
    }

    /// The port is part of the origin: a staging app on :3000 linking to a
    /// different app on :8080 must not be crawled as the same site.
    #[test]
    fn a_different_port_is_a_different_site() {
        assert!(!is_same_domain("http://127.0.0.1:9140/a", "http://127.0.0.1:9141/b").unwrap());
        assert!(is_same_domain("http://127.0.0.1:9140/a", "http://127.0.0.1:9140/b").unwrap());
        // Default ports are implicit, so these are the same origin.
        assert!(is_same_domain("http://example.com/a", "http://example.com:80/b").unwrap());
        assert!(is_same_domain("https://example.com/a", "https://example.com:443/b").unwrap());
    }
}
