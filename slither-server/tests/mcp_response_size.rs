//! Guardrails on the size of MCP tool responses.
//!
//! The product thesis is that structured, token-efficient tool output beats the
//! HTML dashboard — an agent has to be able to call these tools at realistic
//! crawl sizes without exhausting its own context. Nothing enforced that, and a
//! sitewide check embedding one entry per crawled page made a single default
//! `slither_query` call grow without bound. These tests fail if that returns.

use std::sync::Arc;

use serde_json::{json, Value};

use slither_core::jobs::{db, JobManager, JobStatus, JobType};
use slither_server::mcp::dispatch_line;
use slither_server::AppState;

/// Roughly four characters per token — enough to reason about context cost
/// without pulling in a tokenizer.
const CHARS_PER_TOKEN: usize = 4;

fn test_state() -> (Arc<AppState>, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let conn = db::open_db(&tmp.path().join("slither.db")).unwrap();
    let jobs_dir = tmp.path().join("jobs");
    std::fs::create_dir_all(&jobs_dir).unwrap();

    let state = Arc::new(AppState {
        job_manager: JobManager::new(conn, jobs_dir),
        crawl_semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
    });
    (state, tmp)
}

/// A completed job whose crawl.json has `pages` pages and, for each of
/// `sitewide_checks`, one issue affecting every one of them — the shape that
/// caused the blow-up (missing security headers, HTTP-not-HTTPS, and similar
/// fire on every page of a real site).
fn completed_job_with_sitewide_issues(
    state: &Arc<AppState>,
    pages: usize,
    sitewide_checks: usize,
) -> String {
    let job = state
        .job_manager
        .create_job(
            JobType::Crawl,
            "https://example.com",
            "example.com",
            json!({}),
        )
        .unwrap();

    let page_urls: Vec<String> = (0..pages)
        .map(|i| format!("https://example.com/section/page-{i}"))
        .collect();

    let page_values: Vec<Value> = page_urls
        .iter()
        .map(|url| {
            json!({
                "url": url, "status": 200, "redirect_chain": null,
                "response_time_ms": 42, "content_type": "text/html; charset=utf-8",
                "depth": 1, "title": "A representative page title",
                "meta_description": "A representative meta description for the page.",
                "meta_robots": null, "canonical": url, "h1": ["Heading"],
                "headings": [], "word_count": 500, "body_text": "",
                "internal_links": [], "external_links": [], "images": [],
                "schema_types": [], "og_tags": {}, "content_hash": "abc123"
            })
        })
        .collect();

    let issue_values: Vec<Value> = (0..sitewide_checks)
        .map(|c| {
            json!({
                "category": "security",
                "check": format!("sitewide_check_{c}"),
                "display_name": format!("Sitewide Check {c}"),
                "severity": "info",
                "description": "A check that fires on every crawled page.",
                "guidance": "Guidance text of the sort every issue carries, long \
                             enough to be representative of real analyzer output.",
                "urls": page_urls
                    .iter()
                    .map(|u| json!({ "url": u, "detail": "missing" }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    let crawl = json!({
        "slither_version": "0.3.0",
        "crawl_metadata": {
            "domain": "example.com", "seed_url": "https://example.com",
            "crawl_date": "2026-08-14T00:00:00Z", "duration_ms": 1000,
            "pages_discovered": pages, "pages_crawled": pages,
            "pages_skipped_robots": 0, "pages_errored": 0,
            "settings": slither_core::CrawlConfig::default(), "backend": "local"
        },
        "export_settings": { "include_body_text": false, "summary_only": false, "format": "pretty" },
        "pages": page_values,
        "issues": { "issues": issue_values },
        "summary": {
            "total_pages": pages, "by_status": { "200": pages },
            "avg_response_time_ms": 42, "avg_word_count": 500,
            "total_internal_links": 0, "total_external_links": 0, "total_images": 0,
            "images_without_alt": 0, "pages_with_schema": 0,
            "total_issues": sitewide_checks * pages, "critical_issues": 0,
            "warning_issues": 0, "info_issues": sitewide_checks * pages,
            "issues_by_category": {}, "health_score": 80, "grade": "B",
            "grade_verdict": "Good", "response_time_p50_ms": 42,
            "response_time_p95_ms": 42, "cwv_pages_tested": 0, "cwv_pages_good": 0,
            "cwv_pages_needs_work": 0, "cwv_pages_poor": 0
        }
    });

    let dir = job.output_dir.clone().unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        std::path::Path::new(&dir).join("crawl.json"),
        serde_json::to_string(&crawl).unwrap(),
    )
    .unwrap();
    state
        .job_manager
        .update_status(&job.id, JobStatus::Completed)
        .unwrap();

    job.id
}

async fn call_tool(state: &Arc<AppState>, name: &str, arguments: Value) -> String {
    let line = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    })
    .to_string();

    let responses = dispatch_line(state, &line).await;
    serde_json::to_string(&responses[0]).unwrap()
}

/// The regression this suite exists for. `per_page` bounds how many *issues*
/// come back, but each issue used to embed every affected URL, so a 500-page
/// crawl with a handful of sitewide checks produced a response in the hundreds
/// of thousands of tokens from one default call.
#[tokio::test]
async fn default_query_stays_small_on_a_large_crawl() {
    let (state, _tmp) = test_state();
    let job_id = completed_job_with_sitewide_issues(&state, 500, 6);

    let body = call_tool(&state, "slither_query", json!({ "job_id": job_id })).await;
    let approx_tokens = body.len() / CHARS_PER_TOKEN;

    assert!(
        approx_tokens < 4_000,
        "a default slither_query on a 500-page crawl produced ~{approx_tokens} tokens \
         ({} chars); it must stay small enough to call safely",
        body.len()
    );
}

/// The response must still say how much was left out, or the caller silently
/// receives a partial list and cannot tell.
#[tokio::test]
async fn truncated_issues_report_the_true_affected_count() {
    let (state, _tmp) = test_state();
    let job_id = completed_job_with_sitewide_issues(&state, 300, 1);

    let body = call_tool(&state, "slither_query", json!({ "job_id": job_id })).await;
    let parsed: Value = serde_json::from_str(&body).unwrap();
    let issue = &parsed["result"]["structuredContent"]["issues"][0];

    assert_eq!(
        issue["affected_url_count"], 300,
        "true total must be reported"
    );
    assert_eq!(issue["affected_urls_truncated"], true);
    assert_eq!(
        issue["affected_urls"].as_array().unwrap().len(),
        5,
        "default cap is 5 URLs per issue"
    );
}

/// Raising the cap must work, so a caller that genuinely wants more can ask.
#[tokio::test]
async fn max_urls_per_issue_is_honored_and_bounded() {
    let (state, _tmp) = test_state();
    let job_id = completed_job_with_sitewide_issues(&state, 300, 1);

    let body = call_tool(
        &state,
        "slither_query",
        json!({ "job_id": job_id, "max_urls_per_issue": 100 }),
    )
    .await;
    let parsed: Value = serde_json::from_str(&body).unwrap();
    let issue = &parsed["result"]["structuredContent"]["issues"][0];
    assert_eq!(issue["affected_urls"].as_array().unwrap().len(), 100);

    // Above the ceiling the request is clamped, not honored verbatim.
    let body = call_tool(
        &state,
        "slither_query",
        json!({ "job_id": job_id, "max_urls_per_issue": 100_000 }),
    )
    .await;
    let parsed: Value = serde_json::from_str(&body).unwrap();
    let issue = &parsed["result"]["structuredContent"]["issues"][0];
    assert!(issue["affected_urls"].as_array().unwrap().len() <= 500);
}

/// An issue affecting few pages must come back complete — the cap should be
/// invisible in the common case.
#[tokio::test]
async fn small_issues_are_not_truncated() {
    let (state, _tmp) = test_state();
    let job_id = completed_job_with_sitewide_issues(&state, 5, 1);

    let body = call_tool(&state, "slither_query", json!({ "job_id": job_id })).await;
    let parsed: Value = serde_json::from_str(&body).unwrap();
    let issue = &parsed["result"]["structuredContent"]["issues"][0];

    assert_eq!(issue["affected_url_count"], 5);
    assert_eq!(issue["affected_urls_truncated"], false);
    assert_eq!(issue["affected_urls"].as_array().unwrap().len(), 5);
}

/// slither_summary is the "what shape is this site in?" call and must stay cheap
/// regardless of crawl size.
#[tokio::test]
async fn summary_stays_small_on_a_large_crawl() {
    let (state, _tmp) = test_state();
    let job_id = completed_job_with_sitewide_issues(&state, 1000, 6);

    let body = call_tool(&state, "slither_summary", json!({ "job_id": job_id })).await;
    let approx_tokens = body.len() / CHARS_PER_TOKEN;

    assert!(
        approx_tokens < 2_000,
        "slither_summary produced ~{approx_tokens} tokens on a 1000-page crawl"
    );
}

/// The link graph bounds every list by top_n, not just the ranked ones.
#[tokio::test]
async fn link_graph_bounds_all_of_its_lists() {
    let (state, _tmp) = test_state();
    // Every page is an orphan here: no page links to any other.
    let job_id = completed_job_with_sitewide_issues(&state, 400, 1);

    let body = call_tool(
        &state,
        "slither_link_graph",
        json!({ "job_id": job_id, "top_n": 10 }),
    )
    .await;
    let parsed: Value = serde_json::from_str(&body).unwrap();
    let graph = &parsed["result"]["structuredContent"];

    assert!(
        graph["orphan_pages"].as_array().unwrap().len() <= 10,
        "orphan_pages must respect top_n"
    );
    assert!(
        graph["silo_distribution"].as_array().unwrap().len() <= 10,
        "silo_distribution must respect top_n"
    );
    assert_eq!(
        graph["orphan_page_count"], 400,
        "the true orphan total must still be reported"
    );

    let approx_tokens = body.len() / CHARS_PER_TOKEN;
    assert!(
        approx_tokens < 2_000,
        "slither_link_graph produced ~{approx_tokens} tokens"
    );
}

// ---------------------------------------------------------------------------
// Backwards-compatibility text block
// ---------------------------------------------------------------------------

use slither_server::mcp::{dispatch_line_in_session, McpSession};

async fn call_tool_in_session(
    state: &Arc<AppState>,
    session: &McpSession,
    name: &str,
    arguments: Value,
) -> Value {
    let line = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    })
    .to_string();

    let responses = dispatch_line_in_session(state, session, &line).await;
    serde_json::to_value(&responses[0]).unwrap()
}

async fn initialize(state: &Arc<AppState>, session: &McpSession, version: &str) {
    let line = json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {
            "protocolVersion": version,
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "1" }
        }
    })
    .to_string();
    dispatch_line_in_session(state, session, &line).await;
}

