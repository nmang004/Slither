use slither_core::crawler::url_utils::{is_crawlable_url, is_same_domain, normalize_url};

#[test]
fn test_normalize_removes_trailing_slash() {
    assert_eq!(
        normalize_url("https://example.com/page/").unwrap(),
        "https://example.com/page"
    );
}

#[test]
fn test_normalize_lowercases_host() {
    assert_eq!(
        normalize_url("https://EXAMPLE.COM/Page").unwrap(),
        "https://example.com/Page"
    );
}

#[test]
fn test_normalize_removes_fragment() {
    assert_eq!(
        normalize_url("https://example.com/page#section").unwrap(),
        "https://example.com/page"
    );
}

#[test]
fn test_normalize_removes_default_port() {
    assert_eq!(
        normalize_url("https://example.com:443/page").unwrap(),
        "https://example.com/page"
    );
    assert_eq!(
        normalize_url("http://example.com:80/page").unwrap(),
        "http://example.com/page"
    );
}

#[test]
fn test_normalize_sorts_query_params() {
    assert_eq!(
        normalize_url("https://example.com/page?z=1&a=2&m=3").unwrap(),
        "https://example.com/page?a=2&m=3&z=1"
    );
}

#[test]
fn test_normalize_preserves_path_case() {
    // Paths are case-sensitive on most servers
    assert_eq!(
        normalize_url("https://example.com/About-Us").unwrap(),
        "https://example.com/About-Us"
    );
}

#[test]
fn test_same_domain() {
    assert!(is_same_domain("https://example.com/a", "https://example.com/b").unwrap());
    assert!(!is_same_domain("https://example.com/a", "https://other.com/b").unwrap());
}

#[test]
fn test_same_domain_with_subdomain() {
    assert!(!is_same_domain("https://www.example.com/a", "https://blog.example.com/b").unwrap());
}

#[test]
fn test_crawlable_url() {
    assert!(is_crawlable_url("https://example.com/page"));
    assert!(is_crawlable_url("http://example.com/page"));
    assert!(!is_crawlable_url("mailto:test@example.com"));
    assert!(!is_crawlable_url("javascript:void(0)"));
    assert!(!is_crawlable_url("tel:+1234567890"));
    assert!(!is_crawlable_url("#section"));
}
