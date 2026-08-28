use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use slither_core::jobs::{Job, JobStatus, JobType, ListJobsFilter, WebhookManager};
use slither_core::CrawlConfig;

use crate::error::ApiError;
use crate::params;
use crate::AppState;

/// Queue-depth ceiling shared by the REST and MCP crawl entry points, so one
/// transport cannot queue work without bound while the other is capped.
pub const MAX_QUEUED_JOBS: u64 = 50;

// ---------------------------------------------------------------------------
// Request / query types
// ---------------------------------------------------------------------------

/// A crawl-job creation request.
///
/// `deny_unknown_fields` is load-bearing, not tidiness. Crawl parameters belong
/// under `options`, but the MCP tool takes them as flat arguments, so putting
/// `max_pages` at the top level here is an easy mistake — and it used to return
/// `201` and then crawl to the 500-page default against whatever site the
/// operator was trying to sample three pages of. Naming the misplaced field is
/// the whole point.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateJobRequest {
    #[serde(rename = "type")]
    pub job_type: String,
    pub url: String,
    pub options: Option<Value>,
    pub webhook_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListJobsQuery {
    pub status: Option<String>,
    #[serde(rename = "type")]
    pub job_type: Option<String>,
    pub domain: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// The job types this server actually executes.
///
/// The design document lists `inspect`/`extract`/`screenshot` as REST job
/// types, and [`JobType`] still parses them, but nothing dequeues them: one
/// used to be accepted `201 queued` and then sit there forever, holding a slot
/// against the ceiling that REST and MCP share. Fifty of them wedged every
/// subsequent creation on both transports until the process was restarted.
const EXECUTABLE_JOB_TYPES: &[&str] = &["crawl"];

/// Every option key a crawl honors. Anything else is a mistake worth naming:
/// an unrecognised key was previously accepted and dropped, so a misspelled
/// `maxpages` — or a `depth`, which this API has never supported — ran the
/// full default crawl while the caller believed it had asked for a small one.
pub const CRAWL_OPTION_KEYS: &[&str] = &[
    "max_pages",
    "concurrency",
    "delay_ms",
    "backend",
    "pagespeed",
    "pagespeed_sample",
    "pagespeed_strategy",
];

/// Refuse keys outside `allowed`, naming the offender and the accepted set.
pub fn reject_unknown_keys(what: &str, value: &Value, allowed: &[&str]) -> Result<(), String> {
    let Some(obj) = value.as_object() else {
        return Ok(());
    };
    let unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| !allowed.contains(k))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Unknown {what}{}: {}. Accepted: {}.",
        if unknown.len() == 1 { "" } else { "s" },
        unknown.join(", "),
        allowed.join(", ")
    ))
}

/// Build a crawl configuration from the JSON options object.
///
/// Shared by the REST `options` body and the MCP tool arguments, which carry
/// the same keys, so the two transports cannot drift in what they honor —
/// REST previously ignored the PageSpeed options entirely while MCP applied
/// them. Every value is read strictly (see [`crate::params`]) and validated
/// *before* any row or directory is written, so a request that cannot be
/// honored is refused instead of silently downgraded or half-committed.
pub fn crawl_config_from_json(seed_url: &str, options: &Value) -> Result<CrawlConfig, String> {
    if !(options.is_object() || options.is_null()) {
        return Err(format!(
            "'options' must be a JSON object, got {}.",
            match options {
                Value::Array(_) => "an array",
                Value::String(_) => "a string",
                Value::Number(_) => "a number",
                Value::Bool(_) => "a boolean",
                _ => "another type",
            }
        ));
    }
    let max_pages = params::u64_or(options, "max_pages", 500)?;
    if max_pages == 0 {
        return Err("Parameter 'max_pages' must be at least 1.".to_string());
    }
    // Clamping down is safe — it can only make the crawl smaller than asked.
    let max_pages = max_pages.min(10_000) as u32;

    let concurrency = params::u64_or(options, "concurrency", 3)?;
    if concurrency == 0 {
        return Err(
            "Parameter 'concurrency' must be at least 1 (0 would never issue a request)."
                .to_string(),
        );
    }
    let concurrency = concurrency.min(20) as u32;

    // No ceiling: a longer delay is always the polite direction.
    let delay_ms = params::u64_or(options, "delay_ms", 250)?;

    let backend = params::str_or(options, "backend", "local")?.to_string();
    if backend != "local" {
        return Err(format!(
            "backend '{backend}' is not supported by the server yet — only 'local'. \
             For JS-rendered crawls use the CLI: slither crawl <url> --backend {backend}."
        ));
    }

    let pagespeed = params::bool_or(options, "pagespeed", false)?;
    let pagespeed_sample = params::opt_u64(options, "pagespeed_sample")?
        .map(|v: u64| v.min(u32::MAX as u64) as u32)
        .filter(|v| *v > 0);
    let pagespeed_strategy = params::one_of(
        "pagespeed_strategy",
        params::str_or(options, "pagespeed_strategy", "mobile")?,
        &["mobile", "desktop"],
    )?
    .to_string();

    Ok(CrawlConfig {
        seed_url: seed_url.to_string(),
        max_pages,
        concurrency,
        delay_ms,
        backend,
        pagespeed,
        pagespeed_sample,
        pagespeed_strategy,
        ..Default::default()
    })
}

