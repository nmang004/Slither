pub mod tools;
pub mod transport;

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::task::{AbortHandle, JoinSet};

use crate::AppState;
use transport::{JsonRpcRequest, JsonRpcResponse, StdioReader, StdioWriter};

/// Tracks in-flight request handlers so `notifications/cancelled` can abort
/// the work rather than merely acknowledging the intent.
///
/// Entries are keyed by the JSON rendering of the request id. JSON-RPC ids may
/// be strings or numbers and the two are distinct identities, which
/// `Value::to_string` preserves — `5` renders as `5` and `"5"` as `"5"`, so
/// they cannot collide.
///
/// Each entry carries a monotonic sequence number alongside the handle. A
/// misbehaving client can reuse an id while the first request is still running;
/// the sequence lets a finishing handler recognise that the entry under its key
/// now belongs to a newer request and leave it alone.
#[derive(Clone, Default)]
pub struct CancelRegistry {
    inflight: Arc<Mutex<HashMap<String, (u64, AbortHandle)>>>,
    next_seq: Arc<AtomicU64>,
}

impl CancelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn key(id: &Value) -> String {
        id.to_string()
    }

    /// Abort the handler for `id`, if one is still running. Returns whether a
    /// handler was found.
    ///
    /// The entry is removed here rather than by the aborted task: `abort` drops
    /// the future at its next await point, so the task's own deregistration
    /// never runs.
    pub async fn cancel(&self, id: &Value) -> bool {
        let mut inflight = self.inflight.lock().await;
        match inflight.remove(&Self::key(id)) {
            Some((_, handle)) => {
                handle.abort();
                true
            }
            None => false,
        }
    }

    /// Number of handlers currently tracked — used by tests to prove entries do
    /// not leak.
    pub async fn inflight_count(&self) -> usize {
        self.inflight.lock().await.len()
    }
}

/// Whether a request may be cancelled.
///
/// The MCP spec forbids cancelling `initialize`; such a request is never
/// registered, so a cancellation naming it finds nothing and is ignored.
pub fn is_cancellable(method: &str) -> bool {
    method != "initialize"
}

/// Spawn one handler, registering its abort handle so a cancellation can stop
/// it, and write whatever response it produces.
///
/// Two ordering constraints are load-bearing:
///
/// 1. The registry lock is held across both the spawn and the insert. The task
///    deregisters through the same lock, so it cannot remove an entry that has
///    not been inserted yet — otherwise a fast handler could leave a stale
///    handle behind that a later cancellation would abort in place of the live
///    request holding that id.
/// 2. The handler deregisters *before* writing. Once the work is finished the
///    response is committed, and aborting mid-write would truncate the line and
///    desynchronise the stream. Cancellation therefore stops work in progress,
///    never a response already being emitted.
pub async fn spawn_tracked<F>(
    handlers: &mut JoinSet<()>,
    registry: &CancelRegistry,
    writer: &Arc<Mutex<StdioWriter>>,
    id: Option<Value>,
    method: &str,
    work: F,
) where
    F: Future<Output = Option<JsonRpcResponse>> + Send + 'static,
{
    let tracked_key = match &id {
        Some(id) if is_cancellable(method) => Some(CancelRegistry::key(id)),
        // Notifications carry no id, and `initialize` must stay uncancellable.
        _ => None,
    };

    let Some(key) = tracked_key else {
        let writer = Arc::clone(writer);
        handlers.spawn(async move {
            if let Some(resp) = work.await {
                write_response(&writer, &resp).await;
            }
        });
        return;
    };

    let seq = registry.next_seq.fetch_add(1, Ordering::Relaxed);
    let inflight = Arc::clone(&registry.inflight);
    let writer = Arc::clone(writer);
    let task_key = key.clone();

    let mut map = registry.inflight.lock().await;
    let handle = handlers.spawn(async move {
        let resp = work.await;

        {
            let mut inflight = inflight.lock().await;
            if inflight.get(&task_key).is_some_and(|(s, _)| *s == seq) {
                inflight.remove(&task_key);
            }
        }

        if let Some(resp) = resp {
            write_response(&writer, &resp).await;
        }
    });
    map.insert(key, (seq, handle));
}

/// Extract the `requestId` from a `notifications/cancelled` payload.
fn cancelled_request_id(params: Option<&Value>) -> Option<Value> {
    params?.get("requestId").cloned()
}

