use slither_core::analysis::analyzers::js::JsAnalyzer;
use slither_core::analysis::{AnalysisContext, Analyzer};
use slither_core::models::issue::{Issue, IssueCategory, Severity};
use slither_core::models::page::{PageData, ScriptData};

fn base_page(url: &str) -> PageData {
    let mut page = slither_core::crawler::parser::parse_html(
        "<html><head><title>Test</title></head><body>Hello</body></html>",
        url,
    );
    page.status = 200;
    // parse_html populates scripts from the document; clear so tests can
    // opt in to specific script fixtures.
    page.scripts = Vec::new();
    page
}

fn run(pages: Vec<PageData>) -> Vec<Issue> {
    let ctx = AnalysisContext {
        seed_url: "https://example.com".to_string(),
        domain: "example.com".to_string(),
        sitemap_data: None,
        pages,
        robots_txt: None,
    };
    JsAnalyzer.analyze(&ctx)
}

#[test]
fn test_analyzer_reports_javascript_category() {
    assert_eq!(JsAnalyzer.category(), IssueCategory::JavaScript);
}

#[test]
fn test_no_issues_when_clean() {
    let page = base_page("https://example.com/");
    let issues = run(vec![page]);
    assert!(issues.is_empty(), "Expected no issues, got {:?}", issues);
}

#[test]
fn test_js_injected_canonical_is_critical() {
    let mut page = base_page("https://example.com/");
    page.js_injected_canonical = true;
    let issues = run(vec![page]);
    let found: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.check == "js_injected_canonical")
        .collect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Critical);
}

#[test]
fn test_js_injected_title_is_warning() {
    let mut page = base_page("https://example.com/");
    page.js_injected_title = true;
    let issues = run(vec![page]);
    let found: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.check == "js_injected_title")
        .collect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
}

#[test]
fn test_js_injected_description_is_warning() {
    let mut page = base_page("https://example.com/");
    page.js_injected_description = true;
    let issues = run(vec![page]);
    assert!(issues
        .iter()
        .any(|i| i.check == "js_injected_description" && i.severity == Severity::Warning));
}

#[test]
fn test_js_injected_h1_is_warning() {
    let mut page = base_page("https://example.com/");
    page.js_injected_h1 = true;
    let issues = run(vec![page]);
    assert!(issues
        .iter()
        .any(|i| i.check == "js_injected_h1" && i.severity == Severity::Warning));
}

#[test]
fn test_js_injected_structured_data_is_warning() {
    let mut page = base_page("https://example.com/");
    page.js_injected_structured_data = true;
    let issues = run(vec![page]);
    assert!(issues
        .iter()
        .any(|i| i.check == "js_injected_structured_data" && i.severity == Severity::Warning));
}

#[test]
fn test_render_blocking_scripts_detected() {
    let mut page = base_page("https://example.com/");
    page.scripts = vec![
        ScriptData {
            src: Some("https://example.com/app.js".to_string()),
            is_async: false,
            is_defer: false,
            is_module: false,
            in_head: true,
            size_bytes: 0,
            is_third_party: false,
        },
        // Deferred script — should NOT count as render-blocking
        ScriptData {
            src: Some("https://example.com/later.js".to_string()),
            is_async: false,
            is_defer: true,
            is_module: false,
            in_head: true,
            size_bytes: 0,
            is_third_party: false,
        },
    ];
    let issues = run(vec![page]);
    let found: Vec<&Issue> = issues
        .iter()
        .filter(|i| i.check == "js_render_blocking")
        .collect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].severity, Severity::Warning);
    assert_eq!(found[0].urls.len(), 1);
}

#[test]
fn test_render_blocking_skipped_for_async_defer_module() {
    let mut page = base_page("https://example.com/");
    page.scripts = vec![
        ScriptData {
            src: Some("https://example.com/a.js".to_string()),
            is_async: true,
            is_defer: false,
            is_module: false,
            in_head: true,
            size_bytes: 0,
            is_third_party: false,
        },
        ScriptData {
            src: Some("https://example.com/b.js".to_string()),
            is_async: false,
            is_defer: false,
            is_module: true,
            in_head: true,
            size_bytes: 0,
            is_third_party: false,
        },
    ];
    let issues = run(vec![page]);
    assert!(!issues.iter().any(|i| i.check == "js_render_blocking"));
}

