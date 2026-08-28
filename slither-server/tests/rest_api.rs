//! Request-level tests for the REST API.
//!
//! These drive the real router (auth, body-limit and trace layers included) via
//! `tower::ServiceExt::oneshot`, so routing, extractors and status codes are
//! exercised end to end without binding a socket.
//!
//! This binary never configures an API key, so the auth layer passes requests
//! through — auth enforcement is covered by the unit tests in `src/auth.rs`,
//! which can reach the process-global key.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use slither_core::jobs::{db, JobManager, JobType};
use slither_server::{build_router, AppState};

/// Build an AppState backed by a throwaway SQLite file and jobs directory, so
/// tests never touch the user's real `$SLITHER_HOME`.
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

async fn body_json(body: Body) -> Value {
    let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn health_is_reachable_without_auth() {
    let (state, _tmp) = test_state();
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

/// The queue ceiling is enforced inside the insert transaction. The queue is
/// filled through the manager directly with inspect jobs — only crawl jobs
/// spawn an executor, so nothing runs — and the request that should be refused
/// is a `crawl`, the one type the API accepts. It points at a closed local port
/// so that if the cap ever stopped working the test fails fast and offline
/// instead of reaching out to the network.
#[tokio::test]
async fn queue_cap_returns_429_once_full() {
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
    let (state, _tmp) = test_state();

    for i in 0..50 {
        state
            .job_manager
            .create_job(
                JobType::Inspect,
                &format!("https://example.com/{i}"),
                "example.com",
                json!({}),
            )
            .unwrap();
    }

    let app = build_router(Arc::clone(&state));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "type": "crawl", "url": "http://127.0.0.1:1/" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn under_the_cap_a_job_is_created() {
    std::env::set_var("SLITHER_ALLOW_PRIVATE_TARGETS", "1");
    let (state, _tmp) = test_state();
    let app = build_router(Arc::clone(&state));

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "type": "crawl", "url": "http://127.0.0.1:1/" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp.into_body()).await;
    assert_eq!(body["status"], "queued");
    assert_eq!(body["domain"], "127.0.0.1");
}

/// Only `crawl` is executable. The other types were accepted with a 201 and then
/// sat `queued` forever, and because REST and MCP share one queue cap, fifty of
/// them wedged job creation on both transports until the process restarted.
#[tokio::test]
async fn an_unexecutable_job_type_is_refused_rather_than_queued() {
    let (state, _tmp) = test_state();
    let app = build_router(Arc::clone(&state));

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "type": "inspect", "url": "https://example.com/" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        state
            .job_manager
            .list_jobs(&slither_core::jobs::manager::ListJobsFilter::default())
            .unwrap()
            .len(),
        0,
        "a refused job must leave no row behind"
    );
}

/// A non-local backend must be refused up front. Previously any backend string
/// was accepted and silently ran a local crawl.
#[tokio::test]
async fn crawl_rejects_a_non_local_backend() {
    let (state, _tmp) = test_state();
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "type": "crawl",
                        "url": "https://example.com/",
                        "options": { "backend": "playwright" }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unknown_job_returns_404() {
    let (state, _tmp) = test_state();
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/jobs/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// The result-file endpoint canonicalizes and confines paths to the job's own
/// output directory; a traversal attempt must never return the target file.
#[tokio::test]
async fn result_file_refuses_to_escape_the_output_dir() {
    let (state, tmp) = test_state();

    let job = state
        .job_manager
        .create_job(
            JobType::Crawl,
            "https://example.com/",
            "example.com",
            json!({}),
        )
        .unwrap();

    // A file that exists, but outside the job's output directory.
    let secret = tmp.path().join("secret.txt");
    std::fs::write(&secret, "top secret").unwrap();

    let app = build_router(Arc::clone(&state));
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/jobs/{}/results/{}",
                    job.id, "..%2F..%2Fsecret.txt"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_ne!(
        resp.status(),
        StatusCode::OK,
        "traversal must not resolve to a file outside the output dir"
    );
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes).contains("top secret"),
        "response must not leak the out-of-tree file"
    );
}

/// A file genuinely inside the output directory is served.
#[tokio::test]
async fn result_file_serves_a_file_inside_the_output_dir() {
    let (state, _tmp) = test_state();

    let job = state
        .job_manager
        .create_job(
            JobType::Crawl,
            "https://example.com/",
            "example.com",
            json!({}),
        )
        .unwrap();

    let dir = job.output_dir.clone().unwrap();
    std::fs::write(
        std::path::Path::new(&dir).join("crawl.json"),
        "{\"ok\":true}",
    )
    .unwrap();

    let app = build_router(Arc::clone(&state));
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/jobs/{}/results/crawl.json", job.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp.into_body()).await;
    assert_eq!(body["ok"], true);
}
