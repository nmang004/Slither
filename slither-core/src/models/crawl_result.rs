use crate::models::config::CrawlConfig;
use crate::models::page::PageData;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportSettings {
    pub include_body_text: bool,
    pub summary_only: bool,
    pub format: String,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            include_body_text: false,
            summary_only: false,
            format: "pretty".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlResult {
    pub slither_version: String,
    pub crawl_metadata: CrawlMetadata,
    pub export_settings: ExportSettings,
    pub pages: Vec<PageData>,
    pub issues: CrawlIssues,
    pub summary: CrawlSummary,
    /// Raw robots.txt content, retained for AI-crawler policy analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub robots_txt: Option<String>,
    /// Sitemap discovery results, retained so a re-run of the pipeline (the
    /// PageSpeed pass, the server executor) keeps sitemap coverage analysis
    /// instead of silently dropping it. `None` means discovery never ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sitemap_data: Option<crate::crawler::sitemaps::SitemapData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlMetadata {
    pub domain: String,
    pub seed_url: String,
    pub crawl_date: String,
    pub duration_ms: u64,
    pub pages_discovered: u32,
    pub pages_crawled: u32,
    pub pages_skipped_robots: u32,
    pub pages_errored: u32,
    pub settings: CrawlConfig,
    pub backend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlIssues {
    pub issues: Vec<crate::models::issue::Issue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlSummary {
    pub total_pages: u32,
    pub by_status: HashMap<String, u32>,
    pub avg_response_time_ms: u32,
    pub avg_word_count: u32,
    pub total_internal_links: u32,
    pub total_external_links: u32,
    pub total_images: u32,
    pub images_without_alt: u32,
    pub pages_with_schema: u32,
    pub total_issues: u32,
    pub critical_issues: u32,
    pub warning_issues: u32,
    pub info_issues: u32,
    pub issues_by_category: HashMap<String, CategorySummary>,
    pub health_score: u32,
    pub grade: String,
    pub grade_verdict: String,
    pub response_time_p50_ms: u32,
    pub response_time_p95_ms: u32,
    #[serde(default)]
    pub cwv_pages_tested: u32,
    #[serde(default)]
    pub cwv_pages_good: u32,
    #[serde(default)]
    pub cwv_pages_needs_work: u32,
    #[serde(default)]
    pub cwv_pages_poor: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_lcp_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_inp_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_cls: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_performance_score: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategorySummary {
    pub total_checks: u32,
    pub issues_found: u32,
    pub affected_urls: u32,
    pub critical: u32,
    pub warning: u32,
    pub info: u32,
}
