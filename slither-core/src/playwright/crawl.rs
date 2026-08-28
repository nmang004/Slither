use crate::models::config::CrawlConfig;
use crate::models::page::PageData;
use crate::models::CrawlEvent;
use crate::playwright::render::render_page;
use crate::playwright::PlaywrightClient;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tracing::{info, warn};
use url::Url;

/// Run a hybrid Playwright crawl: local crawler for URL discovery, then
/// render each discovered page through Chrome via CDP.
///
/// This mirrors the two-phase approach used by the Cloudflare backend:
///
/// - **Phase 1 (discover):** Run the local `crawl()` to find all reachable
///   URLs. Forward `QueueUpdate` events so the CLI can show progress.
/// - **Phase 2 (render):** For each discovered URL whose host matches the
///   seed (status 200-399), render it through Chrome and parse the
///   JS-rendered HTML into `PageData`.
///
/// On render failure the locally-crawled static HTML is used as a fallback
/// so no pages are silently lost.
///
/// Returns `(pages, pages_crawled, pages_errored, pages_skipped_robots)`.
pub async fn run_playwright_crawl(
    client: &PlaywrightClient,
    seed_url: &str,
    config: &CrawlConfig,
    event_tx: Option<mpsc::UnboundedSender<CrawlEvent>>,
) -> anyhow::Result<(Vec<PageData>, u32, u32, u32)> {
    info!("Starting hybrid Playwright crawl: local discovery + Chrome rendering");

    // ------------------------------------------------------------------
    // Phase 1: Local crawl for URL discovery
    // ------------------------------------------------------------------
    let discovery_config = CrawlConfig {
        seed_url: seed_url.to_string(),
        max_pages: config.max_pages,
        max_depth: config.max_depth,
        follow_subdomains: config.follow_subdomains,
        concurrency: config.concurrency.min(3),
        delay_ms: config.delay_ms.max(250),
        user_agent: config.user_agent.clone(),
        ignore_robots: config.ignore_robots,
        timeout_seconds: config.timeout_seconds,
        ..CrawlConfig::default()
    };

    let (local_tx, mut local_rx) = tokio::sync::mpsc::channel::<CrawlEvent>(256);

    // Forward discovery progress events to caller
    let event_tx_clone = event_tx.clone();
    let forward_handle = tokio::spawn(async move {
        while let Some(event) = local_rx.recv().await {
            if let Some(tx) = &event_tx_clone {
                if matches!(&event, CrawlEvent::QueueUpdate { .. }) {
                    let _ = tx.send(event);
                }
            }
        }
    });

    let crawl_handle =
        tokio::spawn(async move { crate::crawler::crawl(discovery_config, local_tx).await });

    let local_result = crawl_handle
        .await
        .map_err(|e| anyhow::anyhow!("Local crawl task failed: {}", e))?
        .map_err(|e| anyhow::anyhow!("Local crawl failed: {}", e))?;

    // Wait for event forwarding to finish
    let _ = forward_handle.await;

    // ------------------------------------------------------------------
    // Filter discovered URLs to same host as seed, status 200-399
    // ------------------------------------------------------------------
    let seed_host = Url::parse(seed_url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default();

    // Build a lookup of locally-crawled pages for fallback
    let local_pages: std::collections::HashMap<String, PageData> = local_result
        .pages
        .iter()
        .filter(|p| p.status >= 200 && p.status < 400)
        .filter(|p| {
            Url::parse(&p.url)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .map(|h| h == seed_host)
                .unwrap_or(false)
        })
        .map(|p| (p.url.clone(), p.clone()))
        .collect();

    let urls: Vec<String> = local_pages.keys().cloned().collect();
    let total_urls = urls.len() as u32;
    info!(
        "Local discovery found {} URLs, rendering via Chrome",
        total_urls
    );

    // ------------------------------------------------------------------
    // Phase 2: Render each URL through Chrome
    // ------------------------------------------------------------------
    let semaphore = Arc::new(Semaphore::new(config.concurrency.min(6) as usize));
    let pages: Arc<tokio::sync::Mutex<Vec<PageData>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let rendered_count = Arc::new(AtomicU32::new(0));
    let errored_count = Arc::new(AtomicU32::new(0));
    let render_wait_ms = config.render_wait_ms;
    let render_timeout_secs = config.timeout_seconds;

    // Emit a QueueUpdate so the CLI knows the total for phase 2
    if let Some(tx) = &event_tx {
        let _ = tx.send(CrawlEvent::QueueUpdate {
            crawled: 0,
            queued: total_urls as usize,
            estimated_total: total_urls as usize,
        });
    }

    let browser = client.browser(); // Arc<Browser>
    let local_pages = Arc::new(local_pages);
    let mut handles = Vec::new();

    for url in urls {
        let semaphore = semaphore.clone();
        let pages = pages.clone();
        let rendered_count = rendered_count.clone();
        let errored_count = errored_count.clone();
        let event_tx = event_tx.clone();
        let local_pages = local_pages.clone();

        // Arc::clone is cheap — this is how we share the non-Clone Browser
        // across spawned tasks.
        let browser = Arc::clone(&browser);

        let handle = tokio::spawn(async move {
            let _permit = semaphore.acquire().await.unwrap();

            match render_page(&browser, &url, render_wait_ms, render_timeout_secs).await {
                Ok((html, console_errors)) => {
                    let mut page = crate::crawler::parser::parse_html(&html, &url);
                    page.status = 200;
                    page.content_type = Some("text/html".to_string());
                    crate::crawler::compute_url_metadata(&mut page);
                    page.is_https = url.starts_with("https://");
                    page.console_errors = console_errors;

                    // Preserve metadata from the local crawl + detect JS-injected SEO tags
                    if let Some(local) = local_pages.get(&url) {
                        page.response_time_ms = local.response_time_ms;
                        page.depth = local.depth;
                        page.security_headers = local.security_headers.clone();
                        page.x_robots_tag = local.x_robots_tag.clone();
                        page.redirect_chain = local.redirect_chain.clone();
                        crate::crawler::apply_js_injection_flags(&mut page, local);
                    }

                    rendered_count.fetch_add(1, Ordering::Relaxed);

                    if let Some(tx) = &event_tx {
                        let _ = tx.send(CrawlEvent::PageCrawled {
                            url: page.url.clone(),
                            status: page.status,
                            response_time_ms: page.response_time_ms,
                        });
                    }

                    pages.lock().await.push(page);
                }
                Err(e) => {
                    warn!(
                        "Chrome render failed for {}: {} — using local fallback",
                        url, e
                    );
                    errored_count.fetch_add(1, Ordering::Relaxed);

                    // Graceful degradation: use the locally-crawled page
                    if let Some(local) = local_pages.get(&url) {
                        pages.lock().await.push(local.clone());

                        if let Some(tx) = &event_tx {
                            let _ = tx.send(CrawlEvent::PageCrawled {
                                url: url.clone(),
                                status: local.status,
                                response_time_ms: local.response_time_ms,
                            });
                        }
                    } else if let Some(tx) = &event_tx {
                        let _ = tx.send(CrawlEvent::PageError {
                            url,
                            error: e.to_string(),
                        });
                    }
                }
            }
        });
        handles.push(handle);
    }

    // Wait for all rendering tasks to complete
    for handle in handles {
        let _ = handle.await;
    }

    let final_pages = Arc::try_unwrap(pages)
        .expect("all tasks finished")
        .into_inner();
    let pages_crawled = rendered_count.load(Ordering::Relaxed);
    let pages_errored = errored_count.load(Ordering::Relaxed);

    info!(
        "Hybrid Playwright crawl complete: {} rendered, {} fell back to local",
        pages_crawled, pages_errored
    );

    Ok((final_pages, pages_crawled, pages_errored, 0))
}
