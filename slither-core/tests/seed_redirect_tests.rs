//! Regression tests for seeds that redirect.
//!
//! Seeding `https://www.example.com/` when the site canonicalizes to the apex
//! is the single most common thing a consultant types, and it used to collapse
//! the entire crawl to one page while still reporting a confident grade. Two
//! distinct defects produced that:
//!
//! 1. The crawl scope was established from the URL the user typed, but pages
//!    adopt the URL that finally served them. After a host-level redirect every
//!    discovered link looked external and was discarded.
//! 2. The queue was seeded with the user's URL while the visited set was seeded
//!    with the resolved one, so the landing page deduped against a key derived
//!    from its own destination and was dropped — taking the homepage out of the
//!    report and making everything it links to look like an orphan.
//!
//! Both are silent failures: exit 0, `pages_errored` 0, no warning. These tests
//! pin the behaviour by crawling the same fixture twice under different seeds
//! and requiring the results to be identical.

use slither_core::models::config::CrawlConfig;
use slither_core::models::CrawlEvent;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

fn header<'a>(req: &'a str, name: &str) -> Option<&'a str> {
    let want = format!("{}:", name.to_ascii_lowercase());
    req.lines()
        .find(|l| l.to_ascii_lowercase().starts_with(&want))
        .map(|l| l[want.len()..].trim())
}

fn page(path: &str, links: &[&str]) -> String {
    let anchors: String = links
        .iter()
        .map(|l| format!("<a href=\"{l}\">{l}</a>"))
        .collect();
    format!(
        "<!doctype html><html><head><title>Fixture page {path} for the redirect test</title>\
         <meta name=\"description\" content=\"A fixture page at {path} carrying enough text to read as a genuine description.\">\
         </head><body><h1>Fixture page {path}</h1>\
         <p>This page exists so the crawler has real content to record.</p>{anchors}</body></html>"
    )
}

fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn moved(location: &str) -> String {
    format!(
        "HTTP/1.1 301 Moved Permanently\r\nLocation: {location}\r\n\
         Content-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

const NOT_FOUND: &str = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

/// Serve a small site on every loopback address, and 301 any request that
/// arrives with a `localhost` Host header to the same path on `127.0.0.1`.
///
/// `localhost` and `127.0.0.1` are distinct hosts to the crawler but the same
/// listening socket, which reproduces a www-to-apex canonical redirect without
/// depending on external DNS. Binding `0.0.0.0` (rather than `127.0.0.1`) is
/// what makes both names reach the fixture.
async fn spawn_host_redirect_site() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let via_alias = header(&req, "host").is_some_and(|h| h.starts_with("localhost"));

                let response = if path == "/robots.txt" {
                    NOT_FOUND.to_string()
                } else if via_alias {
                    moved(&format!("http://127.0.0.1:{port}{path}"))
                } else {
                    ok(&page(&path, &["/", "/a/", "/b/", "/c/"]))
                };

                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (port, handle)
}

/// Serve a site whose root 301s to `/home/`, the shape of a CMS that moves its
/// homepage behind a path prefix.
async fn spawn_path_redirect_site() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();

                let response = match path.as_str() {
                    "/robots.txt" => NOT_FOUND.to_string(),
                    "/" => moved(&format!("http://127.0.0.1:{port}/home/")),
                    _ => ok(&page(&path, &["/home/", "/a/", "/b/"])),
                };

                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });

    (port, handle)
}

fn config(seed_url: String) -> CrawlConfig {
    CrawlConfig {
        seed_url,
        max_depth: 10,
        max_pages: 20,
        concurrency: 2,
        delay_ms: 0,
        user_agent: "Slither/0.1.0 (seed redirect test)".to_string(),
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
    }
}

/// Crawl `seed_url` and return the discovered page URLs with the ephemeral port
/// stripped, so results from two different seeds can be compared directly.
async fn crawled_paths(seed_url: String, port: u16) -> Vec<String> {
    let (event_tx, mut event_rx) = mpsc::channel::<CrawlEvent>(256);
    let drain = tokio::spawn(async move { while event_rx.recv().await.is_some() {} });

    let result = slither_core::crawler::crawl(config(seed_url), event_tx)
        .await
        .expect("crawl should succeed");
    drain.await.unwrap();

    let mut paths: Vec<String> = result
        .pages
        .iter()
        .map(|p| p.url.replace(&format!(":{port}"), ""))
        .collect();
    paths.sort();
    paths
}

/// A seed that redirects to a different host must audit the host it lands on,
/// not discard every link as external.
#[tokio::test]
async fn seed_host_redirect_audits_the_destination_host() {
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
    let (port, _server) = spawn_host_redirect_site().await;

    let redirected = crawled_paths(format!("http://localhost:{port}/"), port).await;
    let direct = crawled_paths(format!("http://127.0.0.1:{port}/"), port).await;

    assert!(
        redirected.len() > 1,
        "a seed that redirects to another host collapsed the crawl to {} page(s): {redirected:?}",
        redirected.len()
    );
    assert_eq!(
        redirected, direct,
        "seeding via a redirecting host must produce the same audit as seeding the destination"
    );
}

/// The page the seed redirects to is the homepage. It must appear in the
/// report; dropping it also strips the inbound links of everything it points
/// at, which surfaces as a page of phantom orphans.
#[tokio::test]
async fn seed_redirect_landing_page_is_recorded() {
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
    let (port, _server) = spawn_path_redirect_site().await;

    let paths = crawled_paths(format!("http://127.0.0.1:{port}/"), port).await;

    assert!(
        paths.iter().any(|u| u.ends_with("/home/")),
        "the page the seed redirected to is missing from the crawl: {paths:?}"
    );
}
