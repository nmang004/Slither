pub mod config;
pub mod crawl_result;
pub mod issue;
pub mod page;

use crate::models::config::CrawlConfig;
use crate::models::crawl_result::CrawlResult;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
// Event variants naturally vary in size; boxing every large field would add
// noise for a short-lived, low-volume progress channel.
#[allow(clippy::large_enum_variant)]
pub enum CrawlEvent {
    Started {
        domain: String,
        settings: CrawlConfig,
    },
    RobotsFetched {
        rules_count: usize,
        crawl_delay: Option<u64>,
    },
    PageCrawled {
        url: String,
        status: u16,
        response_time_ms: u64,
    },
    PageError {
        url: String,
        error: String,
    },
    QueueUpdate {
        crawled: usize,
        queued: usize,
        estimated_total: usize,
    },
    Completed {
        result: Box<CrawlResult>,
    },
}
