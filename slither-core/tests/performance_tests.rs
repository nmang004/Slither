use slither_core::analysis::analyzers::performance::PerformanceAnalyzer;
use slither_core::analysis::{AnalysisContext, Analyzer};
use slither_core::models::issue::{IssueCategory, Severity};
use slither_core::models::page::PageData;

fn make_page(
    url: &str,
    lcp: Option<f64>,
    inp: Option<f64>,
    cls: Option<f64>,
    score: Option<u32>,
) -> PageData {
    let mut page = slither_core::crawler::parser::parse_html(
        "<html><head><title>Test</title></head><body>Hello</body></html>",
        url,
    );
    page.status = 200;
    page.lcp_ms = lcp;
    page.inp_ms = inp;
    page.cls = cls;
    page.performance_score = score;
    page
}

fn run_analyzer(pages: Vec<PageData>) -> Vec<slither_core::models::issue::Issue> {
    let ctx = AnalysisContext {
        seed_url: "https://example.com".to_string(),
        domain: "example.com".to_string(),
        sitemap_data: None,
        pages,
        robots_txt: None,
    };
    PerformanceAnalyzer.analyze(&ctx)
}

#[test]
fn test_no_issues_when_no_cwv_data() {
    let pages = vec![make_page("https://example.com", None, None, None, None)];
    let issues = run_analyzer(pages);
    assert!(issues.is_empty());
}

#[test]
fn test_no_issues_when_all_good() {
    let pages = vec![make_page(
        "https://example.com",
        Some(2000.0),
        Some(150.0),
        Some(0.05),
        Some(95),
    )];
    let issues = run_analyzer(pages);
    assert!(issues.is_empty());
}

#[test]
fn test_lcp_poor_is_critical() {
    let pages = vec![make_page(
        "https://example.com/slow",
        Some(5000.0),
        Some(100.0),
        Some(0.05),
        Some(40),
    )];
    let issues = run_analyzer(pages);
    let lcp_issue = issues.iter().find(|i| i.check == "cwv_lcp_poor");
    assert!(lcp_issue.is_some());
    assert_eq!(lcp_issue.unwrap().severity, Severity::Critical);
}

#[test]
fn test_lcp_needs_work_is_warning() {
    let pages = vec![make_page(
        "https://example.com/ok",
        Some(3000.0),
        Some(100.0),
        Some(0.05),
        Some(85),
    )];
    let issues = run_analyzer(pages);
    let lcp_issue = issues.iter().find(|i| i.check == "cwv_lcp_needs_work");
    assert!(lcp_issue.is_some());
    assert_eq!(lcp_issue.unwrap().severity, Severity::Warning);
}

#[test]
fn test_cls_poor_is_critical() {
    let pages = vec![make_page(
        "https://example.com",
        Some(2000.0),
        Some(100.0),
        Some(0.3),
        Some(60),
    )];
    let issues = run_analyzer(pages);
    let cls_issue = issues.iter().find(|i| i.check == "cwv_cls_poor");
    assert!(cls_issue.is_some());
    assert_eq!(cls_issue.unwrap().severity, Severity::Critical);
}

#[test]
fn test_score_poor_is_critical() {
    let pages = vec![make_page(
        "https://example.com",
        Some(2000.0),
        Some(100.0),
        Some(0.05),
        Some(40),
    )];
    let issues = run_analyzer(pages);
    let score_issue = issues.iter().find(|i| i.check == "cwv_score_poor");
    assert!(score_issue.is_some());
    assert_eq!(score_issue.unwrap().severity, Severity::Critical);
}

#[test]
fn test_performance_category() {
    assert_eq!(PerformanceAnalyzer.category(), IssueCategory::Performance);
}

#[test]
fn test_scoring_includes_performance_weight() {
    use slither_core::analysis::scoring::compute_health_score;
    use slither_core::models::issue::{Issue, IssueUrl};

    let issues = vec![Issue {
        category: IssueCategory::Performance,
        check: "cwv_lcp_poor".to_string(),
        display_name: "LCP exceeds 4.0s (Poor)".to_string(),
        severity: Severity::Critical,
        description: "Test".to_string(),
        guidance: "Fix".to_string(),
        urls: vec![IssueUrl {
            url: "/page".to_string(),
            detail: None,
        }],
    }];
    let grade = compute_health_score(&issues, 10);
    // Performance critical_per_url = 2.0, 1 URL = -2 points
    assert_eq!(grade.score, 98);
}