/// POST /api/v1/jobs
pub async fn create_job(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    // Deserialize by hand so an unknown or misspelled field comes back as a
    // 400 in this API's own error shape, naming the offender, rather than as
    // axum's plain-text 422.
    let req: CreateJobRequest = serde_json::from_value(body)
        .map_err(|e| ApiError::bad_request(format!("Invalid request body: {e}")))?;

    // ---- Validation only, in full, before anything is committed. ----------
    //
    // Ordering is load-bearing: the job row and its output directory are
    // written by `create_job_if_under_cap`, and a failure after that point
    // leaves a queued job the caller never receives an id for — an unreclaimable
    // slot against the shared queue ceiling.

    let job_type =
        JobType::from_str(&req.job_type).filter(|t| EXECUTABLE_JOB_TYPES.contains(&t.as_str()));
    let job_type = job_type.ok_or_else(|| {
        ApiError::bad_request(format!(
            "Job type '{}' is not supported. Supported types: {}. \
             Single-page audits and screenshots are available through the MCP tools \
             (slither_inspect, slither_screenshot) and the CLI, not as queued jobs.",
            req.job_type,
            EXECUTABLE_JOB_TYPES.join(", ")
        ))
    })?;

    // Parse URL to extract domain.
    let parsed_url = url::Url::parse(&req.url)
        .map_err(|e| ApiError::bad_request(format!("Invalid URL: {e}")))?;

    if !matches!(parsed_url.scheme(), "http" | "https") {
        return Err(ApiError::bad_request(
            "Only http and https URLs are supported",
        ));
    }

    let domain = parsed_url.host_str().unwrap_or("unknown").to_string();

    let config = req.options.unwrap_or(json!({}));

    // Backend, numeric options and PageSpeed settings all validated up front.
    // (The MCP tool applies the equivalent unknown-argument check against its
    // own published schema, so neither transport silently drops a setting.)
    reject_unknown_keys("options field", &config, CRAWL_OPTION_KEYS)
        .map_err(ApiError::bad_request)?;
    let crawl_config = crawl_config_from_json(&req.url, &config).map_err(ApiError::bad_request)?;

    // Pre-check the target the same way the MCP tool does: a private-address
    // target used to be accepted here only to fail later inside the fetcher —
    // a 201 followed by a failed job instead of a 400.
    slither_core::net_guard::check_url_allowed(&req.url)
        .await
        .map_err(ApiError::bad_request)?;

    // A webhook URL the caller mistyped is a client error, and it must be
    // caught here rather than after the job row exists.
    if let Some(ref webhook_url) = req.webhook_url {
        slither_core::jobs::webhook::validate_webhook_url(webhook_url)
            .map_err(|e| ApiError::bad_request(e.to_string()))?;
    }

    // ---- Commit. ----------------------------------------------------------

    // Persist the settings that will actually run, not the raw request. The
    // stored config is what `GET /jobs/{id}` reports, and it should not be able
    // to disagree with the crawl (it used to echo `{}` while the crawl ran with
    // defaults the caller never saw).
    let effective_config = json!({
        "max_pages": crawl_config.max_pages,
        "concurrency": crawl_config.concurrency,
        "delay_ms": crawl_config.delay_ms,
        "backend": crawl_config.backend,
        "pagespeed": crawl_config.pagespeed,
        "pagespeed_sample": crawl_config.pagespeed_sample,
        "pagespeed_strategy": crawl_config.pagespeed_strategy,
    });

    // The queue-depth ceiling is enforced inside the insert transaction, so a
    // burst of simultaneous requests cannot all observe the same under-cap
    // count and overshoot it.
    let job = state
        .job_manager
        .create_job_if_under_cap(
            job_type,
            &req.url,
            &domain,
            effective_config,
            MAX_QUEUED_JOBS,
        )
        .map_err(|e| ApiError::internal_logged("Failed to create job", e))?
        .ok_or_else(|| {
            ApiError::too_many_requests("Too many queued jobs. Wait for existing jobs to complete.")
        })?;

    // Register one-shot webhook if provided. The URL is already known good, so
    // only a storage failure can land here — roll the job back rather than
    // stranding a queued row nobody holds an id for.
    if let Some(ref webhook_url) = req.webhook_url {
        let wh_mgr = WebhookManager::new(state.job_manager.conn());
        if let Err(e) = wh_mgr.register_job_webhook(&job.id, webhook_url) {
            let _ = state.job_manager.delete_job(&job.id);
            return Err(ApiError::internal_logged("Failed to register webhook", e));
        }
    }

    crate::executor::fire_webhook(&state, &job.id, "job.queued").await;

    // Spawn the background crawl executor.
    let state_clone = Arc::clone(&state);
    let semaphore = Arc::clone(&state.crawl_semaphore);
    let job_id = job.id.clone();
    tokio::spawn(async move {
        crate::executor::execute_crawl_job(state_clone, job_id, crawl_config, semaphore).await;
    });

    let body = json!({
        "id": job.id,
        "status": job.status,
        "type": job.job_type,
        "domain": job.domain,
        "created_at": job.created_at,
    });

    Ok((StatusCode::CREATED, Json(body)))
}

