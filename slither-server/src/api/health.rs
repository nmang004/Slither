use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use slither_core::jobs::ListJobsFilter;

use crate::AppState;

pub async fn health(State(state): State<Arc<AppState>>) -> Json<Value> {
    let active_jobs = state
        .job_manager
        .list_jobs(&ListJobsFilter {
            status: Some("running".to_string()),
            ..Default::default()
        })
        .map(|jobs| jobs.len())
        .unwrap_or(0);

    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "active_jobs": active_jobs,
    }))
}
