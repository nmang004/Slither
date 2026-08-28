pub mod extract;
pub mod render;
pub mod screenshot;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

/// Cloudflare Browser Rendering API client.
#[derive(Clone)]
pub struct CloudflareClient {
    account_id: String,
    http: reqwest::Client,
}

// Manual Debug so the bearer token stored in the reqwest client's default
// headers can never be printed by a `{:?}` / tracing::debug! of this struct.
impl std::fmt::Debug for CloudflareClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudflareClient")
            .field("account_id", &self.account_id)
            .field("http", &"<reqwest::Client>")
            .finish()
    }
}

/// Errors from Cloudflare API interactions.
#[derive(Debug, thiserror::Error)]
pub enum CloudflareError {
    #[error("Cloudflare auth failed. Run `slither setup cloudflare` to reconfigure.")]
    AuthFailed,

    #[error("Daily browser time limit reached (10 min). Resets at midnight UTC. Run locally with: slither crawl <url>")]
    FreeTierExhausted,

    #[error("Cloudflare API unreachable. Run locally with: slither crawl <url>")]
    ApiUnreachable,

    #[error("Crawl timed out. Try reducing --max-pages.")]
    CrawlTimeout,

    #[error("Rate limited by Cloudflare. Retrying...")]
    RateLimited,

    #[error("Cloudflare API error: {0}")]
    ApiError(String),

    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

impl CloudflareClient {
    /// Create a new client from explicit credentials or environment variables.
    /// Returns None if credentials are not available.
    pub fn new(account_id: Option<String>, api_token: Option<String>) -> Option<Self> {
        let account_id = account_id
            .or_else(|| std::env::var("CLOUDFLARE_ACCOUNT_ID").ok())
            .filter(|s| !s.is_empty())?;
        let api_token = api_token
            .or_else(|| std::env::var("CLOUDFLARE_API_TOKEN").ok())
            .filter(|s| !s.is_empty())?;

        let mut headers = HeaderMap::new();
        let mut val = HeaderValue::from_str(&format!("Bearer {}", api_token)).ok()?;
        // Mark the token sensitive so reqwest/hyper never log it.
        val.set_sensitive(true);
        headers.insert(AUTHORIZATION, val);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .ok()?;

        Some(Self { account_id, http })
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn base_url(&self) -> String {
        format!(
            "https://api.cloudflare.com/client/v4/accounts/{}/browser-rendering",
            self.account_id
        )
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    /// User-friendly message when CF is not configured.
    pub fn not_configured_message() -> String {
        [
            "",
            "  This feature requires Cloudflare Browser Rendering",
            "  (free tier: 10 min/day).",
            "",
            "  Get started in ~2 minutes:",
            "    1. Sign up at cloudflare.com (free)",
            "    2. Run: slither setup cloudflare",
            "",
        ]
        .join("\n")
    }
}