/// A client on a revision that understands `structuredContent` must not be sent
/// the same JSON twice. The duplicate was 47% of every response.
#[tokio::test]
async fn a_modern_client_is_not_sent_the_payload_twice() {
    let (state, _tmp) = test_state();
    let job_id = completed_job_with_sitewide_issues(&state, 200, 4);

    let session = McpSession::new();
    initialize(&state, &session, "2025-06-18").await;
    let response = call_tool_in_session(
        &state,
        &session,
        "slither_query",
        json!({ "job_id": job_id }),
    )
    .await;

    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let structured = &response["result"]["structuredContent"];

    assert!(
        !structured.is_null(),
        "structuredContent must still carry the full payload"
    );
    assert!(
        text.len() < 400,
        "the text block should be a short summary, got {} chars",
        text.len()
    );
    assert!(
        text.contains("slither_query") && text.contains("structuredContent"),
        "the summary should name the tool and point at the real payload: {text}"
    );
}

/// A client on an older revision cannot read `structuredContent`, so the text
/// block must still carry the full serialized JSON.
#[tokio::test]
async fn a_legacy_client_still_receives_the_full_json_as_text() {
    let (state, _tmp) = test_state();
    let job_id = completed_job_with_sitewide_issues(&state, 20, 2);

    let session = McpSession::new();
    initialize(&state, &session, "2024-11-05").await;
    let response = call_tool_in_session(
        &state,
        &session,
        "slither_query",
        json!({ "job_id": job_id }),
    )
    .await;

    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let structured = &response["result"]["structuredContent"];

    let parsed: Value = serde_json::from_str(text)
        .expect("a legacy client's text block must be the parseable JSON payload");
    assert_eq!(
        &parsed, structured,
        "text and structured payloads must agree"
    );
}

