//! JSON-RPC level tests for the MCP server.
//!
//! `dispatch_line` is the sequential composition of the same parse/dispatch
//! pieces the stdio loop drives concurrently, so these cover framing and
//! protocol behaviour without a live stdio session.

use std::sync::Arc;

use serde_json::{json, Value};

use slither_core::jobs::{db, JobManager};
use slither_server::mcp::dispatch_line;
use slither_server::AppState;

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

/// Responses are serialized for assertion so the tests read against the wire
/// shape the client actually sees.
async fn dispatch(state: &Arc<AppState>, line: &str) -> Vec<Value> {
    dispatch_line(state, line)
        .await
        .iter()
        .map(|r| serde_json::to_value(r).unwrap())
        .collect()
}

#[tokio::test]
async fn initialize_echoes_a_supported_protocol_version() {
    let (state, _tmp) = test_state();

    let responses = dispatch(
        &state,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" }
        })
        .to_string(),
    )
    .await;

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "slither");
}

/// An unsupported version falls back to the newest one the server implements
/// rather than echoing something it cannot speak.
#[tokio::test]
async fn initialize_falls_back_for_an_unknown_version() {
    let (state, _tmp) = test_state();

    let responses = dispatch(
        &state,
        &json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "1999-01-01" }
        })
        .to_string(),
    )
    .await;

    assert_eq!(responses[0]["result"]["protocolVersion"], "2025-06-18");
}

#[tokio::test]
async fn tools_list_returns_the_tool_definitions() {
    let (state, _tmp) = test_state();

    let responses = dispatch(
        &state,
        &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }).to_string(),
    )
    .await;

    let tools = responses[0]["result"]["tools"].as_array().unwrap();
    assert!(!tools.is_empty());
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"slither_crawl"), "got {names:?}");
}

#[tokio::test]
async fn ping_is_answered() {
    let (state, _tmp) = test_state();

    let responses = dispatch(
        &state,
        &json!({ "jsonrpc": "2.0", "id": 3, "method": "ping" }).to_string(),
    )
    .await;

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["id"], 3);
    assert!(responses[0]["error"].is_null());
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let (state, _tmp) = test_state();

    let responses = dispatch(
        &state,
        &json!({ "jsonrpc": "2.0", "id": 4, "method": "no/such/method" }).to_string(),
    )
    .await;

    assert_eq!(responses[0]["error"]["code"], -32601);
}

/// Regression: malformed input was logged and skipped, leaving the client
/// waiting forever for a reply that never came.
#[tokio::test]
async fn malformed_json_returns_a_parse_error() {
    let (state, _tmp) = test_state();

    let responses = dispatch(&state, "this is not json").await;

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["error"]["code"], -32700);
    assert!(responses[0]["id"].is_null());
}

#[tokio::test]
async fn a_batch_is_dispatched_member_by_member() {
    let (state, _tmp) = test_state();

    let responses = dispatch(
        &state,
        &json!([
            { "jsonrpc": "2.0", "id": 1, "method": "ping" },
            { "jsonrpc": "2.0", "id": 2, "method": "tools/list" }
        ])
        .to_string(),
    )
    .await;

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[1]["id"], 2);
}

#[tokio::test]
async fn an_empty_batch_is_an_invalid_request() {
    let (state, _tmp) = test_state();

    let responses = dispatch(&state, "[]").await;

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["error"]["code"], -32600);
}

/// Notifications carry no id and MUST NOT be answered.
#[tokio::test]
async fn notifications_produce_no_response() {
    let (state, _tmp) = test_state();

    let initialized = dispatch(
        &state,
        &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
    )
    .await;
    assert!(initialized.is_empty());

    let cancelled = dispatch(
        &state,
        &json!({
            "jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": { "requestId": 7 }
        })
        .to_string(),
    )
    .await;
    assert!(cancelled.is_empty());

    // An unknown notification is likewise ignored rather than answered.
    let unknown = dispatch(
        &state,
        &json!({ "jsonrpc": "2.0", "method": "notifications/unknown" }).to_string(),
    )
    .await;
    assert!(unknown.is_empty());
}

#[tokio::test]
async fn resources_list_is_available() {
    let (state, _tmp) = test_state();

    let responses = dispatch(
        &state,
        &json!({ "jsonrpc": "2.0", "id": 5, "method": "resources/list" }).to_string(),
    )
    .await;

    assert!(responses[0]["result"]["resources"].is_array());
}

/// An unparseable batch member is answered against its own id so the client can
/// correlate the failure.
#[tokio::test]
async fn an_invalid_batch_member_keeps_its_id() {
    let (state, _tmp) = test_state();

    let responses = dispatch(
        &state,
        &json!([{ "jsonrpc": "2.0", "id": 9, "notmethod": true }]).to_string(),
    )
    .await;

    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["error"]["code"], -32600);
    assert_eq!(responses[0]["id"], 9);
}