#[test]
fn test_excessive_scripts_by_count() {
    let mut page = base_page("https://example.com/");
    page.scripts = (0..25)
        .map(|_| ScriptData {
            src: Some("https://example.com/s.js".to_string()),
            is_async: true,
            is_defer: false,
            is_module: false,
            in_head: false,
            size_bytes: 0,
            is_third_party: false,
        })
        .collect();
    let issues = run(vec![page]);
    assert!(issues
        .iter()
        .any(|i| i.check == "js_excessive_scripts" && i.severity == Severity::Warning));
}

#[test]
fn test_excessive_scripts_by_inline_size() {
    let mut page = base_page("https://example.com/");
    page.scripts = vec![ScriptData {
        src: None,
        is_async: false,
        is_defer: false,
        is_module: false,
        in_head: true,
        size_bytes: 150 * 1024, // 150KB inline
        is_third_party: false,
    }];
    let issues = run(vec![page]);
    assert!(issues.iter().any(|i| i.check == "js_excessive_scripts"));
}

#[test]
fn test_excessive_scripts_not_triggered_below_threshold() {
    let mut page = base_page("https://example.com/");
    page.scripts = (0..10)
        .map(|_| ScriptData {
            src: Some("https://example.com/s.js".to_string()),
            is_async: true,
            is_defer: false,
            is_module: false,
            in_head: false,
            size_bytes: 0,
            is_third_party: false,
        })
        .collect();
    let issues = run(vec![page]);
    assert!(!issues.iter().any(|i| i.check == "js_excessive_scripts"));
}

#[test]
fn test_heavy_third_party_triggers_above_threshold() {
    let mut page = base_page("https://example.com/");
    page.scripts = (0..11)
        .map(|_| ScriptData {
            src: Some("https://cdn.third-party.com/s.js".to_string()),
            is_async: true,
            is_defer: false,
            is_module: false,
            in_head: false,
            size_bytes: 0,
            is_third_party: true,
        })
        .collect();
    let issues = run(vec![page]);
    assert!(issues
        .iter()
        .any(|i| i.check == "js_third_party_heavy" && i.severity == Severity::Warning));
}

#[test]
fn test_heavy_third_party_not_triggered_at_threshold() {
    let mut page = base_page("https://example.com/");
    page.scripts = (0..10)
        .map(|_| ScriptData {
            src: Some("https://cdn.third-party.com/s.js".to_string()),
            is_async: true,
            is_defer: false,
            is_module: false,
            in_head: false,
            size_bytes: 0,
            is_third_party: true,
        })
        .collect();
    let issues = run(vec![page]);
    assert!(!issues.iter().any(|i| i.check == "js_third_party_heavy"));
}

#[test]
fn test_console_errors_warning_and_critical() {
    let mut warn_page = base_page("https://example.com/warn");
    warn_page.console_errors = vec!["TypeError: x".to_string(), "SyntaxError: y".to_string()];

    let mut crit_page = base_page("https://example.com/crit");
    crit_page.console_errors = (0..6).map(|i| format!("Error #{}", i)).collect();

    let issues = run(vec![warn_page, crit_page]);

    let warn = issues
        .iter()
        .find(|i| i.check == "js_console_errors")
        .expect("expected console_errors issue");
    assert_eq!(warn.severity, Severity::Warning);
    // Only the below-threshold page. The heavy page is reported by the
    // Critical check alone — previously it appeared in both, deducting twice
    // for a single condition.
    assert_eq!(warn.urls.len(), 1);
    assert_eq!(warn.urls[0].url, "https://example.com/warn");

    let crit = issues
        .iter()
        .find(|i| i.check == "js_console_errors_critical")
        .expect("expected critical console_errors issue");
    assert_eq!(crit.severity, Severity::Critical);
    assert_eq!(crit.urls.len(), 1);
    assert_eq!(crit.urls[0].url, "https://example.com/crit");
}

#[test]
fn test_pages_without_rendering_data_produce_no_injection_issues() {
    // Simulate a page that wasn't rendered (all js_* fields default to false)
    let page = base_page("https://example.com/static-only");
    let issues = run(vec![page]);
    for issue in &issues {
        assert!(
            !issue.check.starts_with("js_injected_") && !issue.check.starts_with("js_console_"),
            "Unexpected JS-injection/console issue: {}",
            issue.check
        );
    }
}
