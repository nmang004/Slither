use crate::models::page::{PageData, SecurityHeaders};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::warn;

// ---------------------------------------------------------------------------
// Header request helpers — fetch security headers for rendered pages
// ---------------------------------------------------------------------------

/// Merge response headers (from a HEAD/GET fetch) into an existing `PageData`.
///
/// Sets `response_time_ms` and parses security-related headers into
/// `page.security_headers`. Also extracts `x-robots-tag`.
pub fn merge_head_response(
    page: &mut PageData,
    headers: &[(String, String)],
    response_time_ms: u64,
) {
    page.response_time_ms = response_time_ms;

    let mut sh = SecurityHeaders::default();

    for (name, value) in headers {
        match name.to_ascii_lowercase().as_str() {
            "strict-transport-security" => sh.has_hsts = true,
            "content-security-policy" => sh.has_csp = true,
            "x-content-type-options" => sh.has_x_content_type_options = true,
            "x-frame-options" => sh.has_x_frame_options = true,
            "referrer-policy" => {
                sh.has_referrer_policy = true;
                sh.referrer_policy_value = Some(value.clone());
            }
            "x-robots-tag" => {
                page.x_robots_tag = Some(value.clone());
            }
            _ => {}
        }
    }

    page.security_headers = sh;
}

/// Fetch security headers for all pages using GET requests.
/// We use GET instead of HEAD because some servers return different/fewer
/// headers for HEAD requests. The response body is discarded.
pub async fn run_header_requests(
    pages: &mut [PageData],
    user_agent: &str,
    timeout_seconds: u64,
    concurrency: u32,
) {
    use crate::crawler::fetcher::Fetcher;

    let fetcher = Arc::new(Fetcher::new(user_agent, timeout_seconds));
    let semaphore = Arc::new(Semaphore::new(concurrency as usize));

    // Collect URLs so we can spawn tasks without borrowing `pages`.
    let urls: Vec<String> = pages.iter().map(|p| p.url.clone()).collect();

    let mut handles = Vec::with_capacity(urls.len());

    for url in urls {
        let fetcher = Arc::clone(&fetcher);
        let sem = Arc::clone(&semaphore);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            match fetcher.fetch(&url).await {
                Ok(result) => Some((url, result.headers, result.response_time_ms)),
                Err(e) => {
                    warn!("Header request failed for {}: {}", url, e);
                    None
                }
            }
        }));
    }

    // Collect results and merge back.
    // (url, response headers, elapsed_ms) per fetched page.
    type HeaderFetchResult = Option<(String, Vec<(String, String)>, u64)>;
    let mut results: Vec<HeaderFetchResult> = Vec::with_capacity(handles.len());
    for handle in handles {
        results.push(handle.await.ok().flatten());
    }

    for (page, result) in pages.iter_mut().zip(results) {
        if let Some((_url, headers, response_time_ms)) = result {
            merge_head_response(page, &headers, response_time_ms);
        }
    }
}
