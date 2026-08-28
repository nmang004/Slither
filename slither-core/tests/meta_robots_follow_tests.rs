//! `<meta name="robots" content="none">` must stop link discovery.
//!
//! Google documents `none` as shorthand for `noindex, nofollow`, but the crawler
//! only ever matched the literal `nofollow` token, so a page marked `none` had
//! its links followed anyway — spending crawl budget on URLs the site asked not
//! to be crawled, and reporting them as part of the audit.
//!
//! The companion case matters just as much: `googlebot-news` is scoped to Google
//! News, so a News-only directive must NOT suppress link discovery for Search.

use slither_core::models::config::CrawlConfig;
use slither_core::models::CrawlEvent;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

fn doc(meta: &str, links: &str) -> String {
    format!(
        "<!doctype html><html><head><title>Link discovery fixture page</title>{meta}\
         <meta name=\"description\" content=\"A fixture page with a real description on it.\">\
         </head><body><h1>Fixture</h1><p>Body copy for the analyzers.</p>{links}</body></html>"
    )
}

/// The root carries `meta`, and links to `/reached/`. If link discovery runs,
/// the crawl finds two pages; if it is suppressed, one.
async fn spawn_site(meta: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());

    let handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();

                let response = if path == "/robots.txt" {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_string()
                } else {
                    let body = if path == "/" {
                        doc(meta, "<a href=\"/reached/\">reached</a>")
                    } else {
                        doc("", "<a href=\"/\">home</a>")
                    };
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

async fn crawl_count(base: &str) -> usize {
    let config = CrawlConfig {
        seed_url: format!("{base}/"),
        max_depth: 5,
        max_pages: 10,
        concurrency: 2,
        delay_ms: 0,
        user_agent: "Slither/0.1.0 (meta robots test)".to_string(),
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
    result.pages.len()
}

#[tokio::test]
async fn content_none_suppresses_link_discovery() {
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
    let (base, _s) = spawn_site(r#"<meta name="robots" content="none">"#).await;
    assert_eq!(
        crawl_count(&base).await,
        1,
        "`content=\"none\"` is shorthand for `noindex, nofollow`, so /reached/ must not be crawled"
    );
}

#[tokio::test]
async fn plain_nofollow_still_suppresses_link_discovery() {
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
    let (base, _s) = spawn_site(r#"<meta name="robots" content="nofollow">"#).await;
    assert_eq!(
        crawl_count(&base).await,
        1,
        "an explicit nofollow must hold"
    );
}

#[tokio::test]
async fn news_scoped_none_does_not_suppress_search_link_discovery() {
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
    let (base, _s) = spawn_site(r#"<meta name="googlebot-news" content="none">"#).await;
    assert_eq!(
        crawl_count(&base).await,
        2,
        "googlebot-news binds Google News only; general Search discovery must continue"
    );
}

#[tokio::test]
async fn no_directive_follows_links() {
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
    let (base, _s) = spawn_site("").await;
    assert_eq!(crawl_count(&base).await, 2, "control: links are followed");
}
