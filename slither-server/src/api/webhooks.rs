use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use slither_core::jobs::WebhookManager;

use crate::error::ApiError;
use crate::AppState;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateWebhookRequest {
    pub url: String,
    pub events: Vec<String>,
}

// ---------------------------------------------------------------------------
// Valid events
// ---------------------------------------------------------------------------

/// Events a caller may subscribe to. Every one of these is emitted by the
/// executor or an API handler — see the tests in `executor.rs`, which fail if
/// the two lists drift apart.
pub const VALID_EVENTS: &[&str] = &[
    "job.queued",
    "job.running",
    "job.completed",
    "job.failed",
    "job.cancelled",
];

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /api/v1/webhooks
pub async fn create_webhook(
    State(state): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let req: CreateWebhookRequest = serde_json::from_value(body)
        .map_err(|e| ApiError::bad_request(format!("Invalid request body: {e}")))?;

    // Validate that all requested events are in the valid set.
    for event in &req.events {
        if !VALID_EVENTS.contains(&event.as_str()) {
            return Err(ApiError::bad_request(format!(
                "Invalid event: {event}. Valid events: {}",
                VALID_EVENTS.join(", ")
            )));
        }
    }

    if req.events.is_empty() {
        return Err(ApiError::bad_request("At least one event is required"));
    }

    // A malformed or private-target URL and a full table are both client
    // errors. Registration reported them as 500s, which tells a client to
    // retry something that can never succeed.
    slither_core::jobs::webhook::validate_webhook_url(&req.url)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let wh_mgr = WebhookManager::new(state.job_manager.conn());
    let registered = wh_mgr
        .count()
        .map_err(|e| ApiError::internal_logged("Failed to count webhooks", e))?;
    if registered >= slither_core::jobs::webhook::MAX_WEBHOOKS {
        return Err(ApiError::too_many_requests(format!(
            "Too many registered webhooks ({registered}); delete one before adding another."
        )));
    }

    let webhook = wh_mgr
        .register(&req.url, &req.events)
        .map_err(|e| ApiError::internal_logged("Failed to register webhook", e))?;

    let body = json!({
        "id": webhook.id,
        "url": webhook.url,
        "events": webhook.events,
        "created_at": webhook.created_at,
    });

    Ok((StatusCode::CREATED, Json(body)))
}

/// GET /api/v1/webhooks
pub async fn list_webhooks(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let wh_mgr = WebhookManager::new(state.job_manager.conn());
    let webhooks = wh_mgr
        .list()
        .map_err(|e| ApiError::internal_logged("Failed to list webhooks", e))?;

    let webhooks_json: Vec<Value> = webhooks
        .iter()
        .map(|wh| {
            json!({
                "id": wh.id,
                "url": wh.url,
                "events": wh.events,
                "created_at": wh.created_at,
            })
        })
        .collect();

    Ok(Json(json!({ "webhooks": webhooks_json })))
}

/// DELETE /api/v1/webhooks/{id}
pub async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let wh_mgr = WebhookManager::new(state.job_manager.conn());
    let deleted = wh_mgr
        .delete(&id)
        .map_err(|e| ApiError::internal_logged("Failed to delete webhook", e))?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("Webhook not found: {id}")))
    }
}
