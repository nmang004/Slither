use slither_core::models::config::CrawlConfig;
use slither_core::models::CrawlEvent;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// Stand up a throwaway HTTP site of `num_pages` densely interlinked pages on a
/// random localhost port. Returns the base URL; the server task stops when the
/// returned handle is dropped. Used to exercise crawl-budget enforcement
/// deterministically without hitting the network.
async fn spawn_test_site(num_pages: usize) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());

    let handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/");

                let response = if path == "/robots.txt" {
                    // No robots file — the crawler treats this as allow-all.
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                } else {
                    let mut links = String::new();
                    for i in 0..num_pages {
                        links.push_str(&format!("<a href=\"/p{i}\">p{i}</a>"));
                    }
                    let body = format!(
                        "<!doctype html><html><head><title>Page {path}</title>\
                         <meta name=\"description\" content=\"test page {path}\"></head>\
                         <body><h1>Page {path}</h1>{links}</body></html>"
                    );
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (base, handle)
}

/// The crawl budget must be an exact upper bound. Regression test for a race
/// where the limit was checked against a counter bumped only after each fetch
/// finished, letting up to `concurrency` extra tasks spawn while requests were
/// in flight (e.g. `--max-pages 12` yielding 15 pages).
#[tokio::test]
async fn test_max_pages_is_respected_exactly() {
    // The SSRF guard blocks loopback targets by default; opt in for this
    // in-process fixture. No sibling test asserts the guard blocks, so the
    // process-wide flag is safe here.
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");

    // 30 interlinked pages available, but the budget is 8 with concurrency 4.
    let (base, _server) = spawn_test_site(30).await;

    let config = CrawlConfig {
        seed_url: format!("{base}/"),
        max_depth: 10,
        max_pages: 8,
        concurrency: 4,
        delay_ms: 0,
        user_agent: "Slither/0.1.0 (cap test)".to_string(),
        ignore_robots: true,
        follow_subdomains: false,
        timeout_seconds: 10,
        generate_html: false,
        output_path: None,
        json_compact: false,
        include_body_text: false,
        summary_only: false,
        generate_csv: false,
        backend: "local".to_string(),
        cf_account_id: None,
        cf_api_token: None,
        skip_header_check: false,
        chrome_path: None,
        headless: true,
        render_wait_ms: 0,
        pagespeed: false,
        pagespeed_key: None,
        pagespeed_sample: None,
        pagespeed_strategy: "mobile".to_string(),
    };

    let (event_tx, mut event_rx) = mpsc::channel::<CrawlEvent>(256);
    let drain = tokio::spawn(async move { while event_rx.recv().await.is_some() {} });

    let result = slither_core::crawler::crawl(config, event_tx)
        .await
        .expect("crawl should succeed");
    drain.await.unwrap();

    assert_eq!(
        result.pages.len(),
        8,
        "budget of 8 must be honored exactly, got {} pages",
        result.pages.len()
    );
    assert_eq!(result.crawl_metadata.pages_crawled, 8);
}

