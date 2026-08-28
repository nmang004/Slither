use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlConfig {
    pub seed_url: String,
    pub max_depth: u32,
    pub max_pages: u32,
    pub concurrency: u32,
    pub delay_ms: u64,
    pub user_agent: String,
    pub ignore_robots: bool,
    pub follow_subdomains: bool,
    pub timeout_seconds: u64,
    pub generate_html: bool,
    pub output_path: Option<String>,
    pub json_compact: bool,
    pub include_body_text: bool,
    pub summary_only: bool,
    pub generate_csv: bool,
    pub backend: String,
    #[serde(skip)]
    pub cf_account_id: Option<String>,
    #[serde(skip)]
    pub cf_api_token: Option<String>,
    pub skip_header_check: bool,
    // Playwright backend
    #[serde(default)]
    pub chrome_path: Option<String>,
    #[serde(default = "default_true")]
    pub headless: bool,
    #[serde(default = "default_render_wait")]
    pub render_wait_ms: u64,
    // PageSpeed
    #[serde(default)]
    pub pagespeed: bool,
    #[serde(skip)]
    pub pagespeed_key: Option<String>,
    #[serde(default)]
    pub pagespeed_sample: Option<u32>,
    #[serde(default = "default_strategy")]
    pub pagespeed_strategy: String,
}

impl CrawlConfig {
    /// Clamp values that would otherwise make a crawl impossible or hang.
    ///
    /// `concurrency: 0` is the dangerous one: it builds a zero-permit
    /// semaphore whose first `acquire` never resolves, so the crawl hangs
    /// forever with no output. The CLI rejects these at parse time, but a
    /// config deserialized from a REST/MCP request reaches the crawler
    /// directly, so the invariant is enforced here too.
    pub fn sanitize(&mut self) {
        self.concurrency = self.concurrency.max(1);
        self.max_pages = self.max_pages.max(1);
        self.timeout_seconds = self.timeout_seconds.max(1);
    }

    /// A sanitized copy — convenience for call sites holding a `&CrawlConfig`.
    pub fn sanitized(&self) -> Self {
        let mut config = self.clone();
        config.sanitize();
        config
    }
}

fn default_true() -> bool {
    true
}
fn default_render_wait() -> u64 {
    2000
}
fn default_strategy() -> String {
    "mobile".to_string()
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            seed_url: String::new(),
            max_depth: 10,
            max_pages: 500,
            concurrency: 3,
            delay_ms: 250,
            user_agent: concat!("Slither/", env!("CARGO_PKG_VERSION"), " (SEO audit tool)")
                .to_string(),
            ignore_robots: false,
            follow_subdomains: false,
            timeout_seconds: 15,
            generate_html: true,
            output_path: None,
            json_compact: false,
            include_body_text: false,
            summary_only: false,
            generate_csv: false,
            backend: "local".to_string(),
            cf_account_id: None,
            cf_api_token: None,
            skip_header_check: false,
            chrome_path: None,
            headless: true,
            render_wait_ms: 2000,
            pagespeed: false,
            pagespeed_key: None,
            pagespeed_sample: None,
            pagespeed_strategy: "mobile".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CrawlConfig;

    /// Regression: `concurrency: 0` built a zero-permit semaphore and the
    /// crawl hung forever instead of failing or running.
    #[test]
    fn sanitize_raises_zero_concurrency_to_one() {
        let mut config = CrawlConfig {
            concurrency: 0,
            ..Default::default()
        };
        config.sanitize();
        assert_eq!(config.concurrency, 1);
    }

    #[test]
    fn sanitize_raises_other_zero_values() {
        let mut config = CrawlConfig {
            max_pages: 0,
            timeout_seconds: 0,
            ..Default::default()
        };
        config.sanitize();
        assert_eq!(config.max_pages, 1);
        assert_eq!(config.timeout_seconds, 1);
    }

    #[test]
    fn sanitize_leaves_valid_values_alone() {
        let mut config = CrawlConfig {
            concurrency: 8,
            max_pages: 500,
            timeout_seconds: 30,
            ..Default::default()
        };
        config.sanitize();
        assert_eq!(config.concurrency, 8);
        assert_eq!(config.max_pages, 500);
        assert_eq!(config.timeout_seconds, 30);
    }

    #[test]
    fn sanitized_returns_a_corrected_copy() {
        let config = CrawlConfig {
            concurrency: 0,
            ..Default::default()
        };
        assert_eq!(config.sanitized().concurrency, 1);
        assert_eq!(config.concurrency, 0, "original is left untouched");
    }
}
