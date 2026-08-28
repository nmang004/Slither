pub mod api;
pub mod models;

use std::time::Instant;
use tokio::sync::Mutex;

/// Rate-limited HTTP client for PageSpeed Insights API.
pub struct PageSpeedClient {
    http: reqwest::Client,
    api_key: Option<String>,
    strategy: String,
    request_log: Mutex<Vec<Instant>>,
}

impl PageSpeedClient {
    pub fn new(api_key: Option<String>, strategy: String) -> Self {
        Self {
            // A single PSI request can take 20–30s; bound it so one hung
            // request cannot stall the whole enrichment run.
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
            api_key,
            strategy,
            request_log: Mutex::new(Vec::new()),
        }
    }

    fn rate_limit(&self) -> usize {
        if self.api_key.is_some() {
            400
        } else {
            25
        }
    }

    pub async fn wait_for_rate_limit(&self) {
        let window = std::time::Duration::from_secs(100);
        loop {
            let now = Instant::now();
            {
                let mut log = self.request_log.lock().await;
                log.retain(|t| now.duration_since(*t) < window);
                if log.len() < self.rate_limit() {
                    log.push(now);
                    return;
                }
            } // guard dropped before await
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    pub fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    pub fn strategy(&self) -> &str {
        &self.strategy
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
}