/// GET /api/v1/jobs/{id}
pub async fn get_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let job = state
        .job_manager
        .get_job(&id)
        .map_err(|e| ApiError::internal_logged("Failed to get job", e))?
        .ok_or_else(|| ApiError::not_found(format!("Job not found: {id}")))?;

    Ok(Json(job_to_json(&job)))
}

/// GET /api/v1/jobs
pub async fn list_jobs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListJobsQuery>,
) -> Result<Json<Value>, ApiError> {
    let filter = ListJobsFilter {
        status: query.status,
        job_type: query.job_type,
        domain: query.domain,
        limit: query.limit.unwrap_or(20),
        offset: query.offset.unwrap_or(0),
    };

    let jobs = state
        .job_manager
        .list_jobs(&filter)
        .map_err(|e| ApiError::internal_logged("Failed to list jobs", e))?;

    let count = jobs.len();
    let jobs_json: Vec<Value> = jobs.iter().map(job_to_json).collect();

    Ok(Json(json!({
        "jobs": jobs_json,
        "count": count,
    })))
}

/// DELETE /api/v1/jobs/{id}
pub async fn delete_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // If the job has not finished, cancel it first — and *tell* anyone
    // listening. Deleting an unfinished job used to deliver nothing at all: the
    // row (and its cascaded one-shot registration) was gone before any terminal
    // event fired, so a caller waiting on `webhook_url` waited forever.
    if let Ok(Some(job)) = state.job_manager.get_job(&id) {
        if matches!(job.status, JobStatus::Running | JobStatus::Queued) {
            let cancelled = state
                .job_manager
                .update_status(&id, JobStatus::Cancelled)
                .map_err(|e| ApiError::internal_logged("Failed to cancel job", e))?;
            if cancelled {
                crate::executor::fire_webhook(&state, &id, "job.cancelled").await;
            }
        }
    }

    let deleted = state
        .job_manager
        .delete_job(&id)
        .map_err(|e| ApiError::internal_logged("Failed to delete job", e))?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("Job not found: {id}")))
    }
}

