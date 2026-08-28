use slither_core::models::config::CrawlConfig;
use slither_core::models::crawl_result::{
    CrawlIssues, CrawlMetadata, CrawlResult, CrawlSummary, ExportSettings,
};
use slither_core::models::page::SecurityHeaders;
use std::collections::HashMap;

fn make_test_result() -> CrawlResult {
    let mut by_status = HashMap::new();
    by_status.insert("200".to_string(), 1);

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
            by_status,
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
fn test_html_report_contains_tabs() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(html.contains("tab-overview"));
    assert!(html.contains("tab-issues"));
    assert!(html.contains("tab-pages"));
    assert!(html.contains("tab-structure"));
}

#[test]
fn test_html_report_contains_health_score() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(html.to_lowercase().contains("health score"));
    assert!(html.contains("<svg")); // SVG gauge
}

#[test]
fn test_html_report_self_contained() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    // No external resource references
    assert!(!html.contains("href=\"http"));
    assert!(!html.contains("src=\"http"));
    assert!(html.contains("<style>")); // CSS is inline
}

#[test]
fn test_html_report_contains_page_data() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(html.contains("https://test.com/"));
    assert!(html.contains("Test Page"));
}

#[test]
fn test_html_report_contains_overview_charts() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    // Should contain donut chart and bar chart SVGs
    let svg_count = html.matches("<svg").count();
    assert!(
        svg_count >= 3,
        "Expected at least 3 SVGs (gauge + donut + bar), got {}",
        svg_count
    );
}

#[test]
fn test_html_report_score_breakdown() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    // Score breakdown section should be present in overview tab
    assert!(html.contains("score-breakdown"));
    assert!(html.contains("toggleScoreBreakdown"));
}

#[test]
fn test_html_report_escapes_crawled_content() {
    // A crawled page whose title/H1/meta contain HTML must never render as
    // executable markup — this is the stored-XSS regression guard.
    let mut result = make_test_result();
    let page = &mut result.pages[0];
    page.title = Some("<img src=x onerror=alert(1)>".to_string());
    page.h1 = vec!["<script>alert('h1')</script>".to_string()];
    page.meta_description = Some("</title><svg onload=alert(2)>".to_string());

    let html = slither_core::report::html::render_html_report(&result).unwrap();

    // The crawled payload is only ever emitted un-escaped inside the JSON island
    // in a <script> block, where it is a JS string literal consumed via
    // textContent (not innerHTML) and `</` is neutralized. Everywhere it lands
    // in HTML markup it must be escaped. Verify by removing the JSON-island line
    // and asserting no executable form survives in the HTML body.
    let body: String = html
        .lines()
        .filter(|l| !l.contains("__structurePageData"))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !body.contains("<img src=x onerror=alert(1)>"),
        "title HTML was not escaped in the HTML body"
    );
    assert!(
        !body.contains("<script>alert('h1')</script>"),
        "H1 HTML was not escaped in the HTML body"
    );
    assert!(
        !body.contains("<svg onload=alert(2)>"),
        "meta description HTML was not escaped in the HTML body"
    );
    // The escaped form should be present instead.
    assert!(
        body.contains("&lt;img src=x onerror=alert(1)&gt;"),
        "expected escaped title in HTML body"
    );
}

#[test]
fn test_html_report_page_data_json_cannot_break_script() {
    // The JSON island embedded in a <script> block must be escaped so a title
    // containing </script> cannot terminate the element.
    let mut result = make_test_result();
    result.pages[0].title = Some("</script><script>alert(3)</script>".to_string());

    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(
        !html.contains("</script><script>alert(3)</script>"),
        "page_data_json allowed a </script> breakout"
    );
    // The neutralized form uses <\/ so the parser never sees a closing tag.
    assert!(
        html.contains("<\\/script>"),
        "expected escaped </ in JSON island"
    );
}

#[test]
fn test_html_report_structure_tab() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    // Structure tab should contain explorer and stats
    assert!(html.contains("structure-stats"));
    assert!(html.contains("structure-explorer"));
    assert!(html.contains("explorer-tree"));
    // Structure tab navigation should exist
    assert!(html.contains("Structure"));
}

#[test]
fn test_html_report_light_theme() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    // Light theme colors should be present
    assert!(html.contains("#f4f7fa")); // --bg
    assert!(html.contains("#5BB4D4") || html.contains("#5bb4d4")); // mascot/accent
                                                                   // Dark theme support and the theme toggle
    assert!(html.contains("[data-theme=\"dark\"]"));
    assert!(html.contains("prefers-color-scheme: dark"));
    assert!(html.contains("toggleTheme"));
    // Sidebar layout
    assert!(html.contains("sidebar"));
    assert!(html.contains("app-layout"));
    // Fun messages
    assert!(
        html.contains("peak hunting form")
            || html.contains("just a few scratches")
            || html.contains("prehistoric problem")
            || html.contains("sharpen those claws")
            || html.contains("needs some TLC")
    );
}