/// Integration test: crawl example.com and verify the output schema.
#[tokio::test]
async fn test_crawl_example_com_produces_valid_output() {
    let config = CrawlConfig {
        seed_url: "https://example.com".to_string(),
        max_depth: 2,
        max_pages: 5,
        concurrency: 2,
        delay_ms: 100,
        user_agent: "Slither/0.1.0 (integration test)".to_string(),
        ignore_robots: false,
        follow_subdomains: false,
        timeout_seconds: 10,
        generate_html: false,
        output_path: None,
        json_compact: false,
        include_body_text: false,
        summary_only: false,
        generate_csv: false,
        backend: "local".to_string(),
        cf_account_id: None,
        cf_api_token: None,
        skip_header_check: false,
        chrome_path: None,
        headless: true,
        render_wait_ms: 2000,
        pagespeed: false,
        pagespeed_key: None,
        pagespeed_sample: None,
        pagespeed_strategy: "mobile".to_string(),
    };

    let (event_tx, mut event_rx) = mpsc::channel::<CrawlEvent>(64);

    // Spawn event consumer to prevent channel backup
    let event_handle = tokio::spawn(async move {
        let mut got_started = false;
        let mut got_completed = false;
        while let Some(event) = event_rx.recv().await {
            match event {
                CrawlEvent::Started { .. } => got_started = true,
                CrawlEvent::Completed { .. } => got_completed = true,
                _ => {}
            }
        }
        (got_started, got_completed)
    });

    let result = slither_core::crawler::crawl(config, event_tx).await;
    assert!(result.is_ok(), "Crawl should succeed: {:?}", result.err());

    let crawl_result = result.unwrap();

    // Verify metadata
    assert_eq!(crawl_result.slither_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(crawl_result.crawl_metadata.domain, "example.com");
    assert!(crawl_result.crawl_metadata.duration_ms > 0);
    assert!(crawl_result.crawl_metadata.pages_crawled > 0);

    // Verify pages
    assert!(!crawl_result.pages.is_empty());
    let first_page = &crawl_result.pages[0];
    assert!(!first_page.url.is_empty());
    assert!(first_page.status > 0);

    // Verify summary
    assert!(crawl_result.summary.total_pages > 0);

    // Verify JSON serialization
    let json = slither_core::report::json::serialize_crawl_result(&crawl_result);
    assert!(json.is_ok(), "JSON serialization should succeed");
    let json_str = json.unwrap();
    assert!(json_str.contains("\"slither_version\""));
    assert!(json_str.contains("\"crawl_metadata\""));
    assert!(json_str.contains("\"pages\""));
    assert!(json_str.contains("\"issues\""));
    assert!(json_str.contains("\"summary\""));

    // Verify JSON parses back
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("JSON should parse");
    assert!(parsed.is_object());
    assert!(parsed["crawl_metadata"]["domain"].as_str() == Some("example.com"));

    // Verify new summary fields
    assert!(!crawl_result.summary.grade.is_empty());
    assert!(!crawl_result.summary.grade_verdict.is_empty());
    // health_score is 0-100, grade is computed
    assert!(crawl_result.summary.health_score <= 100);

    // Verify export_settings is present in JSON
    assert!(json_str.contains("\"export_settings\""));
    assert!(json_str.contains("\"health_score\""));
    assert!(json_str.contains("\"grade\""));
    assert!(json_str.contains("\"response_time_p50_ms\""));
    assert!(json_str.contains("\"response_time_p95_ms\""));

    // Verify CSV generation
    let csv = slither_core::report::csv::generate_csv(&crawl_result);
    assert!(csv.is_ok(), "CSV generation should succeed");
    let csv_str = csv.unwrap();
    assert!(csv_str.starts_with("url,status,"));

    // Verify HTML report generation
    let html = slither_core::report::html::render_html_report(&crawl_result);
    assert!(html.is_ok(), "HTML report should render");
    let html_str = html.unwrap();
    assert!(html_str.contains("tab-overview"));
    assert!(html_str.contains("score-breakdown")); // score breakdown section
    assert!(html_str.contains("tab-structure")); // structure tab
    assert!(html_str.contains("structure-stats")); // structure stats bar

    // Check events were received
    let (got_started, got_completed) = event_handle.await.unwrap();
    assert!(got_started, "Should have received Started event");
    assert!(got_completed, "Should have received Completed event");
}

/// A site whose canonical URLs carry a trailing slash: `/x/` serves 200 and
/// `/x` 301s to it, which is how most CMSes are configured.
async fn spawn_trailing_slash_site() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let host = listener.local_addr().unwrap().to_string();

    let handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let host = host.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();

                let response = if path == "/robots.txt" {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                } else if path != "/" && !path.ends_with('/') {
                    // The canonical form carries the slash.
                    format!(
                        "HTTP/1.1 301 Moved Permanently\r\nLocation: http://{host}{path}/\r\n\
                         Content-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                } else {
                    let links: String = (0..4)
                        .map(|i| format!("<a href=\"/p{i}/\">p{i}</a>"))
                        .collect();
                    let body = format!(
                        "<!doctype html><html><head><title>Page {path}</title>\
                         <meta name=\"description\" content=\"desc {path}\"></head>\
                         <body><h1>Page</h1>{links}</body></html>"
                    );
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (base, handle)
}

/// Regression: URL normalization strips the trailing slash, and that normalized
/// form was what got enqueued and requested. On a site whose canonical URLs end
/// in a slash, every page was therefore fetched at a URL that 301s — so every
/// page was recorded under a non-canonical URL and reported as a redirect, and
/// `slither sitemap` emitted a sitemap full of redirecting URLs.
///
/// Normalization must remain the dedup key only; the request uses the URL as
/// it was actually linked.
#[tokio::test]
async fn trailing_slash_sites_are_crawled_at_their_canonical_urls() {
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
    let (base, _server) = spawn_trailing_slash_site().await;

    let config = CrawlConfig {
        seed_url: format!("{base}/"),
        max_depth: 5,
        max_pages: 5,
        concurrency: 2,
        delay_ms: 0,
        user_agent: "Slither/0.1.0 (slash test)".to_string(),
        ignore_robots: true,
        follow_subdomains: false,
        timeout_seconds: 10,
        generate_html: false,
        output_path: None,
        json_compact: false,
        include_body_text: false,
        summary_only: false,
        generate_csv: false,
        backend: "local".to_string(),
        cf_account_id: None,
        cf_api_token: None,
        skip_header_check: false,
        chrome_path: None,
        headless: true,
        render_wait_ms: 0,
        pagespeed: false,
        pagespeed_key: None,
        pagespeed_sample: None,
        pagespeed_strategy: "mobile".to_string(),
    };

    let (event_tx, mut event_rx) = mpsc::channel::<CrawlEvent>(256);
    let drain = tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
    let result = slither_core::crawler::crawl(config, event_tx)
        .await
        .expect("crawl should succeed");
    drain.await.unwrap();

    let non_canonical: Vec<&str> = result
        .pages
        .iter()
        .filter(|p| p.url != format!("{base}/") && !p.url.ends_with('/'))
        .map(|p| p.url.as_str())
        .collect();
    assert!(
        non_canonical.is_empty(),
        "pages must be recorded at the URL that served them: {non_canonical:?}"
    );

    let redirected: Vec<&str> = result
        .pages
        .iter()
        .filter(|p| p.redirect_chain.is_some())
        .map(|p| p.url.as_str())
        .collect();
    assert!(
        redirected.is_empty(),
        "requesting the canonical URL must not go through a redirect: {redirected:?}"
    );
}

/// A site that redirects an allowed URL into a path robots.txt forbids.
async fn spawn_redirect_into_blocked_site() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());

    let handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();

                let response = if path == "/robots.txt" {
                    let body = "User-agent: *\nDisallow: /blocked/\n";
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\
                         Connection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else if path == "/go" {
                    "HTTP/1.1 302 Found\r\nLocation: /blocked/secret\r\nContent-Length: 0\r\n\
                     Connection: close\r\n\r\n"
                        .to_string()
                } else {
                    let title = if path.contains("blocked") {
                        "SECRET BLOCKED CONTENT"
                    } else {
                        "Home"
                    };
                    let body = format!(
                        "<!doctype html><html><head><title>{title}</title>\
                         <meta name=\"description\" content=\"d\"></head>\
                         <body><h1>{title}</h1><a href=\"/go\">go</a></body></html>"
                    );
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (base, handle)
}

/// Regression: robots.txt applies to every request, including the ones made
/// while following a redirect. The check ran only against the queued URL, so a
/// 302 carried the crawler into a `Disallow`ed path and its content was stored
/// under the allowed URL — the audit then contained content from a path the
/// site had forbidden, mislabelled as a different page.
#[tokio::test]
async fn robots_is_enforced_on_redirect_targets() {
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
    let (base, _server) = spawn_redirect_into_blocked_site().await;

    let config = CrawlConfig {
        seed_url: format!("{base}/"),
        max_depth: 5,
        max_pages: 10,
        concurrency: 2,
        delay_ms: 0,
        user_agent: "Slither/0.1.0 (robots redirect test)".to_string(),
        ignore_robots: false,
        follow_subdomains: false,
        timeout_seconds: 10,
        generate_html: false,
        output_path: None,
        json_compact: false,
        include_body_text: true,
        summary_only: false,
        generate_csv: false,
        backend: "local".to_string(),
        cf_account_id: None,
        cf_api_token: None,
        skip_header_check: false,
        chrome_path: None,
        headless: true,
        render_wait_ms: 0,
        pagespeed: false,
        pagespeed_key: None,
        pagespeed_sample: None,
        pagespeed_strategy: "mobile".to_string(),
    };

    let (event_tx, mut event_rx) = mpsc::channel::<CrawlEvent>(256);
    let drain = tokio::spawn(async move { while event_rx.recv().await.is_some() {} });
    let result = slither_core::crawler::crawl(config, event_tx)
        .await
        .expect("crawl should succeed");
    drain.await.unwrap();

    let leaked: Vec<&str> = result
        .pages
        .iter()
        .filter(|p| {
            p.title.as_deref() == Some("SECRET BLOCKED CONTENT")
                || p.url.contains("/blocked/")
                || p.body_text.contains("SECRET BLOCKED CONTENT")
        })
        .map(|p| p.url.as_str())
        .collect();

    assert!(
        leaked.is_empty(),
        "content from a Disallowed path must never reach the report: {leaked:?}"
    );
}