/// GET /api/v1/jobs/{id}/results/{filename}
pub async fn get_result_file(
    State(state): State<Arc<AppState>>,
    Path((id, filename)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let job = state
        .job_manager
        .get_job(&id)
        .map_err(|e| ApiError::internal_logged("Failed to get job", e))?
        .ok_or_else(|| ApiError::not_found(format!("Job not found: {id}")))?;

    let output_dir = job
        .output_dir
        .as_ref()
        .ok_or_else(|| ApiError::not_found("Job has no output directory"))?;

    let base = std::path::PathBuf::from(output_dir);
    let file_path = base.join(&filename);

    // Canonicalize both to resolve symlinks, .., etc — then verify the result is inside the output dir.
    let canonical_base = base
        .canonicalize()
        .map_err(|_| ApiError::not_found("Job output directory not found"))?;
    let canonical_file = file_path
        .canonicalize()
        .map_err(|_| ApiError::not_found(format!("File not found: {filename}")))?;
    if !canonical_file.starts_with(&canonical_base) {
        return Err(ApiError::bad_request("Invalid filename"));
    }

    let bytes = std::fs::read(&canonical_file)
        .map_err(|_| ApiError::not_found(format!("File not found: {filename}")))?;

    let content_type = match file_path.extension().and_then(|ext| ext.to_str()) {
        Some("json") => "application/json",
        Some("html") => "text/html; charset=utf-8",
        Some("csv") => "text/csv; charset=utf-8",
        Some("png") => "image/png",
        Some("jpeg") | Some("jpg") => "image/jpeg",
        _ => "application/octet-stream",
    };

    // Defense in depth: the report contains crawled content. Serve it as an
    // attachment with nosniff and a locked-down CSP so a browser never treats
    // it as an active document on the API origin.
    let is_html = matches!(file_path.extension().and_then(|e| e.to_str()), Some("html"));
    let mut headers = axum::http::HeaderMap::new();
    if let Ok(v) = content_type.parse() {
        headers.insert(header::CONTENT_TYPE, v);
    }
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    if is_html {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            axum::http::HeaderValue::from_static(
                "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src data:",
            ),
        );
        if let Ok(v) = format!("attachment; filename=\"{filename}\"").parse() {
            headers.insert(header::CONTENT_DISPOSITION, v);
        }
    }

    Ok((headers, bytes))
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn job_to_json(job: &Job) -> Value {
    json!({
        "id": job.id,
        "type": job.job_type,
        "status": job.status,
        "domain": job.domain,
        "url": job.url,
        "config": job.config,
        "progress": job.progress,
        "result_summary": job.result_summary,
        "error": job.error,
        "created_at": job.created_at,
        "started_at": job.started_at,
        "completed_at": job.completed_at,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use slither_core::jobs::db;
    use tower::ServiceExt;

    /// The crawl pre-check resolves DNS for non-literal hosts; allowing private
    /// targets short-circuits it, so these tests stay offline. Nothing else in
    /// this binary reads the variable.
    fn allow_private_targets() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1"));
    }

    fn test_state() -> (Arc<AppState>, tempfile::TempDir) {
        allow_private_targets();
        let tmp = tempfile::TempDir::new().unwrap();
        let conn = db::open_db(&tmp.path().join("slither.db")).unwrap();
        let jobs_dir = tmp.path().join("jobs");
        std::fs::create_dir_all(&jobs_dir).unwrap();
        let state = Arc::new(AppState {
            job_manager: slither_core::jobs::JobManager::new(conn, jobs_dir),
            crawl_semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        });
        (state, tmp)
    }

    /// A target that refuses connections instantly, so a job that is *meant* to
    /// be created does not run a real crawl during the test.
    const DEAD_TARGET: &str = "http://127.0.0.1:1/";

    async fn post_job(state: &Arc<AppState>, body: Value) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/jobs")
            .header("content-type", "application/json");
        // The process-global API key is set by `auth`'s own tests, which share
        // this binary; authenticate when one is present so these tests exercise
        // the handler rather than the auth layer, whatever order they run in.
        if let Some(key) = crate::API_KEY.get() {
            request = request.header("authorization", format!("Bearer {key}"));
        }
        let resp = crate::build_router(Arc::clone(state))
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, json)
    }

    fn error_of(body: &Value) -> String {
        body["error"].as_str().unwrap_or_default().to_string()
    }

    fn queued(state: &Arc<AppState>) -> u64 {
        state.job_manager.count_jobs_by_status("queued").unwrap()
    }

    // -- B16: job types with no executor --------------------------------

    /// `{"type":"inspect"}` used to return 201 and then sit queued forever;
    /// fifty of them made every later creation on either transport return 429
    /// until the process was restarted.
    #[tokio::test]
    async fn a_job_type_with_no_executor_is_refused() {
        let (state, _tmp) = test_state();

        for job_type in ["inspect", "extract", "screenshot", "entity"] {
            let (status, body) =
                post_job(&state, json!({ "type": job_type, "url": DEAD_TARGET })).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{job_type} must be refused, not queued: {body}"
            );
            let err = error_of(&body);
            assert!(
                err.contains(job_type),
                "the error should name the type: {err}"
            );
            assert!(
                err.contains("crawl"),
                "the error should name what is supported: {err}"
            );
        }

        assert_eq!(
            queued(&state),
            0,
            "a refused type must not occupy a queue slot"
        );
    }

    #[tokio::test]
    async fn an_unknown_job_type_is_refused() {
        let (state, _tmp) = test_state();
        let (status, body) =
            post_job(&state, json!({ "type": "nonsense", "url": DEAD_TARGET })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(error_of(&body).contains("crawl"));
    }

    #[tokio::test]
    async fn a_crawl_job_is_created_under_the_cap() {
        let (state, _tmp) = test_state();
        let (status, body) = post_job(&state, json!({ "type": "crawl", "url": DEAD_TARGET })).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["status"], "queued");
        assert_eq!(body["type"], "crawl");
    }

    #[tokio::test]
    async fn the_queue_cap_returns_429_once_full() {
        let (state, _tmp) = test_state();
        for i in 0..MAX_QUEUED_JOBS {
            state
                .job_manager
                .create_job(
                    JobType::Crawl,
                    &format!("http://127.0.0.1:1/{i}"),
                    "127.0.0.1",
                    json!({}),
                )
                .unwrap();
        }
        let (status, _) = post_job(&state, json!({ "type": "crawl", "url": DEAD_TARGET })).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    // -- B15: numeric options -------------------------------------------

    /// `3` and `3.0` are the same number. `as_u64` disagreed, so `3.0` silently
    /// became the 500-page default at 4 requests/second.
    #[tokio::test]
    async fn an_integral_float_option_is_honored() {
        let (state, _tmp) = test_state();
        let (status, body) = post_job(
            &state,
            json!({
                "type": "crawl", "url": DEAD_TARGET,
                "options": { "max_pages": 3.0, "delay_ms": 1000.0, "concurrency": 1.0 }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        let job = state
            .job_manager
            .get_job(body["id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(job.config["max_pages"], 3);
        assert_eq!(job.config["delay_ms"], 1000);
        assert_eq!(job.config["concurrency"], 1);
    }

    #[tokio::test]
    async fn a_plain_integer_option_is_still_honored() {
        let (state, _tmp) = test_state();
        let (_, body) = post_job(
            &state,
            json!({ "type": "crawl", "url": DEAD_TARGET, "options": { "max_pages": 3 } }),
        )
        .await;
        let job = state
            .job_manager
            .get_job(body["id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(job.config["max_pages"], 3);
    }

    /// The dangerous half of B15: a politeness setting that cannot be honored
    /// must be refused, never replaced by a faster default.
    #[tokio::test]
    async fn an_unusable_option_is_refused_not_defaulted() {
        let (state, _tmp) = test_state();

        for (key, value) in [
            ("max_pages", json!(2.5)),
            ("max_pages", json!("3")),
            ("max_pages", json!(0)),
            ("delay_ms", json!(-1)),
            ("delay_ms", json!(true)),
            ("concurrency", json!("many")),
            ("concurrency", json!(0)),
        ] {
            let (status, body) = post_job(
                &state,
                json!({ "type": "crawl", "url": DEAD_TARGET, "options": { key: value } }),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{key}={value} must be refused: {body}"
            );
            assert!(error_of(&body).contains(key), "the error should name {key}");
        }
        assert_eq!(
            queued(&state),
            0,
            "a refused request must leave no job behind"
        );
    }

    /// Crawl parameters belong under `options`. Putting one at the top level —
    /// the shape the MCP tool uses — used to be accepted and ignored.
    #[tokio::test]
    async fn a_misplaced_top_level_parameter_is_named() {
        let (state, _tmp) = test_state();
        let (status, body) = post_job(
            &state,
            json!({ "type": "crawl", "url": DEAD_TARGET, "max_pages": 3 }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            error_of(&body).contains("max_pages"),
            "the error must name the misplaced field: {}",
            error_of(&body)
        );
        assert_eq!(queued(&state), 0);
    }

    /// The same mistake one level down: an option key that is not a real option.
    #[tokio::test]
    async fn an_unknown_option_key_is_named() {
        let (state, _tmp) = test_state();
        let (status, body) = post_job(
            &state,
            json!({ "type": "crawl", "url": DEAD_TARGET, "options": { "maxpages": 3, "depth": 2 } }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let err = error_of(&body);
        assert!(
            err.contains("maxpages") && err.contains("max_pages"),
            "{err}"
        );
        assert_eq!(queued(&state), 0);
    }

    /// PageSpeed settings used to be accepted by REST and dropped on the floor
    /// while the MCP tool honored them; both now run the same config.
    #[tokio::test]
    async fn pagespeed_options_reach_the_stored_config() {
        let (state, _tmp) = test_state();
        let (status, body) = post_job(
            &state,
            json!({
                "type": "crawl", "url": DEAD_TARGET,
                "options": { "pagespeed": true, "pagespeed_strategy": "desktop", "pagespeed_sample": 5 }
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let job = state
            .job_manager
            .get_job(body["id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(job.config["pagespeed"], true);
        assert_eq!(job.config["pagespeed_strategy"], "desktop");
        assert_eq!(job.config["pagespeed_sample"], 5);
    }

    #[tokio::test]
    async fn an_unknown_pagespeed_strategy_is_refused() {
        let (state, _tmp) = test_state();
        let (status, body) = post_job(
            &state,
            json!({ "type": "crawl", "url": DEAD_TARGET, "options": { "pagespeed_strategy": "turbo" } }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(error_of(&body).contains("mobile"));
    }

    // -- B17: webhook validation ordering -------------------------------

    /// A mistyped `webhook_url` used to return 500 — a retryable status — after
    /// the job row and its output directory were already committed, so the
    /// caller never learned the id and the queue slot could never be reclaimed.
    #[tokio::test]
    async fn a_bad_webhook_url_is_a_400_and_leaves_no_job() {
        let (state, _tmp) = test_state();

        for bad in ["not-a-url", "ftp://example.com/hook", ""] {
            let (status, body) = post_job(
                &state,
                json!({ "type": "crawl", "url": DEAD_TARGET, "webhook_url": bad }),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "webhook '{bad}' must be a client error: {body}"
            );
            assert!(
                !error_of(&body).is_empty(),
                "the response must say what was wrong"
            );
        }

        assert_eq!(
            queued(&state),
            0,
            "a rejected webhook must not strand a queued job"
        );
        assert_eq!(
            std::fs::read_dir(state.job_manager.jobs_dir())
                .unwrap()
                .count(),
            0,
            "a rejected webhook must not strand an output directory"
        );
    }

    #[tokio::test]
    async fn a_valid_webhook_url_still_creates_the_job() {
        let (state, _tmp) = test_state();
        let (status, body) = post_job(
            &state,
            json!({
                "type": "crawl", "url": DEAD_TARGET,
                "webhook_url": "https://example.com/hook"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    // -- request shape ---------------------------------------------------

    #[tokio::test]
    async fn a_missing_required_field_is_a_400() {
        let (state, _tmp) = test_state();
        let (status, _) = post_job(&state, json!({ "url": DEAD_TARGET })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn non_object_options_are_refused() {
        let (state, _tmp) = test_state();
        let (status, body) = post_job(
            &state,
            json!({ "type": "crawl", "url": DEAD_TARGET, "options": [1, 2] }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
}
