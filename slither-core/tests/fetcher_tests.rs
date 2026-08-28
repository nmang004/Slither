use slither_core::crawler::fetcher::Fetcher;

// These tests hit a live external service (httpbin.org) and are non-deterministic
// in CI/offline runs, so they are #[ignore]d. Run with `cargo test -- --ignored`.

#[tokio::test]
#[ignore = "hits live network (httpbin.org)"]
async fn test_fetch_success() {
    let fetcher = Fetcher::new("Slither/0.1.0 (test)", 15);
    let result = fetcher.fetch("https://httpbin.org/html").await;
    assert!(result.is_ok());
    let fetch = result.unwrap();
    assert_eq!(fetch.status, 200);
    assert!(fetch.body.contains("<html>") || fetch.body.contains("<h1>"));
    assert!(fetch.response_time_ms > 0);
}

#[tokio::test]
#[ignore = "hits live network (httpbin.org)"]
async fn test_fetch_redirect_no_follow() {
    let fetcher = Fetcher::new("Slither/0.1.0 (test)", 15);
    let result = fetcher.fetch("https://httpbin.org/redirect/1").await;
    assert!(result.is_ok());
    let fetch = result.unwrap();
    // Should NOT auto-follow — returns 302
    assert!(fetch.status == 302 || fetch.status == 301);
    assert!(fetch.redirect_location.is_some());
}

#[tokio::test]
#[ignore = "hits live network (httpbin.org)"]
async fn test_fetch_404() {
    let fetcher = Fetcher::new("Slither/0.1.0 (test)", 15);
    let result = fetcher.fetch("https://httpbin.org/status/404").await;
    assert!(result.is_ok());
    let fetch = result.unwrap();
    assert_eq!(fetch.status, 404);
}

#[tokio::test]
#[ignore = "hits live network (httpbin.org)"]
async fn test_fetch_timeout() {
    let fetcher = Fetcher::new("Slither/0.1.0 (test)", 1); // 1 second timeout
    let result = fetcher.fetch("https://httpbin.org/delay/5").await;
    assert!(result.is_err());
}
