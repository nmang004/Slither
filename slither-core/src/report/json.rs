use crate::models::crawl_result::CrawlResult;
use crate::models::page::PageData;
use anyhow::{Context, Result};

/// Serialize a CrawlResult to JSON, respecting export settings.
pub fn serialize_crawl_result(result: &CrawlResult) -> Result<String> {
    // When nothing needs rewriting, serialize the caller's value directly —
    // cloning the whole result (page bodies included) just to serialize it
    // doubles peak memory on a large crawl.
    let include_body = result.export_settings.include_body_text;
    let summary_only = result.export_settings.summary_only;

    if include_body && !summary_only {
        return finish(result, result.export_settings.format == "compact");
    }

    // Otherwise only the pages array changes. Build it without holding a
    // second copy of every page body: each clone's `body_text` is released
    // immediately, so the extra memory is one body rather than all of them.
    let pages: Vec<PageData> = if summary_only {
        Vec::new()
    } else {
        result
            .pages
            .iter()
            .map(|page| {
                let mut page = page.clone();
                page.body_text = String::new();
                page
            })
            .collect()
    };

    let output = CrawlResult {
        slither_version: result.slither_version.clone(),
        crawl_metadata: result.crawl_metadata.clone(),
        export_settings: result.export_settings.clone(),
        pages,
        issues: result.issues.clone(),
        summary: result.summary.clone(),
        robots_txt: result.robots_txt.clone(),
        sitemap_data: result.sitemap_data.clone(),
    };

    finish(&output, output.export_settings.format == "compact")
}

fn finish(result: &CrawlResult, compact: bool) -> Result<String> {
    let json = if compact {
        serde_json::to_string(result)
    } else {
        serde_json::to_string_pretty(result)
    };

    json.context("Failed to serialize crawl result to JSON")
}
