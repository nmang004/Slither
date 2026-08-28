use anyhow::Result;

use crate::analysis::{AnalysisContext, AnalyzerRegistry};
use crate::models::config::CrawlConfig;
use crate::models::crawl_result::{CrawlIssues, CrawlMetadata, CrawlResult, ExportSettings};
use crate::models::page::PageData;

/// Input to the post-crawl analysis/scoring pipeline.
pub struct PipelineInput {
    pub pages: Vec<PageData>,
    pub config: CrawlConfig,
    pub duration_ms: u64,
    pub pages_discovered: u32,
    pub pages_crawled: u32,
    pub pages_skipped_robots: u32,
    pub pages_errored: u32,
    pub backend: String,
    pub sitemap_data: Option<crate::crawler::sitemaps::SitemapData>,
    /// Raw robots.txt content for AI-crawler policy analysis.
    pub robots_txt: Option<String>,
}

/// Enrich pages with PageSpeed data if --pagespeed is enabled.
pub async fn enrich_pagespeed(pages: &mut [PageData], config: &CrawlConfig) {
    if !config.pagespeed {
        return;
    }

    let api_key = config
        .pagespeed_key
        .clone()
        .or_else(|| std::env::var("PAGESPEED_API_KEY").ok())
        .filter(|s| !s.is_empty());

    let page_count = pages
        .iter()
        .filter(|p| p.status >= 200 && p.status < 400)
        .count();
    if page_count > 50 && config.pagespeed_sample.is_none() && api_key.is_none() {
        eprintln!(
            "  \u{26a0} {} pages to analyze without an API key \u{2014} consider --pagespeed-sample or --pagespeed-key",
            page_count,
        );
    }

    let indices: Vec<usize> = select_sample_indices(pages, config.pagespeed_sample);

    if indices.is_empty() {
        return;
    }

    let client = std::sync::Arc::new(crate::pagespeed::PageSpeedClient::new(
        api_key,
        config.pagespeed_strategy.clone(),
    ));

    tracing::info!("Fetching PageSpeed data for {} pages", indices.len());

    // Fetch with bounded concurrency. The client's shared rate limiter still
    // gates total request rate, so this only shortens wall-clock time (a
    // sequential run took minutes-to-hours for large crawls). Abort early after
    // repeated quota errors instead of hammering the API for every page.
    const CONCURRENCY: usize = 8;
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(CONCURRENCY));
    let quota_errors = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    let mut handles = Vec::with_capacity(indices.len());
    for &idx in &indices {
        let url = pages[idx].url.clone();
        let client = std::sync::Arc::clone(&client);
        let semaphore = std::sync::Arc::clone(&semaphore);
        let quota_errors = std::sync::Arc::clone(&quota_errors);
        handles.push(tokio::spawn(async move {
            let _permit = semaphore.acquire().await.ok()?;
            // Stop issuing requests once quota looks exhausted.
            if quota_errors.load(std::sync::atomic::Ordering::Relaxed) >= 3 {
                return None;
            }
            match crate::pagespeed::api::fetch_pagespeed(&client, &url).await {
                Ok(result) => Some((idx, result)),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("429") || msg.to_lowercase().contains("quota") {
                        quota_errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    tracing::warn!("PageSpeed failed for {}: {}", url, e);
                    None
                }
            }
        }));
    }

    for handle in handles {
        if let Ok(Some((idx, result))) = handle.await {
            let status = result.overall_cwv_status();
            pages[idx].lcp_ms = Some(result.lcp_ms);
            pages[idx].inp_ms = result.inp_ms;
            pages[idx].cls = Some(result.cls);
            pages[idx].fcp_ms = Some(result.fcp_ms);
            pages[idx].ttfb_ms = Some(result.ttfb_ms);
            pages[idx].performance_score = Some(result.performance_score);
            pages[idx].cwv_status = Some(status.to_string());
        }
    }
}

fn select_sample_indices(pages: &[PageData], sample: Option<u32>) -> Vec<usize> {
    let valid: Vec<usize> = pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.status >= 200 && p.status < 400)
        .map(|(i, _)| i)
        .collect();

    match sample {
        Some(n) => {
            use rand::seq::SliceRandom;
            let mut rng = rand::thread_rng();
            let mut selected = valid;
            selected.shuffle(&mut rng);
            selected.truncate(n as usize);
            selected
        }
        None => valid,
    }
}

/// Run the post-crawl analysis/scoring pipeline and build a `CrawlResult`.
pub fn run_post_crawl_pipeline(input: PipelineInput) -> Result<CrawlResult> {
    let domain = url::Url::parse(&input.config.seed_url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default();

    // Run analyzers
    let registry = AnalyzerRegistry::default_registry();
    let analysis_ctx = AnalysisContext {
        seed_url: input.config.seed_url.clone(),
        domain: domain.clone(),
        sitemap_data: input.sitemap_data.clone(),
        pages: input.pages.clone(),
        robots_txt: input.robots_txt.clone(),
    };
    let issues_list = registry.run_all(&analysis_ctx);
    let issues = CrawlIssues {
        issues: issues_list,
    };

    // Build summary and scoring
    let mut summary = crate::crawler::build_summary(&input.pages);
    let grade = crate::analysis::scoring::compute_health_score(&issues.issues, summary.total_pages);
    summary.health_score = grade.score;
    summary.grade = grade.letter;
    summary.grade_verdict = grade.verdict;

    let (critical, warning, info_count) =
        crate::analysis::scoring::count_urls_by_severity(&issues.issues);
    summary.critical_issues = critical;
    summary.warning_issues = warning;
    summary.info_issues = info_count;
    summary.total_issues = critical + warning + info_count;
    summary.issues_by_category = crate::analysis::scoring::build_category_summaries(&issues.issues);

    let mut times: Vec<u64> = input.pages.iter().map(|p| p.response_time_ms).collect();
    let (p50, p95) = crate::analysis::scoring::compute_percentiles(&mut times);
    summary.response_time_p50_ms = p50;
    summary.response_time_p95_ms = p95;

    Ok(CrawlResult {
        slither_version: env!("CARGO_PKG_VERSION").to_string(),
        crawl_metadata: CrawlMetadata {
            domain,
            seed_url: input.config.seed_url.clone(),
            crawl_date: chrono::Utc::now().to_rfc3339(),
            duration_ms: input.duration_ms,
            pages_discovered: input.pages_discovered,
            pages_crawled: input.pages_crawled,
            pages_skipped_robots: input.pages_skipped_robots,
            pages_errored: input.pages_errored,
            settings: input.config.clone(),
            backend: input.backend,
        },
        export_settings: ExportSettings::default(),
        pages: input.pages,
        issues,
        summary,
        robots_txt: input.robots_txt,
        sitemap_data: input.sitemap_data,
    })
}
