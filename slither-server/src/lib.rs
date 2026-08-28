pub mod api;
pub mod auth;
pub mod error;
pub mod executor;
pub mod mcp;
pub mod params;

use std::sync::Arc;

use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use slither_core::jobs::{self, db, JobManager};

/// Build a CORS layer from `SLITHER_CORS_ORIGINS` (comma-separated allowlist).
///
/// Default is **no cross-origin access**: without this, a permissive
/// `allow_origin(Any)` let any web page the user visits drive the local API
/// (create jobs → SSRF → read/delete results) since preflight for a JSON POST
/// would succeed. Returning `None` means same-origin only.
fn cors_layer_from_env() -> Option<CorsLayer> {
    let raw = std::env::var("SLITHER_CORS_ORIGINS").ok()?;
    let origins: Vec<http::HeaderValue> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<http::HeaderValue>().ok())
        .collect();
    if origins.is_empty() {
        return None;
    }
    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([http::Method::GET, http::Method::POST, http::Method::DELETE])
            .allow_headers([http::header::AUTHORIZATION, http::header::CONTENT_TYPE]),
    )
}

// ---------------------------------------------------------------------------
// Server configuration — passed in by the caller instead of relying on env vars
// ---------------------------------------------------------------------------

pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub api_key: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3001,
            api_key: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Global API key (set once during startup, read by auth middleware)
// ---------------------------------------------------------------------------

pub(crate) static API_KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();

// ---------------------------------------------------------------------------
// Application state shared across all handlers
// ---------------------------------------------------------------------------

pub struct AppState {
    pub job_manager: JobManager,
    pub crawl_semaphore: Arc<tokio::sync::Semaphore>,
}

// ---------------------------------------------------------------------------
// Public entry-point — usable from both the standalone binary and slither CLI
// ---------------------------------------------------------------------------

pub async fn run(mcp_mode: bool, config: ServerConfig) -> anyhow::Result<()> {
    // Store the API key in the global OnceLock so auth middleware can read it.
    if let Some(key) = config.api_key {
        let _ = API_KEY.set(key);
    }

    // Initialise tracing (respects RUST_LOG; defaults to info).
    // In MCP mode, logs MUST go to stderr — stdout is reserved for JSON-RPC.
    if mcp_mode {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
            .init();
    }

    // Ensure jobs directory exists.
    let jobs_dir = jobs::jobs_dir();
    std::fs::create_dir_all(&jobs_dir)?;

    // Open (or create) the SQLite database.
    let db_path = jobs::db_path();
    let conn = db::open_db(&db_path)?;

    // Build the job manager. Registering as an owner happens here, so every job
    // this process creates is stamped with it.
    let job_manager = JobManager::new(conn, jobs_dir);

    // Max concurrent crawls, configurable via SLITHER_MAX_CONCURRENT_CRAWLS.
    let max_concurrent = std::env::var("SLITHER_MAX_CONCURRENT_CRAWLS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(3);
    let crawl_semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
    let state = Arc::new(AppState {
        job_manager,
        crawl_semaphore,
    });

    // Publish liveness before doing anything else, so a concurrent instance
    // never sees this process as a dead owner.
    spawn_heartbeat(Arc::clone(&state));

    // Reclaim jobs abandoned by processes that are gone — at startup, and then
    // on a timer so a crash that happens *after* startup is also cleaned up
    // rather than leaving rows that hold queue slots forever.
    reclaim_orphaned_jobs(&state).await;
    spawn_reclaim_loop(Arc::clone(&state));

    // Warn once at startup if no API key is configured.
    auth::warn_if_unauthenticated();

    // MCP stdio mode — no HTTP listener.
    if mcp_mode {
        return mcp::run_mcp_server(state).await;
    }

    let app = build_router(Arc::clone(&state));

    // Resolve host / port from config.
    let host = config.host;
    let port = config.port;

    // Refuse to expose an unauthenticated API on a non-loopback interface unless
    // the operator explicitly opts in. Loopback stays easy for local-first use.
    let is_loopback = slither_core::net_guard::is_loopback_name(&host)
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);
    let anon_allowed = matches!(
        std::env::var("SLITHER_ALLOW_ANONYMOUS").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE")
    );
    if API_KEY.get().is_none() && !is_loopback && !anon_allowed {
        anyhow::bail!(
            "Refusing to start an unauthenticated API on non-loopback host '{host}'. \
             Set --api-key / SLITHER_API_KEY, or set SLITHER_ALLOW_ANONYMOUS=1 to override."
        );
    }

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // Print startup banner.
    println!();
    println!("  Slither Server v{}", env!("CARGO_PKG_VERSION"));
    println!("  Listening on http://{addr}");
    println!();

    tracing::info!("slither-server listening on {addr}");
    axum::serve(listener, app).await?;

    Ok(())
}

