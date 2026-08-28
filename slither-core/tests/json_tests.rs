use slither_core::models::config::CrawlConfig;
use slither_core::models::crawl_result::{
    CrawlIssues, CrawlMetadata, CrawlResult, CrawlSummary, ExportSettings,
};
use slither_core::models::page::SecurityHeaders;
use std::collections::HashMap;

fn make_test_result() -> CrawlResult {
    CrawlResult {
        slither_version: "0.1.0".to_string(),
        crawl_metadata: CrawlMetadata {
            domain: "test.com".to_string(),
            seed_url: "https://test.com".to_string(),
            crawl_date: "2026-03-01T00:00:00Z".to_string(),
            duration_ms: 1000,
            pages_discovered: 1,
            pages_crawled: 1,
            pages_skipped_robots: 0,
            pages_errored: 0,
            settings: CrawlConfig::default(),
            backend: "local".to_string(),
        },
        export_settings: ExportSettings::default(),
        pages: vec![slither_core::PageData {
            url: "https://test.com/".to_string(),
            status: 200,
            body_text: "This is body text that should be excluded by default.".to_string(),
            word_count: 10,
            redirect_chain: None,
            response_time_ms: 100,
            content_type: Some("text/html".to_string()),
            depth: 0,
            title: Some("Test Page".to_string()),
            meta_description: Some("Test description".to_string()),
            meta_robots: None,
            canonical: None,
            h1: vec!["Test".to_string()],
            headings: vec![],
            internal_links: vec![],
            external_links: vec![],
            images: vec![],
            schema_types: vec![],
            og_tags: HashMap::new(),
            content_hash: "abc123".to_string(),
            is_https: true,
            security_headers: SecurityHeaders::default(),
            mixed_content: Vec::new(),
            insecure_forms: Vec::new(),
            url_length: 18,
            has_parameters: false,
            has_underscores: false,
            has_uppercase: false,
            has_non_ascii: false,
            has_multiple_slashes: false,
            has_repetitive_path: false,
            title_length: Some(9),
            title_pixel_width: None,
            meta_description_length: Some(16),
            meta_description_pixel_width: None,
            title_count: 1,
            meta_description_count: 1,
            title_in_head: true,
            meta_desc_in_head: true,
            canonical_is_relative: false,
            canonical_count: 0,
            canonical_source: None,
            has_self_canonical: false,
            x_robots_tag: None,
            meta_robots_directives: Vec::new(),
            hreflang_tags: Vec::new(),
            pagination: None,
            readability_score: None,
            is_soft_404: false,
            structured_data: Vec::new(),
            unsafe_cross_origin_links: Vec::new(),
            lcp_ms: None,
            inp_ms: None,
            cls: None,
            fcp_ms: None,
            ttfb_ms: None,
            performance_score: None,
            cwv_status: None,
            js_injected_title: false,
            js_injected_description: false,
            js_injected_canonical: false,
            js_injected_h1: false,
            js_injected_structured_data: false,
            console_errors: Vec::new(),
            scripts: Vec::new(),
        }],
        issues: CrawlIssues { issues: vec![] },
        summary: CrawlSummary {
            total_pages: 1,
            by_status: HashMap::new(),
            avg_response_time_ms: 100,
            avg_word_count: 10,
            total_internal_links: 0,
            total_external_links: 0,
            total_images: 0,
            images_without_alt: 0,
            pages_with_schema: 0,
            total_issues: 0,
            critical_issues: 0,
            warning_issues: 0,
            info_issues: 0,
            issues_by_category: HashMap::new(),
            health_score: 100,
            grade: "A".to_string(),
            grade_verdict: "Excellent".to_string(),
            response_time_p50_ms: 100,
            response_time_p95_ms: 100,
            cwv_pages_tested: 0,
            cwv_pages_good: 0,
            cwv_pages_needs_work: 0,
            cwv_pages_poor: 0,
            avg_lcp_ms: None,
            avg_inp_ms: None,
            avg_cls: None,
            avg_performance_score: None,
        },
        robots_txt: None,
        sitemap_data: None,
    }
}

#[test]
fn test_body_text_excluded_by_default() {
    let result = make_test_result();
    let json = slither_core::report::json::serialize_crawl_result(&result).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["pages"][0]["body_text"].as_str().unwrap(), "");
}

#[test]
fn test_body_text_included_when_flagged() {
    let mut result = make_test_result();
    result.export_settings.include_body_text = true;
    let json = slither_core::report::json::serialize_crawl_result(&result).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let body = parsed["pages"][0]["body_text"].as_str().unwrap();
    assert!(body.contains("body text"));
}

#[test]
fn test_compact_format() {
    let mut result = make_test_result();
    result.export_settings.format = "compact".to_string();
    let json = slither_core::report::json::serialize_crawl_result(&result).unwrap();
    assert!(!json.contains('\n'));
}

#[test]
fn test_summary_only() {
    let mut result = make_test_result();
    result.export_settings.summary_only = true;
    let json = slither_core::report::json::serialize_crawl_result(&result).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["pages"].as_array().unwrap().is_empty());
}

#[test]
fn test_export_settings_in_output() {
    let result = make_test_result();
    let json = slither_core::report::json::serialize_crawl_result(&result).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed["export_settings"].is_object());
    assert_eq!(parsed["export_settings"]["include_body_text"], false);
}