/// The protocol revision that introduced `structuredContent`. Clients that
/// negotiate this or later read the structured payload directly, so shipping a
/// second escaped copy of it in a text block is pure waste.
const STRUCTURED_CONTENT_REVISION: &str = "2025-06-18";

/// Per-connection MCP state.
///
/// Only what the client told us at `initialize` and we need to honor for the
/// rest of the connection. Session-scoped rather than global so concurrent
/// tests (and any future non-stdio transport) cannot see each other's
/// negotiation.
#[derive(Debug)]
pub struct McpSession {
    /// Whether to include the backwards-compatibility text copy of a tool's
    /// JSON result. Defaults to `true`: until the client has told us otherwise,
    /// assume the older shape, since sending too much is recoverable and
    /// sending too little is not.
    duplicate_result_as_text: AtomicBool,
}

impl McpSession {
    pub fn new() -> Self {
        Self {
            duplicate_result_as_text: AtomicBool::new(true),
        }
    }

    /// Record the negotiated revision, which decides the result shape.
    fn set_negotiated_version(&self, version: &str) {
        // Revisions are ISO dates, so lexicographic order is chronological.
        let understands_structured = version >= STRUCTURED_CONTENT_REVISION;
        self.duplicate_result_as_text
            .store(!understands_structured, Ordering::Relaxed);
    }

    fn duplicates_result_as_text(&self) -> bool {
        self.duplicate_result_as_text.load(Ordering::Relaxed)
    }
}

impl Default for McpSession {
    fn default() -> Self {
        Self::new()
    }
}

/// A one-line stand-in for the full JSON, for clients that read
/// `structuredContent` and do not need the payload twice.
///
/// The spec says a tool returning structured content SHOULD also return text.
/// An empty content array satisfies the letter of that while giving a human
/// reading the transcript nothing, so summarize instead: top-level scalars
/// verbatim and collections by size, which is what someone skimming actually
/// wants.
fn compact_result_summary(tool: &str, value: &Value) -> String {
    const MAX_STRING: usize = 60;
    const MAX_PARTS: usize = 12;

    let Some(obj) = value.as_object() else {
        return format!("{tool}: see structuredContent");
    };

    let mut parts: Vec<String> = Vec::new();
    for (key, val) in obj.iter().take(MAX_PARTS) {
        let rendered = match val {
            Value::Array(items) => format!("{key}=[{} items]", items.len()),
            Value::Object(fields) => format!("{key}={{{} fields}}", fields.len()),
            Value::String(s) if s.chars().count() > MAX_STRING => {
                let truncated: String = s.chars().take(MAX_STRING).collect();
                format!("{key}=\"{truncated}…\"")
            }
            other => format!("{key}={other}"),
        };
        parts.push(rendered);
    }
    if obj.len() > MAX_PARTS {
        parts.push(format!("… {} more fields", obj.len() - MAX_PARTS));
    }

    format!(
        "{tool}: {} — full data in structuredContent",
        parts.join(", ")
    )
}

/// Run the MCP (Model Context Protocol) server over stdio.
///
/// Reads JSON-RPC 2.0 requests line-by-line from stdin and writes responses
/// to stdout. This replaces the HTTP server when `--mcp` is passed.
///
/// Each message is handled in its own task so a slow tool call (a rendered
/// `slither_inspect` or a screenshot can take tens of seconds) does not stall
/// `ping` or any other request behind it. Responses may therefore complete out
/// of order — legal JSON-RPC, since `id` correlates request to response — but
/// each one is written as a single complete line under the writer lock.
///
/// Every cancellable handler's abort handle is tracked in a [`CancelRegistry`],
/// so `notifications/cancelled` stops the work rather than merely acknowledging
/// the request.
pub async fn run_mcp_server(state: Arc<AppState>) -> anyhow::Result<()> {
    tracing::info!("MCP server starting on stdio");

    let mut reader = StdioReader::new();
    let writer = Arc::new(Mutex::new(StdioWriter::new()));
    let mut handlers = JoinSet::new();
    let registry = CancelRegistry::new();
    let session = Arc::new(McpSession::new());

    loop {
        // Reap finished handlers so a long session cannot accumulate completed
        // join handles without bound.
        while handlers.try_join_next().is_some() {}

        let line = match reader.read_message().await {
            Ok(Some(line)) => line,
            Ok(None) => {
                tracing::info!("stdin closed, MCP server shutting down");
                break;
            }
            Err(e) => {
                // An over-long or unreadable message leaves the stream out of
                // sync; there is no safe way to resume framing.
                tracing::error!("fatal MCP transport error: {e}");
                break;
            }
        };

        let messages = match parse_messages(&line) {
            Ok(messages) => messages,
            Err(resp) => {
                write_response(&writer, &resp).await;
                continue;
            }
        };

        for message in messages {
            let req = match to_request(message) {
                Ok(req) => req,
                Err(resp) => {
                    write_response(&writer, &resp).await;
                    continue;
                }
            };

            // Cancellation is handled inline rather than spawned: it must take
            // effect immediately, and queueing it behind a spawn would let the
            // work it targets run on.
            if req.method == "notifications/cancelled" {
                match cancelled_request_id(req.params.as_ref()) {
                    Some(id) => {
                        if registry.cancel(&id).await {
                            tracing::debug!("cancelled in-flight request {id}");
                        } else {
                            // An unknown or already-finished id is a legitimate
                            // race, not an error, and a notification is never
                            // answered either way.
                            tracing::debug!("cancellation for {id}: nothing in flight");
                        }
                    }
                    None => tracing::debug!("cancellation without a requestId"),
                }
                continue;
            }

            let state = Arc::clone(&state);
            let session = Arc::clone(&session);
            let id = req.id.clone();
            let method = req.method.clone();
            spawn_tracked(&mut handlers, &registry, &writer, id, &method, async move {
                // Notifications (no id) MUST NOT receive a response per JSON-RPC spec
                handle_request(&state, &session, &req).await
            })
            .await;
        }
    }

    // Drain outstanding handlers so their responses are written before exit
    // rather than being dropped with the runtime.
    while handlers.join_next().await.is_some() {}

    Ok(())
}

