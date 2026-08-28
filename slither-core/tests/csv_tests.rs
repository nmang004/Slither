use slither_core::models::config::CrawlConfig;
use slither_core::models::crawl_result::{
    CrawlIssues, CrawlMetadata, CrawlResult, CrawlSummary, ExportSettings,
};
use slither_core::models::page::{ImageData, SecurityHeaders};
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
            body_text: String::new(),
            word_count: 100,
            redirect_chain: None,
            response_time_ms: 150,
            content_type: Some("text/html".to_string()),
            depth: 0,
            title: Some("Test Page".to_string()),
            meta_description: Some("A test page".to_string()),
            meta_robots: None,
            canonical: None,
            h1: vec!["Hello".to_string()],
            headings: vec![],
            internal_links: vec![],
            external_links: vec![],
            images: vec![],
            schema_types: vec![],
            og_tags: HashMap::new(),
            content_hash: "abc".to_string(),
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
            meta_description_length: Some(11),
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
            avg_response_time_ms: 150,
            avg_word_count: 100,
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
            response_time_p50_ms: 150,
            response_time_p95_ms: 150,
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
fn test_csv_header_row() {
    let result = make_test_result();
    let csv = slither_core::report::csv::generate_csv(&result).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert!(lines[0].starts_with("url,status,title,"));
}

#[test]
fn test_csv_data_row() {
    let result = make_test_result();
    let csv = slither_core::report::csv::generate_csv(&result).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 2); // header + 1 data row
    assert!(lines[1].contains("https://test.com/"));
    assert!(lines[1].contains("200"));
}

#[test]
fn test_csv_escapes_commas_in_title() {
    let mut result = make_test_result();
    result.pages[0].title = Some("Hello, World".to_string());
    let csv = slither_core::report::csv::generate_csv(&result).unwrap();
    assert!(csv.contains("\"Hello, World\""));
}

#[test]
fn test_csv_images_missing_alt_count() {
    let mut result = make_test_result();
    result.pages[0].images = vec![
        ImageData {
            src: "/a.jpg".into(),
            alt: Some("ok".into()),
            width: None,
            height: None,
        },
        ImageData {
            src: "/b.jpg".into(),
            alt: None,
            width: None,
            height: None,
        },
    ];
    let csv = slither_core::report::csv::generate_csv(&result).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    // data row should contain image count (2) and missing alt count (1)
    assert!(lines[1].contains(",2,1,"));
}

/// Split one RFC 4180 record into fields. Kept local so these tests do not add
/// a dependency; it handles the quoting the generator actually emits.
fn split_record(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => fields.push(std::mem::take(&mut cur)),
            other => cur.push(other),
        }
    }
    fields.push(cur);
    fields
}

/// Parse the generated CSV into (header, rows), joining physical lines that
/// belong to one record because a quoted cell contains a newline.
fn parse_csv(csv: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let mut records: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in csv.lines() {
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
        if current.matches('"').count().is_multiple_of(2) {
            records.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        records.push(current);
    }
    let mut it = records.into_iter().filter(|r| !r.trim().is_empty());
    let header = split_record(&it.next().expect("header row"));
    let rows = it.map(|r| split_record(&r)).collect();
    (header, rows)
}

fn field(header: &[String], row: &[String], name: &str) -> String {
    let idx = header
        .iter()
        .position(|h| h == name)
        .unwrap_or_else(|| panic!("missing column {name}"));
    row[idx].clone()
}

/// Every data row must have exactly as many fields as the header. This is the
/// invariant a spreadsheet or `csv.DictReader` depends on, and the one an added
/// column can silently break.
#[test]
fn every_row_has_the_same_field_count_as_the_header() {
    let mut result = make_test_result();
    // Values that exercise the quoting path, so the count is checked against
    // escaped cells rather than only simple ones.
    result.pages[0].title = Some("Comma, and \"quote\"".to_string());
    result.pages[0].meta_description = Some("multi\nline".to_string());

    let csv = slither_core::report::csv::generate_csv(&result).unwrap();
    let (header, rows) = parse_csv(&csv);
    assert_eq!(rows.len(), 1, "one data row");
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.len(), header.len(), "row {i} field count");
    }
}

/// Regression: the CSV counted `data:` URI placeholders and 1x1 pixels as
/// images missing alt text, while the issue check and the crawl summary
/// excluded them. One run reported 306 in the CSV against 3 in the issue list
/// for the same page — and the CSV is what a spreadsheet workflow acts on.
#[test]
fn csv_missing_alt_count_excludes_decorative_images() {
    let mut result = make_test_result();
    result.pages[0].images = vec![
        // Real content image with no alt — must count.
        ImageData {
            src: "https://test.com/photo.jpg".into(),
            alt: None,
            width: Some(800),
            height: Some(600),
        },
        // Lazy-load placeholder — decorative, must not count.
        ImageData {
            src: "data:image/svg+xml,%3Csvg%3E".into(),
            alt: None,
            width: None,
            height: None,
        },
        // Tracking pixel — decorative, must not count.
        ImageData {
            src: "https://test.com/px.gif".into(),
            alt: None,
            width: Some(1),
            height: Some(1),
        },
    ];

    let csv = slither_core::report::csv::generate_csv(&result).unwrap();
    let (header, rows) = parse_csv(&csv);
    assert_eq!(
        field(&header, &rows[0], "images"),
        "3",
        "all images counted"
    );
    assert_eq!(
        field(&header, &rows[0], "images_missing_alt"),
        "1",
        "only the real content image is missing alt"
    );
}

/// Regression: only `meta_robots` was exported, so a page noindexed by header
/// was indistinguishable from an indexable one and a "filter for noindex" pass
/// over the spreadsheet silently missed it.
#[test]
fn csv_exposes_header_delivered_noindex() {
    let mut result = make_test_result();
    result.pages[0].x_robots_tag = Some("NOINDEX".to_string());

    let csv = slither_core::report::csv::generate_csv(&result).unwrap();
    let (header, rows) = parse_csv(&csv);
    assert_eq!(field(&header, &rows[0], "x_robots_tag"), "NOINDEX");
    assert_eq!(
        field(&header, &rows[0], "is_indexable"),
        "false",
        "uppercase header directives are still noindex"
    );
}

/// `none` is documented by Google as equivalent to `noindex, nofollow`.
#[test]
fn csv_treats_none_directive_as_not_indexable() {
    let mut result = make_test_result();
    result.pages[0].x_robots_tag = Some("none".to_string());
    let csv = slither_core::report::csv::generate_csv(&result).unwrap();
    let (header, rows) = parse_csv(&csv);
    assert_eq!(field(&header, &rows[0], "is_indexable"), "false");
}

/// An ordinary page stays indexable — the guard must not invert.
#[test]
fn csv_marks_an_ordinary_page_indexable() {
    let result = make_test_result();
    let csv = slither_core::report::csv::generate_csv(&result).unwrap();
    let (header, rows) = parse_csv(&csv);
    assert_eq!(field(&header, &rows[0], "is_indexable"), "true");
    assert_eq!(field(&header, &rows[0], "x_robots_tag"), "");
}
