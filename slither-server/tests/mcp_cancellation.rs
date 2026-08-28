//! Cancellation tests for the MCP server.
//!
//! These drive `spawn_tracked` — the same helper the stdio loop uses — rather
//! than a stand-in, so the registration, abort and deregistration paths under
//! test are the ones that run in production. The work future is supplied by the
//! test, which is what makes "did the work actually stop?" observable.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::Mutex;
use tokio::task::JoinSet;

use slither_server::mcp::transport::{JsonRpcResponse, StdioWriter};
use slither_server::mcp::{is_cancellable, spawn_tracked, CancelRegistry};

/// How long the simulated work runs before it would mark itself complete.
const WORK: Duration = Duration::from_millis(200);
/// Comfortably longer than `WORK`, so a flag still unset afterwards means the
/// work was stopped rather than merely unfinished.
const PAST_WORK: Duration = Duration::from_millis(600);

fn writer() -> Arc<Mutex<StdioWriter>> {
    Arc::new(Mutex::new(StdioWriter::new()))
}

/// Work that sleeps, then records that it ran to completion.
///
/// It resolves to `None` so nothing is ever written to the real stdout the test
/// writer wraps; the flag, not the response, is the observable.
async fn slow_work(done: Arc<AtomicBool>) -> Option<JsonRpcResponse> {
    tokio::time::sleep(WORK).await;
    done.store(true, Ordering::SeqCst);
    None
}

/// The point of the whole feature: a cancellation must stop work in flight.
#[tokio::test]
async fn cancelling_a_request_stops_the_work() {
    let registry = CancelRegistry::new();
    let writer = writer();
    let mut handlers = JoinSet::new();
    let done = Arc::new(AtomicBool::new(false));
    let id = json!(42);

    spawn_tracked(
        &mut handlers,
        &registry,
        &writer,
        Some(id.clone()),
        "tools/call",
        slow_work(Arc::clone(&done)),
    )
    .await;

    // The handler must be registered before it can be cancelled.
    assert_eq!(registry.inflight_count().await, 1);

    assert!(registry.cancel(&id).await, "handler should be registered");

    // Wait past the point the work would have finished on its own.
    tokio::time::sleep(PAST_WORK).await;
    assert!(
        !done.load(Ordering::SeqCst),
        "work continued after cancellation"
    );
    assert_eq!(
        registry.inflight_count().await,
        0,
        "cancelled handler left an entry behind"
    );

    // An aborted task must not stall shutdown.
    tokio::time::timeout(Duration::from_secs(2), async {
        while handlers.join_next().await.is_some() {}
    })
    .await
    .expect("drain stalled on an aborted task");
}

/// Control for the test above: without a cancellation the same work completes,
/// so an unset flag there is evidence of the abort and not of a slow test.
#[tokio::test]
async fn uncancelled_work_runs_to_completion() {
    let registry = CancelRegistry::new();
    let writer = writer();
    let mut handlers = JoinSet::new();
    let done = Arc::new(AtomicBool::new(false));

    spawn_tracked(
        &mut handlers,
        &registry,
        &writer,
        Some(json!(1)),
        "tools/call",
        slow_work(Arc::clone(&done)),
    )
    .await;

    while handlers.join_next().await.is_some() {}
    assert!(done.load(Ordering::SeqCst));
}

/// A completed handler must clean up after itself, or a long session leaks an
/// entry per request.
#[tokio::test]
async fn a_completed_request_leaves_no_registry_entry() {
    let registry = CancelRegistry::new();
    let writer = writer();
    let mut handlers = JoinSet::new();

    // Work that finishes immediately exercises the race the registry lock is
    // held to close: the task could otherwise try to deregister before the
    // parent had registered it, stranding a stale handle under that id.
    for i in 0..25 {
        spawn_tracked(
            &mut handlers,
            &registry,
            &writer,
            Some(json!(i)),
            "tools/call",
            async { None::<JsonRpcResponse> },
        )
        .await;
    }

    while handlers.join_next().await.is_some() {}
    assert_eq!(registry.inflight_count().await, 0);
}

/// Cancelling something that already finished (or never existed) is a normal
/// race, not an error.
#[tokio::test]
async fn cancelling_an_unknown_id_is_a_silent_no_op() {
    let registry = CancelRegistry::new();

    assert!(!registry.cancel(&json!(999)).await);
    assert!(!registry.cancel(&json!("never-seen")).await);
    assert_eq!(registry.inflight_count().await, 0);
}

/// The MCP spec forbids cancelling `initialize`; it is never registered, so a
/// cancellation naming it finds nothing to abort.
#[tokio::test]
async fn initialize_is_not_cancellable() {
    assert!(!is_cancellable("initialize"));
    assert!(is_cancellable("tools/call"));

    let registry = CancelRegistry::new();
    let writer = writer();
    let mut handlers = JoinSet::new();
    let done = Arc::new(AtomicBool::new(false));
    let id = json!(7);

    spawn_tracked(
        &mut handlers,
        &registry,
        &writer,
        Some(id.clone()),
        "initialize",
        slow_work(Arc::clone(&done)),
    )
    .await;

    assert_eq!(
        registry.inflight_count().await,
        0,
        "initialize must not be registered as cancellable"
    );
    assert!(!registry.cancel(&id).await);

    // It still runs to completion despite the cancellation attempt.
    while handlers.join_next().await.is_some() {}
    assert!(done.load(Ordering::SeqCst));
}

/// JSON-RPC treats the number `5` and the string `"5"` as different ids, so the
/// registry key must keep them apart.
#[tokio::test]
async fn numeric_and_string_ids_do_not_collide() {
    let registry = CancelRegistry::new();
    let writer = writer();
    let mut handlers = JoinSet::new();
    let numeric = Arc::new(AtomicBool::new(false));
    let string = Arc::new(AtomicBool::new(false));

    spawn_tracked(
        &mut handlers,
        &registry,
        &writer,
        Some(json!(5)),
        "tools/call",
        slow_work(Arc::clone(&numeric)),
    )
    .await;
    spawn_tracked(
        &mut handlers,
        &registry,
        &writer,
        Some(json!("5")),
        "tools/call",
        slow_work(Arc::clone(&string)),
    )
    .await;

    assert_eq!(registry.inflight_count().await, 2);

    // Cancelling the numeric id must leave the string id running.
    assert!(registry.cancel(&json!(5)).await);
    assert_eq!(registry.inflight_count().await, 1);

    while handlers.join_next().await.is_some() {}
    assert!(
        !numeric.load(Ordering::SeqCst),
        "numeric id was not stopped"
    );
    assert!(
        string.load(Ordering::SeqCst),
        "string id was wrongly stopped"
    );
}