/// Write one response, holding the writer lock across the whole line so
/// concurrent handlers cannot interleave mid-line.
async fn write_response(writer: &Arc<Mutex<StdioWriter>>, resp: &JsonRpcResponse) {
    let mut writer = writer.lock().await;
    if let Err(e) = writer.write_response(resp).await {
        tracing::error!("failed to write MCP response: {e}");
    }
}

/// Split a raw input line into the messages it carries.
///
/// Parsing is lenient so malformed input gets a JSON-RPC parse error rather
/// than silence — a client that gets no reply waits forever. A top-level array
/// is a batch; its members are returned individually, which the
/// newline-delimited framing handles naturally.
fn parse_messages(line: &str) -> Result<Vec<Value>, Box<JsonRpcResponse>> {
    let value: Value = serde_json::from_str(line).map_err(|e| {
        tracing::warn!("malformed JSON-RPC message: {e}");
        Box::new(JsonRpcResponse::error(
            Some(Value::Null),
            PARSE_ERROR,
            format!("Parse error: {e}"),
        ))
    })?;

    match value {
        Value::Array(items) if items.is_empty() => Err(Box::new(JsonRpcResponse::error(
            Some(Value::Null),
            INVALID_REQUEST,
            "Invalid Request: empty batch".to_string(),
        ))),
        Value::Array(items) => Ok(items),
        other => Ok(vec![other]),
    }
}

/// Deserialize one message, preserving its id (when present) so an unparseable
/// member can still be answered against the right request.
fn to_request(message: Value) -> Result<JsonRpcRequest, Box<JsonRpcResponse>> {
    let id = message.get("id").cloned();
    serde_json::from_value(message).map_err(|e| {
        Box::new(JsonRpcResponse::error(
            Some(id.unwrap_or(Value::Null)),
            INVALID_REQUEST,
            format!("Invalid Request: {e}"),
        ))
    })
}

/// Dispatch one raw input line and collect the responses it produces.
///
/// This is the sequential composition of the pieces the server loop drives
/// concurrently, and is the entry point for driving the protocol without a live
/// stdio session.
pub async fn dispatch_line(state: &Arc<AppState>, line: &str) -> Vec<JsonRpcResponse> {
    dispatch_line_in_session(state, &McpSession::new(), line).await
}

/// As [`dispatch_line`], but against a caller-owned session, so a test can
/// drive `initialize` and a subsequent `tools/call` through the same negotiated
/// state the stdio loop would give them.
pub async fn dispatch_line_in_session(
    state: &Arc<AppState>,
    session: &McpSession,
    line: &str,
) -> Vec<JsonRpcResponse> {
    let messages = match parse_messages(line) {
        Ok(messages) => messages,
        Err(resp) => return vec![*resp],
    };

    let mut responses = Vec::new();
    for message in messages {
        match to_request(message) {
            Ok(req) => {
                if let Some(resp) = handle_request(state, session, &req).await {
                    responses.push(resp);
                }
            }
            Err(resp) => responses.push(*resp),
        }
    }
    responses
}