/// How often the reclaim pass runs after startup.
///
/// Recovery used to happen only at startup, which was tolerable when it failed
/// everything unconditionally. Now that it only takes work from owners that are
/// provably gone, a crashed peer's jobs become reclaimable a couple of minutes
/// later — so something has to look again.
const RECLAIM_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Refresh this process's ownership heartbeat forever.
///
/// Deliberately a plain OS thread, not a tokio task: a missed heartbeat is what
/// tells another process that these jobs are abandoned, so it must not be
/// possible for application work on the runtime to starve it.
fn spawn_heartbeat(state: Arc<AppState>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(slither_core::jobs::manager::HEARTBEAT_INTERVAL);
        if let Err(e) = state.job_manager.heartbeat() {
            tracing::warn!("failed to refresh job ownership heartbeat: {e}");
        }
    });
}

/// Fail the jobs whose owning process is gone and notify webhooks for them.
///
/// Firing here is what keeps a one-shot `webhook_url` from being stranded at
/// `fired = 0` forever when the process that was going to deliver it died.
async fn reclaim_orphaned_jobs(state: &Arc<AppState>) {
    match state.job_manager.recover_orphaned_jobs() {
        Ok(ids) if !ids.is_empty() => {
            tracing::warn!(
                count = ids.len(),
                "reclaimed jobs whose owning process is no longer running"
            );
            for id in ids {
                executor::fire_terminal_webhook(state, &id).await;
            }
        }
        Ok(_) => {}
        Err(e) => tracing::error!("failed to check for orphaned jobs: {e}"),
    }
}

fn spawn_reclaim_loop(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(RECLAIM_INTERVAL);
        // The first tick completes immediately; startup already ran a pass.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            reclaim_orphaned_jobs(&state).await;
        }
    });
}

/// Assemble the HTTP router around a prepared [`AppState`].
///
/// Kept separate from [`run`] so the routes — and the auth, CORS and body-limit
/// layers wrapped around them — can be exercised without binding a socket.
pub fn build_router(state: Arc<AppState>) -> Router {
    // Health endpoint lives OUTSIDE the auth layer so that load-balancers and
    // monitoring can always reach it.
    let authed_routes = Router::new()
        .route("/api/v1/jobs", axum::routing::post(api::jobs::create_job))
        .route("/api/v1/jobs", axum::routing::get(api::jobs::list_jobs))
        .route("/api/v1/jobs/{id}", axum::routing::get(api::jobs::get_job))
        .route(
            "/api/v1/jobs/{id}",
            axum::routing::delete(api::jobs::delete_job),
        )
        .route(
            "/api/v1/jobs/{id}/results/{filename}",
            axum::routing::get(api::jobs::get_result_file),
        )
        .route(
            "/api/v1/webhooks",
            axum::routing::post(api::webhooks::create_webhook),
        )
        .route(
            "/api/v1/webhooks",
            axum::routing::get(api::webhooks::list_webhooks),
        )
        .route(
            "/api/v1/webhooks/{id}",
            axum::routing::delete(api::webhooks::delete_webhook),
        )
        .layer(axum::middleware::from_fn(auth::auth_middleware));

    let mut app = Router::new()
        .route("/api/v1/health", axum::routing::get(api::health::health))
        .merge(authed_routes);

    // CORS is default-closed; only add a layer when an explicit allowlist is set.
    if let Some(cors) = cors_layer_from_env() {
        app = app.layer(cors);
    }

    app.layer(RequestBodyLimitLayer::new(1_048_576)) // 1 MB
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
