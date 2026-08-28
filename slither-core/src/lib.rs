pub mod analysis;
pub mod crawler;
/// Single-page audit. Static inspection needs no third-party service; the
/// rendered and compare modes are gated behind the `cloudflare` feature.
pub mod inspect;
pub mod jobs;
pub mod link_graph;
pub mod models;
pub mod net_guard;
pub mod output;
pub mod pipeline;
pub mod report;
pub mod utils;

#[cfg(feature = "cloudflare")]
pub mod cloudflare;

#[cfg(feature = "playwright")]
pub mod playwright;

pub mod pagespeed;

pub use models::config::CrawlConfig;
pub use models::crawl_result::CrawlResult;
pub use models::page::PageData;
pub use models::CrawlEvent;
pub use pipeline::{enrich_pagespeed, run_post_crawl_pipeline, PipelineInput};