/// JSON-RPC 2.0 reserved error codes.
const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;

async fn handle_request(
    state: &Arc<AppState>,
    session: &McpSession,
    req: &JsonRpcRequest,
) -> Option<JsonRpcResponse> {
    match req.method.as_str() {
        // -----------------------------------------------------------------
        // Handshake
        // -----------------------------------------------------------------
        "initialize" => {
            // Protocol-version negotiation: echo the client's requested version
            // when we support it, otherwise fall back to our latest. Hardcoding a
            // single version ignored what the client asked for.
            const SUPPORTED: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
            let requested = req
                .params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str());
            let negotiated = match requested {
                Some(v) if SUPPORTED.contains(&v) => v,
                _ => SUPPORTED[0],
            };
            session.set_negotiated_version(negotiated);

            let result = json!({
                "protocolVersion": negotiated,
                // Advertise only capabilities we actually implement.
                "capabilities": {
                    "tools": { "listChanged": false },
                    "resources": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "slither",
                    "version": env!("CARGO_PKG_VERSION")
                }
            });
            Some(JsonRpcResponse::success(req.id.clone(), result))
        }

        // Notifications — no id, no response.
        "notifications/initialized" => None,

        // The stdio loop intercepts cancellation before dispatch so it can abort
        // the registered handler. Reaching here means the sequential path, which
        // runs one request at a time and so has nothing in flight to stop.
        "notifications/cancelled" => {
            let request_id = cancelled_request_id(req.params.as_ref()).unwrap_or(Value::Null);
            tracing::debug!("cancellation for {request_id} on the sequential path: no-op");
            None
        }

        // -----------------------------------------------------------------
        // Ping — required by spec, must respond promptly
        // -----------------------------------------------------------------
        "ping" => Some(JsonRpcResponse::success(req.id.clone(), json!({}))),

        // -----------------------------------------------------------------
        // Tools
        // -----------------------------------------------------------------
        "tools/list" => {
            let defs = tools::tool_definitions();
            Some(JsonRpcResponse::success(
                req.id.clone(),
                json!({ "tools": defs }),
            ))
        }

        "tools/call" => {
            let params = req.params.clone().unwrap_or(json!({}));
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));

            match tools::call_tool(state, name, args).await {
                Ok(text) => {
                    // Tools return serialized JSON. It goes in structuredContent
                    // per the current spec; the text block carries the same JSON
                    // again only for clients too old to read structuredContent,
                    // since that copy is otherwise half the response.
                    let structured = serde_json::from_str::<Value>(&text).ok();
                    let text_block = match &structured {
                        Some(value) if !session.duplicates_result_as_text() => {
                            compact_result_summary(name, value)
                        }
                        _ => text,
                    };
                    let mut result = json!({
                        "content": [{ "type": "text", "text": text_block }],
                        "isError": false
                    });
                    if let Some(structured) = structured {
                        result["structuredContent"] = structured;
                    }
                    Some(JsonRpcResponse::success(req.id.clone(), result))
                }
                Err(msg) => Some(JsonRpcResponse::success(
                    req.id.clone(),
                    json!({
                        "content": [{ "type": "text", "text": msg }],
                        "isError": true
                    }),
                )),
            }
        }

        // -----------------------------------------------------------------
        // Resources — crawl artifacts (crawl.json, report.html, export.csv)
        // -----------------------------------------------------------------
        "resources/list" => Some(JsonRpcResponse::success(
            req.id.clone(),
            json!({ "resources": tools::list_resources(state) }),
        )),

        "resources/read" => {
            let uri = req
                .params
                .as_ref()
                .and_then(|p| p.get("uri"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match tools::read_resource(state, uri) {
                Ok(contents) => Some(JsonRpcResponse::success(
                    req.id.clone(),
                    json!({ "contents": [contents] }),
                )),
                Err(msg) => Some(JsonRpcResponse::error(req.id.clone(), -32602, msg)),
            }
        }

        // -----------------------------------------------------------------
        // Unknown
        // -----------------------------------------------------------------
        _ => {
            // Ignore unknown notifications (no id) silently
            req.id.as_ref()?;
            tracing::warn!(method = %req.method, "unknown MCP method");
            Some(JsonRpcResponse::error(
                req.id.clone(),
                -32601,
                format!("Method not found: {}", req.method),
            ))
        }
    }
}
