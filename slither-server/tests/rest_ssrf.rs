//! The SSRF pre-check on job creation.
//!
//! Kept in its own test binary because it clears
//! `SLITHER_ALLOW_PRIVATE_TARGETS`, which is process-global — sharing a binary
//! with other tests would let that mutation race them.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;

use slither_core::jobs::{db, JobManager};
use slither_server::{build_router, AppState};

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

/// A private-address crawl target is rejected at creation with a 400. It used
/// to be accepted with a 201 and fail later inside the fetcher, leaving a
/// failed job instead of a clear client error.
#[tokio::test]
async fn crawl_of_a_private_target_is_rejected_with_400() {
    std::env::remove_var("SLITHER_ALLOW_PRIVATE_TARGETS");

    let (state, _tmp) = test_state();
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "type": "crawl", "url": "http://127.0.0.1:9/" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn crawl_of_localhost_by_name_is_rejected() {
    std::env::remove_var("SLITHER_ALLOW_PRIVATE_TARGETS");

    let (state, _tmp) = test_state();
    let app = build_router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/jobs")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "type": "crawl", "url": "http://localhost:3000/" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
