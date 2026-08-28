pub mod fetcher;
/// Follow-up HEAD requests that fill in security headers. Nothing here depends
/// on a rendering backend; it lived under `cloudflare` only by accident, which
/// meant a build without that feature could not fetch security headers at all.
pub mod head_requests;
pub mod parser;
pub mod queue;
pub mod robots;
pub mod sitemaps;
pub mod url_utils;

use crate::models::config::CrawlConfig;
use crate::models::crawl_result::{
    CrawlIssues, CrawlMetadata, CrawlResult, CrawlSummary, ExportSettings,
};
use crate::models::page::{PageData, SecurityHeaders};
use crate::models::CrawlEvent;
use anyhow::{Context, Result};
use dashmap::DashMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tracing::{debug, info, warn};
use url::Url;

use self::fetcher::{Fetcher, RequestGate};
use self::queue::CrawlQueue;
use self::robots::RobotsChecker;
use self::url_utils::{is_crawlable_url, is_same_domain, normalize_url};

pub async fn crawl(config: CrawlConfig, event_tx: mpsc::Sender<CrawlEvent>) -> Result<CrawlResult> {
    // A config can arrive straight from a REST/MCP request. A zero concurrency
    // would build a zero-permit semaphore whose first acquire never resolves,
    // hanging the crawl forever.
    let config = config.sanitized();
    let start_time = std::time::Instant::now();

    // Parse and normalize the seed URL
    let requested_seed = normalize_url(&config.seed_url)
        .with_context(|| format!("Invalid seed URL: {}", config.seed_url))?;

    // Resolve the seed's own redirect before anything else, and treat where it
    // lands as the site being audited.
    //
    // Almost every site canonicalises one way or the other — www to apex, apex
    // to www, http to https. Pages are recorded at the URL that served them, so
    // without this the scope filter still compared discovered links against the
    // URL the user typed: seeding `https://www.example.com/` on a site that
    // redirects to the apex classified every link on the landing page as
    // external, and the crawl ended after one page with a passing grade and no
    // warning.
    //
    // Two values come out of this. `seed_url` is the normalized form, used for
    // scope and as the dedup key; `seed_request` is the URL to actually
    // request. They differ because `normalize_url` drops a trailing slash, so
    // asking for `/home` where the site serves `/home/` earns another redirect
    // on every CMS that canonicalises the other way — and records the homepage
    // under a URL the site itself would redirect. The crawl-wide convention is:
    // request as-linked, dedupe on normalized.
    let (seed_url, seed_request) = {
        let probe = Fetcher::new(&config.user_agent, config.timeout_seconds);
        match probe.fetch_with_redirects(&requested_seed, 5).await {
            Ok((result, chain)) if !chain.is_empty() => {
                let landed_raw = result.url.clone();
                let landed = normalize_url(&landed_raw).unwrap_or_else(|_| landed_raw.clone());
                if landed != requested_seed {
                    info!("Seed {requested_seed} redirects to {landed}; auditing that origin");
                }
                (landed, landed_raw)
            }
            // No redirect, or the probe failed — the crawl loop will fetch the
            // seed properly and surface any error there.
            _ => (requested_seed.clone(), config.seed_url.clone()),
        }
    };

    let seed_parsed = Url::parse(&seed_url)?;
    let domain = seed_parsed
        .host_str()
        .context("Seed URL has no host")?
        .to_string();

    // Send Started event
    let _ = event_tx
        .send(CrawlEvent::Started {
            domain: domain.clone(),
            settings: config.clone(),
        })
        .await;

    // Fetch robots.txt
    let robots = if !config.ignore_robots {
        match RobotsChecker::fetch(&config.seed_url, &config.user_agent, config.timeout_seconds)
            .await
        {
            Ok(checker) => {
                let _ = event_tx
                    .send(CrawlEvent::RobotsFetched {
                        rules_count: checker.rules_count(),
                        crawl_delay: checker.crawl_delay(),
                    })
                    .await;
                Some(Arc::new(checker))
            }
            Err(e) => {
                warn!("Failed to fetch robots.txt: {}", e);
                let _ = event_tx
                    .send(CrawlEvent::RobotsFetched {
                        rules_count: 0,
                        crawl_delay: None,
                    })
                    .await;
                None
            }
        }
    } else {
        let _ = event_tx
            .send(CrawlEvent::RobotsFetched {
                rules_count: 0,
                crawl_delay: None,
            })
            .await;
        None
    };

    // Determine effective delay (respect robots.txt crawl-delay if higher)
    let effective_delay_ms = if let Some(ref r) = robots {
        if let Some(crawl_delay) = r.crawl_delay() {
            std::cmp::max(config.delay_ms, crawl_delay * 1000)
        } else {
            config.delay_ms
        }
    } else {
        config.delay_ms
    };

    // Shared state
    let visited: Arc<DashMap<String, ()>> = Arc::new(DashMap::new());
    let pages: Arc<tokio::sync::Mutex<Vec<PageData>>> =
        Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let crawl_queue = Arc::new(CrawlQueue::new());
    let semaphore = Arc::new(Semaphore::new(config.concurrency as usize));
    let pages_crawled = Arc::new(AtomicU32::new(0));
    let pages_errored = Arc::new(AtomicU32::new(0));
    let pages_skipped_robots = Arc::new(AtomicU32::new(0));

    let fetcher = Arc::new(Fetcher::new(&config.user_agent, config.timeout_seconds));
    // One shared gate for the whole crawl. The delay used to be a sleep inside
    // each spawned task, so `concurrency` workers slept in parallel and then
    // fired together — under `Crawl-delay: 2` with 3 workers the host saw three
    // simultaneous requests every 2 s, three times the permitted rate and
    // exactly the burst pattern the directive exists to prevent.
    let request_gate = Arc::new(RequestGate::new(effective_delay_ms));

    // Enqueue the seed where its redirect landed, keyed on the normalized form.
    // Queueing the URL the user typed instead made the landing page dedupe
    // against a key seeded from its own destination, so the homepage was
    // dropped and everything it links to was reported as an orphan.
    visited.insert(seed_url.clone(), ());
    crawl_queue
        .push(without_fragment(&seed_request).to_string(), 0)
        .await;

    // Track active tasks
    let active_tasks = Arc::new(AtomicU32::new(0));

    // Number of fetch tasks spawned so far. This loop is the sole spawner, so a
    // plain counter is an exact cap: gating on `pages_crawled` (bumped only
    // *after* a fetch finishes) let up to `concurrency` extra tasks slip through
    // while fetches were still in flight, overshooting max_pages.
    let mut pages_spawned: u32 = 0;

    loop {
        // Check if we've hit max pages
        if pages_spawned >= config.max_pages {
            debug!("Max pages ({}) reached, stopping", config.max_pages);
            break;
        }

        // Try to get next URL from queue
        if let Some(entry) = crawl_queue.pop().await {
            // Check depth limit
            if entry.depth > config.max_depth {
                debug!(
                    "Skipping {} — exceeds max depth {}",
                    entry.url, config.max_depth
                );
                continue;
            }

            // Check robots.txt (path + query, so rules like `Disallow: /*?`
            // and `Disallow: /search?` can match).
            if let Some(ref robots) = robots {
                let path = Url::parse(&entry.url)
                    .map(|u| match u.query() {
                        Some(q) => format!("{}?{}", u.path(), q),
                        None => u.path().to_string(),
                    })
                    .unwrap_or_default();
                if !robots.is_allowed(&path) {
                    debug!("Skipping {} — disallowed by robots.txt", entry.url);
                    pages_skipped_robots.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            }

            // Check max pages again before spawning
            if pages_spawned >= config.max_pages {
                break;
            }

            // Acquire semaphore permit
            let permit = semaphore.clone().acquire_owned().await?;
            active_tasks.fetch_add(1, Ordering::Relaxed);

            // Clone all shared state for the task
            let fetcher = fetcher.clone();
            let visited = visited.clone();
            let pages = pages.clone();
            let crawl_queue = crawl_queue.clone();
            let event_tx = event_tx.clone();
            let pages_crawled = pages_crawled.clone();
            let pages_errored = pages_errored.clone();
            let seed_url = seed_url.clone();
            let active_tasks = active_tasks.clone();
            let max_pages = config.max_pages;
            let max_depth = config.max_depth;
            let follow_subdomains = config.follow_subdomains;
            let request_gate = request_gate.clone();
            let task_robots = robots.clone();
            let entry_url = entry.url.clone();
            let entry_depth = entry.depth;

            tokio::spawn(async move {
                // Robots applies to every request, including the ones made while
                // following a redirect. Checking only the queued URL let a
                // redirect carry the crawler into a Disallowed path, whose
                // content was then filed under the allowed URL.
                let robots_for_hops = task_robots.clone();
                let allow_hop = move |candidate: &str| -> bool {
                    let Some(ref checker) = robots_for_hops else {
                        return true;
                    };
                    match Url::parse(candidate) {
                        Ok(u) => {
                            let path = match u.query() {
                                Some(q) => format!("{}?{}", u.path(), q),
                                None => u.path().to_string(),
                            };
                            checker.is_allowed(&path)
                        }
                        Err(_) => true,
                    }
                };

                // Fetch the page with redirect following
                match fetcher
                    .fetch_with_redirects_gated(&entry_url, 5, &request_gate, allow_hop)
                    .await
                {
                    Ok((fetch_result, redirect_chain)) => {
                        let status = fetch_result.status;
                        let response_time_ms = fetch_result.response_time_ms;
                        // The URL that actually served this response. Relative
                        // links resolve against it (RFC 3986 §5.1.3), and it is
                        // the page's real identity: filing the record under the
                        // pre-redirect URL made a site that 301s `/` to `/en/`
                        // resolve every relative link one directory too high,
                        // fetch URLs that do not exist, and report the
                        // resulting 404s as the site's own broken links.
                        let final_url = fetch_result.url.clone();

                        let _ = event_tx
                            .send(CrawlEvent::PageCrawled {
                                url: entry_url.clone(),
                                status,
                                response_time_ms,
                            })
                            .await;

                        // Only parse HTML content
                        // Case-insensitive per RFC 9110, and sniff the body when
                        // the server sends no Content-Type at all — otherwise
                        // those responses were silently never parsed, ending the
                        // crawl at one page with nothing reported wrong.
                        let is_html = match fetch_result.content_type.as_deref() {
                            Some(ct) => crate::models::page::is_html_content_type(Some(ct)),
                            None => crate::models::page::body_looks_like_html(&fetch_result.body),
                        };

                        let mut page_data = if is_html && (200..400).contains(&status) {
                            let mut page = parser::parse_html(&fetch_result.body, &final_url);
                            page.status = status;
                            page.response_time_ms = response_time_ms;
                            page.content_type = fetch_result.content_type.clone();
                            page.depth = entry_depth;
                            page.redirect_chain = if redirect_chain.is_empty() {
                                None
                            } else {
                                Some(redirect_chain)
                            };
                            page
                        } else {
                            // Non-HTML or error page — minimal data
                            PageData {
                                url: final_url.clone(),
                                status,
                                redirect_chain: if redirect_chain.is_empty() {
                                    None
                                } else {
                                    Some(redirect_chain)
                                },
                                response_time_ms,
                                content_type: fetch_result.content_type.clone(),
                                depth: entry_depth,
                                title: None,
                                meta_description: None,
                                meta_robots: None,
                                canonical: None,
                                h1: Vec::new(),
                                headings: Vec::new(),
                                word_count: 0,
                                body_text: String::new(),
                                internal_links: Vec::new(),
                                external_links: Vec::new(),
                                images: Vec::new(),
                                schema_types: Vec::new(),
                                og_tags: HashMap::new(),
                                content_hash: String::new(),
                                is_https: final_url.starts_with("https://"),
                                security_headers: SecurityHeaders::default(),
                                mixed_content: Vec::new(),
                                insecure_forms: Vec::new(),
                                url_length: 0,
                                has_parameters: false,
                                has_underscores: false,
                                has_uppercase: false,
                                has_non_ascii: false,
                                has_multiple_slashes: false,
                                has_repetitive_path: false,
                                title_length: None,
                                title_pixel_width: None,
                                meta_description_length: None,
                                meta_description_pixel_width: None,
                                title_count: 0,
                                meta_description_count: 0,
                                title_in_head: false,
                                meta_desc_in_head: false,
                                canonical_is_relative: false,
                                canonical_count: 0,
                                canonical_source: None,
                                has_self_canonical: false,
                                x_robots_tag: None,
                                meta_robots_directives: Vec::new(),
                                hreflang_tags: Vec::new(),
                                pagination: None,
                                readability_score: None,
                                is_soft_404: false,
                                structured_data: Vec::new(),
                                unsafe_cross_origin_links: Vec::new(),
                                lcp_ms: None,
                                inp_ms: None,
                                cls: None,
                                fcp_ms: None,
                                ttfb_ms: None,
                                performance_score: None,
                                cwv_status: None,
                                js_injected_title: false,
                                js_injected_description: false,
                                js_injected_canonical: false,
                                js_injected_h1: false,
                                js_injected_structured_data: false,
                                console_errors: Vec::new(),
                                scripts: Vec::new(),
                            }
                        };

                        // Compute URL metadata and security headers for all pages
                        compute_url_metadata(&mut page_data);
                        page_data.security_headers =
                            extract_security_headers(&fetch_result.headers);
                        page_data.x_robots_tag =
                            extract_header_value(&fetch_result.headers, "x-robots-tag");
                        apply_link_header_canonical(&mut page_data, &fetch_result.headers);

                        // Check meta robots for nofollow.
                        //
                        // Match the directive token, not a substring of the raw
                        // value, and skip surface-scoped tokens: a
                        // `<meta name="googlebot-news" content="nofollow">`
                        // binds Google News alone, so treating it as a site-wide
                        // nofollow would end link discovery on a page general
                        // Search crawls normally. Same rule
                        // `PageData::is_noindex` applies.
                        let should_follow = !page_data
                            .meta_robots_directives
                            .iter()
                            .filter(|d| !parser::is_surface_scoped_directive(d))
                            .any(|d| d.split_whitespace().any(|t| t == "nofollow" || t == "none"));

                        // Enqueue discovered internal links
                        if should_follow
                            && is_html
                            && entry_depth < max_depth
                            && pages_crawled.load(Ordering::Relaxed) < max_pages
                        {
                            for link in &page_data.internal_links {
                                if !is_crawlable_url(&link.url) {
                                    continue;
                                }

                                // Check same domain (or subdomain if configured)
                                let is_valid = if follow_subdomains {
                                    // For subdomain following, check base domain
                                    is_same_domain(&link.url, &seed_url).unwrap_or(false)
                                        || is_subdomain_of(&link.url, &seed_url)
                                } else {
                                    is_same_domain(&link.url, &seed_url).unwrap_or(false)
                                };

                                if !is_valid {
                                    continue;
                                }

                                if let Ok(normalized) = normalize_url(&link.url) {
                                    // Claim the URL with a single atomic insert. A
                                    // contains_key/insert pair lets two concurrent
                                    // tasks both observe "unvisited" and enqueue the
                                    // same URL, double-counting it everywhere.
                                    if visited.insert(normalized, ()).is_none() {
                                        // Enqueue the URL as it was actually linked,
                                        // not the normalized key. Normalization drops
                                        // the trailing slash, and a site that serves
                                        // `/x/` canonically answers `/x` with a 301 —
                                        // so requesting the normalized form made every
                                        // page look like a redirect and recorded every
                                        // URL in its non-canonical form. Only the
                                        // fragment comes off; see [`without_fragment`].
                                        crawl_queue
                                            .push(
                                                without_fragment(&link.url).to_string(),
                                                entry_depth + 1,
                                            )
                                            .await;
                                    }
                                }
                            }
                        }

                        pages_crawled.fetch_add(1, Ordering::Relaxed);

                        let crawled = pages_crawled.load(Ordering::Relaxed) as usize;
                        let queued = crawl_queue.len().await;
                        let _ = event_tx
                            .send(CrawlEvent::QueueUpdate {
                                crawled,
                                queued,
                                estimated_total: crawled + queued,
                            })
                            .await;

                        // Claim the destination too. Without this, two URLs
                        // redirecting to the same target both store it, and the
                        // target is then reported as duplicate content with
                        // itself.
                        // Compare normalized keys on both sides: the requested
                        // URL is not necessarily in normalized form, and
                        // mismatching the two dropped every page whose seed
                        // lacked a trailing slash.
                        let entry_key =
                            normalize_url(&entry_url).unwrap_or_else(|_| entry_url.clone());
                        let store = match normalize_url(&page_data.url) {
                            Ok(key) if key != entry_key => visited.insert(key, ()).is_none(),
                            _ => true,
                        };
                        if store {
                            pages.lock().await.push(page_data);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to fetch {}: {}", entry_url, e);
                        pages_errored.fetch_add(1, Ordering::Relaxed);
                        let _ = event_tx
                            .send(CrawlEvent::PageError {
                                url: entry_url,
                                error: e.to_string(),
                            })
                            .await;
                    }
                }

                active_tasks.fetch_sub(1, Ordering::Relaxed);
                drop(permit);
            });
            pages_spawned += 1;
        } else {
            // Queue is empty — check if any tasks are still running.
            // Use SeqCst and double-check after a brief sleep to avoid TOCTOU race.
            if active_tasks.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                if crawl_queue.is_empty().await && active_tasks.load(Ordering::SeqCst) == 0 {
                    debug!("Queue empty and no active tasks, crawl complete");
                    break;
                }
                continue;
            }
            // Wait a bit for tasks to enqueue new URLs
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    // Wait for all remaining tasks to complete
    // Acquire all semaphore permits to ensure all tasks are done
    let _all_permits = semaphore.acquire_many(config.concurrency).await?;

    let duration_ms = start_time.elapsed().as_millis() as u64;
    let pages_vec = pages.lock().await.clone();
    let pages_discovered = visited.len() as u32;
    let crawled_count = pages_crawled.load(Ordering::Relaxed);
    let errored_count = pages_errored.load(Ordering::Relaxed);
    let skipped_robots_count = pages_skipped_robots.load(Ordering::Relaxed);

    // Fetch sitemaps
    let robots_sitemaps = robots
        .as_ref()
        .map(|r| r.sitemap_urls().to_vec())
        .unwrap_or_default();
    let sitemap_data = {
        let sitemap_fetcher = crate::crawler::sitemaps::SitemapFetcher::new(&fetcher);
        // Pass the seed URL, not the bare host: discovery derives the origin
        // from it, and a host alone loses the scheme and port — a site served on
        // a non-default port reported "no sitemap found" with zero sources and
        // zero errors, because discovery probed the wrong origin.
        sitemap_fetcher.fetch_all(&seed_url, &robots_sitemaps).await
    };

    // Build summary
    let mut summary = build_summary(&pages_vec);

    // Run all analyzers
    let registry = crate::analysis::AnalyzerRegistry::default_registry();
    let robots_txt = robots.as_ref().map(|r| r.raw().to_string());
    let analysis_ctx = crate::analysis::AnalysisContext {
        seed_url: seed_url.clone(),
        domain: domain.clone(),
        sitemap_data: Some(sitemap_data.clone()),
        pages: pages_vec.clone(),
        robots_txt: robots_txt.clone(),
    };
    let issues_list = registry.run_all(&analysis_ctx);
    let issues = CrawlIssues {
        issues: issues_list,
    };

    // Compute health score
    let grade = crate::analysis::scoring::compute_health_score(&issues.issues, summary.total_pages);
    summary.health_score = grade.score;
    summary.grade = grade.letter;
    summary.grade_verdict = grade.verdict;

    // Compute severity counts
    let (critical, warning, info) =
        crate::analysis::scoring::count_urls_by_severity(&issues.issues);
    summary.critical_issues = critical;
    summary.warning_issues = warning;
    summary.info_issues = info;
    summary.total_issues = critical + warning + info;

    // Build category summaries
    summary.issues_by_category = crate::analysis::scoring::build_category_summaries(&issues.issues);

    // Compute percentiles
    let mut times: Vec<u64> = pages_vec.iter().map(|p| p.response_time_ms).collect();
    let (p50, p95) = crate::analysis::scoring::compute_percentiles(&mut times);
    summary.response_time_p50_ms = p50;
    summary.response_time_p95_ms = p95;

    let result = CrawlResult {
        slither_version: env!("CARGO_PKG_VERSION").to_string(),
        crawl_metadata: CrawlMetadata {
            domain: domain.clone(),
            seed_url: seed_url.clone(),
            crawl_date: chrono::Utc::now().to_rfc3339(),
            duration_ms,
            pages_discovered,
            pages_crawled: crawled_count,
            pages_skipped_robots: skipped_robots_count,
            pages_errored: errored_count,
            settings: config,
            backend: "local".to_string(),
        },
        export_settings: ExportSettings::default(),
        pages: pages_vec,
        issues,
        summary,
        robots_txt,
        sitemap_data: Some(sitemap_data),
    };

    let _ = event_tx
        .send(CrawlEvent::Completed {
            result: Box::new(result.clone()),
        })
        .await;

    info!(
        "Crawl complete: {} pages in {}ms",
        crawled_count, duration_ms
    );

    Ok(result)
}

/// A URL with its fragment removed.
///
/// RFC 3986 §3.5: the fragment is everything after the first `#`, it is never
/// sent to the server, and `/x#toc` is the same resource as `/x`. The queued URL
/// becomes the page's identity — it is what gets recorded, what the URL checks
/// measure, and what lands in the generated sitemap's `<loc>` — so queueing the
/// linked form verbatim made a client-side anchor part of the page. On a real
/// sqlite.org crawl 3 of 25 `<loc>` entries carried `#toc`, `#aggfunclist` and
/// `#biwinfunc`, and the URL checks then reported "URL contains spaces" for a
/// `%20` that existed only in the fragment, 117 characters for a 31-character
/// URL, and "has parameters" for a `?` inside one.
///
/// Deliberately a string split rather than a `Url` round-trip: the trailing
/// slash is load-bearing (a site that serves `/x/` answers `/x` with a 301), and
/// splitting at the delimiter leaves every other byte exactly as the page linked
/// it. A literal `#` cannot appear elsewhere in a URL — the `url` crate has
/// already percent-encoded any in the path or query — so the first one is always
/// the fragment delimiter.
fn without_fragment(url: &str) -> &str {
    match url.split_once('#') {
        Some((head, _)) => head,
        None => url,
    }
}

pub fn build_summary(pages: &[PageData]) -> CrawlSummary {
    let total_pages = pages.len() as u32;

    let mut by_status: HashMap<String, u32> = HashMap::new();
    let mut total_response_time: u64 = 0;
    let mut total_word_count: u64 = 0;
    let mut total_internal_links: u32 = 0;
    let mut total_external_links: u32 = 0;
    let mut total_images: u32 = 0;
    let mut images_without_alt: u32 = 0;
    let mut pages_with_schema: u32 = 0;

    for page in pages {
        *by_status.entry(page.status.to_string()).or_insert(0) += 1;
        total_response_time += page.response_time_ms;
        total_word_count += page.word_count as u64;
        total_internal_links += page.internal_links.len() as u32;
        total_external_links += page.external_links.len() as u32;
        total_images += page.images.len() as u32;
        images_without_alt += page
            .images
            .iter()
            .filter(|img| img.alt.is_none() && img.needs_alt_text())
            .count() as u32;
        if !page.schema_types.is_empty() {
            pages_with_schema += 1;
        }
    }

    let avg_response_time_ms = if total_pages > 0 {
        (total_response_time / total_pages as u64) as u32
    } else {
        0
    };

    let avg_word_count = if total_pages > 0 {
        (total_word_count / total_pages as u64) as u32
    } else {
        0
    };

    // CWV aggregation
    let cwv_pages: Vec<&PageData> = pages
        .iter()
        .filter(|p| p.performance_score.is_some())
        .collect();
    let cwv_pages_tested = cwv_pages.len() as u32;
    let cwv_pages_good = cwv_pages
        .iter()
        .filter(|p| p.cwv_status.as_deref() == Some("good"))
        .count() as u32;
    let cwv_pages_needs_work = cwv_pages
        .iter()
        .filter(|p| p.cwv_status.as_deref() == Some("needs_improvement"))
        .count() as u32;
    let cwv_pages_poor = cwv_pages
        .iter()
        .filter(|p| p.cwv_status.as_deref() == Some("poor"))
        .count() as u32;

    let lcp_values: Vec<f64> = cwv_pages.iter().filter_map(|p| p.lcp_ms).collect();
    let avg_lcp_ms = if !lcp_values.is_empty() {
        Some(lcp_values.iter().sum::<f64>() / lcp_values.len() as f64)
    } else {
        None
    };

    let inp_values: Vec<f64> = cwv_pages.iter().filter_map(|p| p.inp_ms).collect();
    let avg_inp_ms = if !inp_values.is_empty() {
        Some(inp_values.iter().sum::<f64>() / inp_values.len() as f64)
    } else {
        None
    };

    let cls_values: Vec<f64> = cwv_pages.iter().filter_map(|p| p.cls).collect();
    let avg_cls = if !cls_values.is_empty() {
        Some(cls_values.iter().sum::<f64>() / cls_values.len() as f64)
    } else {
        None
    };

    let score_values: Vec<u32> = cwv_pages
        .iter()
        .filter_map(|p| p.performance_score)
        .collect();
    let avg_performance_score = if !score_values.is_empty() {
        Some((score_values.iter().sum::<u32>() as f64 / score_values.len() as f64) as u32)
    } else {
        None
    };

    CrawlSummary {
        total_pages,
        by_status,
        avg_response_time_ms,
        avg_word_count,
        total_internal_links,
        total_external_links,
        total_images,
        images_without_alt,
        pages_with_schema,
        total_issues: 0,
        critical_issues: 0,
        warning_issues: 0,
        info_issues: 0,
        issues_by_category: HashMap::new(),
        health_score: 0,
        grade: String::new(),
        grade_verdict: String::new(),
        response_time_p50_ms: 0,
        response_time_p95_ms: 0,
        cwv_pages_tested,
        cwv_pages_good,
        cwv_pages_needs_work,
        cwv_pages_poor,
        avg_lcp_ms,
        avg_inp_ms,
        avg_cls,
        avg_performance_score,
    }
}

/// Compute URL metadata fields on a PageData.
pub fn compute_url_metadata(page: &mut PageData) {
    page.url_length = page.url.len() as u32;
    page.is_https = page.url.starts_with("https://");
    page.has_parameters = page.url.contains('?');

    if let Ok(parsed) = Url::parse(&page.url) {
        let path = parsed.path();
        // Both of these must look past percent-encoding. `Url` stores the path
        // already encoded, so a non-ASCII path such as `/café` is held as
        // `/caf%C3%A9`: the literal check reported "uppercase characters" on a
        // URL that visibly contains none (the hex digits), while the non-ASCII
        // check could never fire at all, since an encoded path is ASCII by
        // construction.
        page.has_uppercase = path_without_escapes(path)
            .chars()
            .any(|c| c.is_ascii_uppercase());
        page.has_underscores = path.contains('_');
        page.has_non_ascii = !path.is_ascii() || has_non_ascii_escape(path);
        page.has_multiple_slashes = path.contains("//");
        page.has_repetitive_path = has_duplicate_segments(path);
    }
}

/// The path with every `%XX` triplet removed, so checks about the characters a
/// user actually sees are not confused by encoding hex digits.
fn path_without_escapes(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = String::with_capacity(path.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            i += 3;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// True if the path percent-encodes any byte outside ASCII, which is what a
/// non-ASCII URL looks like once the `url` crate has encoded it.
fn has_non_ascii_escape(path: &str) -> bool {
    let bytes = path.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'%' {
            if let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                if (hi * 16 + lo) >= 0x80 {
                    return true;
                }
                i += 3;
                continue;
            }
        }
        i += 1;
    }
    false
}

/// Check if a URL path has duplicate segments.
fn has_duplicate_segments(path: &str) -> bool {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let unique: HashSet<&str> = segments.iter().copied().collect();
    segments.len() != unique.len()
}

/// Extract security-related HTTP headers into a SecurityHeaders struct.
pub fn extract_security_headers(headers: &[(String, String)]) -> SecurityHeaders {
    let mut sh = SecurityHeaders::default();

    for (name, value) in headers {
        let lower = name.to_lowercase();
        match lower.as_str() {
            "strict-transport-security" => sh.has_hsts = true,
            "content-security-policy" => sh.has_csp = true,
            "x-content-type-options" => sh.has_x_content_type_options = true,
            "x-frame-options" => sh.has_x_frame_options = true,
            "referrer-policy" => {
                sh.has_referrer_policy = true;
                sh.referrer_policy_value = Some(value.clone());
            }
            _ => {}
        }
    }

    sh
}

/// Extract a specific header value by name (case-insensitive).
pub fn extract_header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    let lower_name = name.to_lowercase();
    headers
        .iter()
        .find(|(k, _)| k.to_lowercase() == lower_name)
        .map(|(_, v)| v.clone())
}

/// Parse the `Link: <url>; rel="canonical"` HTTP header and fold it into the
/// page's canonical. Sets `canonical_source` to `HttpHeader` (or `Both` when the
/// HTML already declared one) so a header-canonicalized page is not reported as
/// "missing canonical".
pub fn apply_link_header_canonical(page: &mut PageData, headers: &[(String, String)]) {
    let Some(link_value) = extract_header_value(headers, "link") else {
        return;
    };
    let Some(header_canonical) = parse_link_header_canonical(&link_value) else {
        return;
    };
    // Resolve relative header canonicals against the page URL.
    let resolved = Url::parse(&page.url)
        .ok()
        .and_then(|base| base.join(&header_canonical).ok())
        .map(|u| u.to_string())
        .unwrap_or(header_canonical);

    if page.canonical.is_some() {
        page.canonical_source = Some(crate::models::page::CanonicalSource::Both);
    } else {
        page.canonical = Some(resolved.clone());
        page.canonical_count = page.canonical_count.max(1);
        page.canonical_source = Some(crate::models::page::CanonicalSource::HttpHeader);
        // Recompute self-canonical against the header value.
        if let (Ok(a), Ok(b)) = (
            url_utils::normalize_url(&resolved),
            url_utils::normalize_url(&page.url),
        ) {
            page.has_self_canonical = a == b;
        }
    }
}

/// Extract the canonical URL from a `Link` header value, if any link element
/// carries `rel="canonical"`. Handles multiple comma-separated link elements.
fn parse_link_header_canonical(value: &str) -> Option<String> {
    for element in split_link_header(value) {
        let mut url: Option<String> = None;
        let mut is_canonical = false;
        for (i, part) in element.split(';').enumerate() {
            let part = part.trim();
            if i == 0 {
                url = part
                    .strip_prefix('<')
                    .and_then(|s| s.strip_suffix('>'))
                    .map(|s| s.trim().to_string());
            } else if let Some(rel) = part.strip_prefix("rel=") {
                let rel = rel.trim().trim_matches('"').to_ascii_lowercase();
                if rel.split_whitespace().any(|t| t == "canonical") {
                    is_canonical = true;
                }
            }
        }
        if is_canonical {
            if let Some(u) = url {
                return Some(u);
            }
        }
    }
    None
}

/// Split a `Link` header into individual link elements on commas that are not
/// inside angle-bracketed URIs.
fn split_link_header(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, ch) in value.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth <= 0 => {
                out.push(value[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(value[start..].trim().to_string());
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

/// Detect pages that appear to be JavaScript-rendered (empty content despite 200 status).
/// Returns (js_page_count, total_page_count) if the threshold is met.
pub fn detect_js_rendering(pages: &[PageData]) -> Option<(usize, usize)> {
    let total = pages.len();
    if total == 0 {
        return None;
    }
    // Only HTML documents can be client-rendered. A PDF, image or JSON endpoint
    // linked with an ordinary <a href> is fetched but deliberately not parsed,
    // so it has no words — which was being read as evidence of JavaScript
    // rendering, advising the user to re-run against a headless-browser backend
    // that would change nothing.
    let html_pages: Vec<&PageData> = pages
        .iter()
        .filter(|p| crate::models::page::is_html_content_type(p.content_type.as_deref()))
        .collect();
    let considered = html_pages.len();
    if considered == 0 {
        return None;
    }
    let js_pages = html_pages
        .iter()
        .filter(|p| p.status == 200 && p.word_count == 0 && p.body_text.is_empty())
        .count();
    if js_pages > 0 && (js_pages * 100 / considered) > 25 {
        Some((js_pages, considered))
    } else {
        None
    }
}

/// Set `js_injected_*` flags on `rendered` by comparing it against the
/// statically-crawled `static_page`. A flag fires when a tag is absent in
/// the static HTML but present after JavaScript rendering.
pub fn apply_js_injection_flags(rendered: &mut PageData, static_page: &PageData) {
    rendered.js_injected_title = static_page.title.is_none() && rendered.title.is_some();
    rendered.js_injected_description =
        static_page.meta_description.is_none() && rendered.meta_description.is_some();
    rendered.js_injected_canonical =
        static_page.canonical.is_none() && rendered.canonical.is_some();
    rendered.js_injected_h1 = static_page.h1.is_empty() && !rendered.h1.is_empty();
    rendered.js_injected_structured_data =
        static_page.schema_types.is_empty() && !rendered.schema_types.is_empty();
}

/// Check if url_a is a subdomain of the domain in url_b.
fn is_subdomain_of(url_a: &str, url_b: &str) -> bool {
    let host_a = Url::parse(url_a)
        .ok()
        .and_then(|u| u.host_str().map(String::from));
    let host_b = Url::parse(url_b)
        .ok()
        .and_then(|u| u.host_str().map(String::from));

    match (host_a, host_b) {
        (Some(a), Some(b)) => {
            a != b && (a.ends_with(&format!(".{}", b)) || b.ends_with(&format!(".{}", a)))
        }
        _ => false,
    }
}

#[cfg(test)]
mod fragment_tests {
    use super::without_fragment;

    #[test]
    fn the_fragment_comes_off_and_nothing_else_does() {
        assert_eq!(
            without_fragment("https://e.com/lang_aggfunc.html#aggfunclist"),
            "https://e.com/lang_aggfunc.html"
        );
        // The trailing slash is load-bearing: a site that serves `/a/` answers
        // `/a` with a 301, so stripping it would make every page a redirect.
        assert_eq!(without_fragment("https://e.com/a/#toc"), "https://e.com/a/");
        assert_eq!(without_fragment("https://e.com/a/"), "https://e.com/a/");
        // A query is sent to the server; a fragment is not.
        assert_eq!(
            without_fragment("https://e.com/s?q=1&r=2#hit"),
            "https://e.com/s?q=1&r=2"
        );
        // Only the first `#` delimits; anything after it is fragment.
        assert_eq!(without_fragment("https://e.com/p#a#b"), "https://e.com/p");
        assert_eq!(without_fragment("https://e.com/p"), "https://e.com/p");
    }
}

#[cfg(test)]
mod link_header_tests {
    use super::parse_link_header_canonical;

    #[test]
    fn parses_canonical_from_link_header() {
        let v = "<https://example.com/canonical>; rel=\"canonical\"";
        assert_eq!(
            parse_link_header_canonical(v).as_deref(),
            Some("https://example.com/canonical")
        );
    }

    #[test]
    fn picks_canonical_among_multiple_links() {
        let v = "<https://example.com/prev>; rel=\"prev\", \
                 <https://example.com/c>; rel=\"canonical\", \
                 <https://example.com/next>; rel=\"next\"";
        assert_eq!(
            parse_link_header_canonical(v).as_deref(),
            Some("https://example.com/c")
        );
    }

    #[test]
    fn none_when_no_canonical_rel() {
        let v = "<https://example.com/style.css>; rel=\"stylesheet\"";
        assert!(parse_link_header_canonical(v).is_none());
    }
}