#[test]
fn test_html_report_has_sidebar_nav() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(html.contains("nav-overview"));
    assert!(html.contains("nav-issues"));
    assert!(html.contains("nav-pages"));
    assert!(html.contains("nav-structure"));
}

#[test]
fn test_html_report_hero_card() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(html.contains("hero-card"));
    assert!(html.contains("hero-score"));
    assert!(html.contains("hero-info"));
    assert!(html.contains("hero-grade"));
    assert!(html.contains("hero-message"));
}

#[test]
fn test_html_report_gauge_animation() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(html.contains("@keyframes gaugeArc"));
    assert!(html.contains("animation: gaugeArc"));
}

/// A crawl that returned no usable pages must not render a full green "2xx"
/// ring — that reported a clean bill of health for a crawl that found nothing.
#[test]
fn test_html_report_empty_crawl_has_no_fabricated_healthy_donut() {
    let mut result = make_test_result();
    result.pages = Vec::new();
    result.summary.total_pages = 0;
    result.summary.by_status = HashMap::new();

    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(
        html.contains("no data"),
        "empty crawl should render an explicit no-data segment"
    );
}

/// The structure tree is walked by several recursive helpers, so a hostile URL
/// with thousands of path segments used to be able to overflow the stack while
/// rendering. Rendering must complete instead.
#[test]
fn test_html_report_survives_pathologically_deep_urls() {
    let mut result = make_test_result();
    let deep = format!("https://test.com/{}", "a/".repeat(5000));
    result.pages[0].url = deep;

    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(html.contains("tab-structure"));
}

/// A short or hand-edited crawl_date must not panic the renderer via a byte
/// slice.
#[test]
fn test_html_report_tolerates_a_short_crawl_date() {
    let mut result = make_test_result();
    result.crawl_metadata.crawl_date = "2026".to_string();

    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(html.contains("2026"));
}

/// Regression: the Pages tab recomputed indexability with a case-sensitive
/// `contains("noindex")` instead of calling `PageData::is_noindex`, so a page
/// carrying `X-Robots-Tag: NOINDEX` or `none` rendered a green "Indexable"
/// badge while the Issues tab in the same file listed it as noindex. The
/// page-level field is the one a reader scans.
#[test]
fn header_noindex_pages_are_not_labelled_indexable() {
    for header in ["NOINDEX", "none", "noindex", "googlebot: noindex"] {
        let mut result = make_test_result();
        result.pages[0].x_robots_tag = Some(header.to_string());
        let html = slither_core::report::html::render_html_report(&result).unwrap();
        assert!(
            html.contains(r#"<span class="idx-no">Noindex</span>"#),
            "X-Robots-Tag: {header} must render as Noindex"
        );
        assert!(
            !html.contains(r#"<span class="idx-yes">Indexable</span>"#),
            "X-Robots-Tag: {header} must not render as Indexable"
        );
    }
}

/// An ordinary page must still read as indexable — the fix must not invert.
#[test]
fn an_ordinary_page_is_labelled_indexable() {
    let result = make_test_result();
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(html.contains(r#"<span class="idx-yes">Indexable</span>"#));
    assert!(!html.contains(r#"<span class="idx-no">Noindex</span>"#));
}

/// The JSON detail island backing the Structure tab must agree with the Pages
/// tab; they were computed separately and could disagree.
#[test]
fn the_detail_island_agrees_with_the_pages_tab_on_indexability() {
    let mut result = make_test_result();
    // A real crawl derives the directive list from the tag, so a fixture that
    // sets only the raw string is not representative.
    result.pages[0].meta_robots = Some("NoIndex, NoFollow".to_string());
    result.pages[0].meta_robots_directives = vec!["noindex".to_string(), "nofollow".to_string()];
    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(
        html.contains(r#"<span class="idx-no">Noindex</span>"#),
        "pages tab says noindex"
    );
    assert!(
        html.contains("\"indexable\":false") || html.contains("\"indexable\": false"),
        "detail island must agree"
    );
}

/// Regression: the Pages tab counted decorative images as missing alt text
/// while the Issues tab excluded them, so one file reported 311 and 3 for the
/// same page. All artifacts of one run must agree on a shared metric.
#[test]
fn the_pages_tab_alt_count_excludes_decorative_images() {
    use slither_core::models::page::ImageData;

    let mut result = make_test_result();
    result.pages[0].images = vec![
        ImageData {
            src: "https://test.com/photo.jpg".into(),
            alt: None,
            width: Some(800),
            height: Some(600),
        },
        ImageData {
            src: "data:image/svg+xml,%3Csvg%3E".into(),
            alt: None,
            width: None,
            height: None,
        },
        ImageData {
            src: "https://test.com/px.gif".into(),
            alt: None,
            width: Some(1),
            height: Some(1),
        },
    ];

    let html = slither_core::report::html::render_html_report(&result).unwrap();
    assert!(
        html.contains("3 (1 missing alt)"),
        "expected 3 images with 1 genuinely missing alt"
    );
}