/// Before `initialize`, nothing is known about the client, so the conservative
/// (duplicated) shape is used rather than assuming modern behavior.
#[tokio::test]
async fn an_unnegotiated_session_defaults_to_the_compatible_shape() {
    let (state, _tmp) = test_state();
    let job_id = completed_job_with_sitewide_issues(&state, 10, 1);

    let session = McpSession::new();
    let response = call_tool_in_session(
        &state,
        &session,
        "slither_query",
        json!({ "job_id": job_id }),
    )
    .await;

    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str::<Value>(text)
        .expect("without negotiation the text block must remain full JSON");
}

/// The saving is the point. The duplicate is not quite half the response —
/// the text copy carries JSON escaping the structured one does not — so the
/// realistic reduction is a little under 50%.
#[tokio::test]
async fn dropping_the_duplicate_roughly_halves_the_response() {
    let (state, _tmp) = test_state();
    let job_id = completed_job_with_sitewide_issues(&state, 200, 4);

    let legacy = McpSession::new();
    initialize(&state, &legacy, "2024-11-05").await;
    let legacy_len = serde_json::to_string(
        &call_tool_in_session(
            &state,
            &legacy,
            "slither_query",
            json!({ "job_id": &job_id }),
        )
        .await,
    )
    .unwrap()
    .len();

    let modern = McpSession::new();
    initialize(&state, &modern, "2025-06-18").await;
    let modern_len = serde_json::to_string(
        &call_tool_in_session(
            &state,
            &modern,
            "slither_query",
            json!({ "job_id": &job_id }),
        )
        .await,
    )
    .unwrap()
    .len();

    let ratio = modern_len as f64 / legacy_len as f64;
    assert!(
        ratio < 0.6,
        "expected the modern response ({modern_len}) to be well under the legacy one \
         ({legacy_len}); ratio was {ratio:.2}"
    );
}
