//! Health-score behaviour and calibration.
//!
//! These replace an earlier suite that asserted exact point values under the
//! per-category-cap model. That model was retired (see
//! `docs/scoring-recalibration.md`): it could not rank sites by quality — a
//! parked placeholder outscored web.dev — because low-severity findings cost
//! ~12 points on every site and non-ranking factors like security headers
//! scored at all. The assertions here describe what the score is *for*, plus a
//! calibration table so a future weight change has to be deliberate.

use slither_core::analysis::scoring::{
    compute_health_score, compute_percentiles, count_urls_by_severity,
};
use slither_core::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

/// `check` decides impact, so tests must name a real check id.
fn issue(category: IssueCategory, check: &str, severity: Severity, url_count: usize) -> Issue {
    Issue {
        category,
        check: check.to_string(),
        display_name: check.to_string(),
        severity,
        description: "Test issue".to_string(),
        guidance: "Fix it".to_string(),
        urls: (0..url_count)
            .map(|i| IssueUrl {
                url: format!("https://example.com/page-{i}"),
                detail: None,
            })
            .collect(),
    }
}

/// Same as [`issue`], but over an explicit URL set so the union logic can be
/// exercised with two checks hitting the same pages.
fn issue_on(category: IssueCategory, check: &str, urls: &[usize]) -> Issue {
    Issue {
        category,
        check: check.to_string(),
        display_name: check.to_string(),
        severity: Severity::Critical,
        description: "Test issue".to_string(),
        guidance: "Fix it".to_string(),
        urls: urls
            .iter()
            .map(|i| IssueUrl {
                url: format!("https://example.com/page-{i}"),
                detail: None,
            })
            .collect(),
    }
}

fn band(score: u32) -> char {
    match score {
        90..=100 => 'A',
        80..=89 => 'B',
        70..=79 => 'C',
        60..=69 => 'D',
        _ => 'F',
    }
}

// ---------------------------------------------------------------------------
// The two properties the score exists to express
// ---------------------------------------------------------------------------

#[test]
fn a_clean_site_scores_full_marks() {
    let grade = compute_health_score(&[], 100);
    assert_eq!(grade.score, 100);
    assert_eq!(grade.letter, "A");
}

/// Severe issues must dominate: pages Google drops from the index are the
/// single biggest thing a site audit can tell you.
#[test]
fn broken_pages_dominate_the_score() {
    let quarter = compute_health_score(
        &[issue(
            IssueCategory::ResponseCodes,
            "internal_client_error",
            Severity::Critical,
            250,
        )],
        1000,
    );
    assert_eq!(
        band(quarter.score),
        'C',
        "25% of pages 404 → {}",
        quarter.score
    );

    let mass = compute_health_score(
        &[issue(
            IssueCategory::ResponseCodes,
            "internal_server_error",
            Severity::Critical,
            400,
        )],
        1000,
    );
    assert!(
        mass.score <= 65,
        "40% of pages 5xx must land in D or worse, got {}",
        mass.score
    );
    assert!(
        mass.score < quarter.score,
        "more broken pages must score worse"
    );
}

/// Minor issues must register but never dominate. A site whose *only* faults
/// are cosmetic should still read as healthy — this is the failure that made
/// every real site a D or F under the old model.
#[test]
fn cosmetic_issues_alone_cannot_sink_a_site() {
    let n = 1000;
    let issues = vec![
        issue(
            IssueCategory::MetaDescription,
            "over_155_chars",
            Severity::Info,
            n,
        ),
        issue(IssueCategory::Headings, "non_sequential", Severity::Info, n),
        issue(IssueCategory::Headings, "h1_missing", Severity::Warning, n),
        issue(
            IssueCategory::Images,
            "missing_alt_attribute",
            Severity::Warning,
            n,
        ),
        issue(
            IssueCategory::Images,
            "missing_dimensions",
            Severity::Info,
            n,
        ),
        issue(IssueCategory::Url, "contains_uppercase", Severity::Info, n),
        issue(IssueCategory::StructuredData, "missing", Severity::Info, n),
    ];
    let grade = compute_health_score(&issues, n as u32);
    assert!(
        grade.score >= 85,
        "a site with only cosmetic findings should stay in A/B, got {}",
        grade.score
    );
    assert!(
        grade.score < 100,
        "minor issues should still cost something, got {}",
        grade.score
    );
}

// ---------------------------------------------------------------------------
// Research-backed exclusions
// ---------------------------------------------------------------------------

/// Mueller: "the security headers are more about, well, security." They are
/// reported, but they are not ranking factors and must not move the score.
#[test]
fn security_headers_do_not_affect_the_score() {
    let n = 500;
    let issues = vec![
        issue(IssueCategory::Security, "missing_hsts", Severity::Info, n),
        issue(IssueCategory::Security, "missing_csp", Severity::Info, n),
        issue(
            IssueCategory::Security,
            "missing_x_frame_options",
            Severity::Info,
            n,
        ),
        issue(
            IssueCategory::Security,
            "missing_referrer_policy",
            Severity::Info,
            n,
        ),
        issue(
            IssueCategory::Security,
            "unsafe_cross_origin",
            Severity::Info,
            n,
        ),
    ];
    assert_eq!(compute_health_score(&issues, n as u32).score, 100);
}

/// HTTPS itself is a different matter and does count.
#[test]
fn serving_over_http_still_counts_against_the_score() {
    let issues = vec![issue(
        IssueCategory::Security,
        "http_urls",
        Severity::Warning,
        500,
    )];
    assert!(compute_health_score(&issues, 500).score < 60);
}

/// A deliberate crawler policy is not a defect.
#[test]
fn ai_crawler_directives_are_reported_but_not_scored() {
    let issues = vec![
        issue(
            IssueCategory::Directives,
            "ai_crawlers_blocked",
            Severity::Info,
            100,
        ),
        issue(
            IssueCategory::Directives,
            "noai_directive",
            Severity::Info,
            100,
        ),
    ];
    assert_eq!(compute_health_score(&issues, 100).score, 100);
}

// ---------------------------------------------------------------------------
// Model mechanics
// ---------------------------------------------------------------------------

/// A page broken two different ways is still one broken page. Summing per-check
/// coverage would double-count it.
#[test]
fn pages_broken_several_ways_are_counted_once() {
    let urls: Vec<usize> = (0..100).collect();
    let once = compute_health_score(
        &[issue_on(
            IssueCategory::ResponseCodes,
            "internal_client_error",
            &urls,
        )],
        1000,
    );
    let twice = compute_health_score(
        &[
            issue_on(IssueCategory::ResponseCodes, "internal_client_error", &urls),
            issue_on(IssueCategory::Canonicals, "non_indexable_target", &urls),
        ],
        1000,
    );
    assert_eq!(
        once.score, twice.score,
        "the same 100 pages broken two ways must not cost twice"
    );
}

/// A page that links to a dead URL is not itself broken, and the dead target is
/// already counted by the 4xx check. One dead link in a global nav previously
/// cost ~20 points because it was treated as a sitewide page defect.
#[test]
fn a_dead_link_in_global_nav_is_bounded() {
    let issues = vec![
        issue(
            IssueCategory::Links,
            "broken_internal",
            Severity::Critical,
            1000,
        ),
        issue(
            IssueCategory::ResponseCodes,
            "internal_client_error",
            Severity::Critical,
            1,
        ),
    ];
    let grade = compute_health_score(&issues, 1000);
    assert!(
        grade.score >= 80,
        "one dead nav link is one fix, not a failing site: got {}",
        grade.score
    );
    assert!(grade.score < 95, "it should still cost something");
}

/// A proportion measured over one or two pages is noise: a single-page site
/// with one flaw is not "100% broken".
#[test]
fn tiny_crawls_are_not_judged_on_a_single_page() {
    let issues = vec![issue(
        IssueCategory::PageTitles,
        "missing",
        Severity::Warning,
        1,
    )];
    let grade = compute_health_score(&issues, 1);
    assert!(
        grade.score >= 80,
        "one page missing a title on a 1-page crawl should not be catastrophic, got {}",
        grade.score
    );
}

/// A crawl that reached no pages has nothing to grade. Reporting 100/A there
/// presented "blocked by robots.txt" or "unreachable" as a flawless result.
#[test]
fn an_empty_crawl_is_not_reported_as_perfect() {
    let grade = compute_health_score(&[], 0);
    assert_ne!(grade.letter, "A");
    assert_eq!(grade.verdict, "No pages crawled");
    assert!(grade.deductions.is_empty());
}

/// A bot wall answering 403 to everything scored 92/A — "excellent site" when
/// it actually means "we were not allowed to look". If every page we saw was
/// non-2xx, there is nothing to grade.
#[test]
fn a_crawl_that_was_blocked_is_not_graded() {
    let issues = vec![issue(
        IssueCategory::ResponseCodes,
        "internal_access_restricted",
        Severity::Warning,
        1,
    )];
    let grade = compute_health_score(&issues, 1);
    assert_ne!(grade.letter, "A");
    assert!(grade.verdict.contains("blocked"), "got {}", grade.verdict);
}

/// But a site with a few restricted pages among many good ones is still graded.
#[test]
fn a_few_restricted_pages_do_not_void_the_grade() {
    let issues = vec![issue(
        IssueCategory::ResponseCodes,
        "internal_access_restricted",
        Severity::Warning,
        3,
    )];
    let grade = compute_health_score(&issues, 200);
    assert!(grade.score > 80, "got {}", grade.score);
}

#[test]
fn score_stays_within_bounds() {
    assert_eq!(compute_health_score(&[], 100).score, 100);

    let mut issues = Vec::new();
    for check in [
        "internal_server_error",
        "internal_client_error",
        "internal_no_response",
    ] {
        issues.push(issue(
            IssueCategory::ResponseCodes,
            check,
            Severity::Critical,
            500,
        ));
    }
    issues.push(issue(
        IssueCategory::Canonicals,
        "non_indexable_target",
        Severity::Critical,
        500,
    ));
    issues.push(issue(
        IssueCategory::Content,
        "exact_duplicate",
        Severity::Warning,
        500,
    ));
    let grade = compute_health_score(&issues, 500);
    assert_eq!(grade.score, 0, "a fully broken site floors at zero");
}

#[test]
fn grade_bands_are_contiguous() {
    assert_eq!(band(100), 'A');
    assert_eq!(band(90), 'A');
    assert_eq!(band(89), 'B');
    assert_eq!(band(80), 'B');
    assert_eq!(band(79), 'C');
    assert_eq!(band(70), 'C');
    assert_eq!(band(69), 'D');
    assert_eq!(band(60), 'D');
    assert_eq!(band(59), 'F');
}

#[test]
fn the_breakdown_explains_where_points_went() {
    let issues = vec![
        issue(
            IssueCategory::ResponseCodes,
            "internal_client_error",
            Severity::Critical,
            50,
        ),
        issue(
            IssueCategory::Headings,
            "non_sequential",
            Severity::Info,
            500,
        ),
    ];
    let grade = compute_health_score(&issues, 500);
    assert!(!grade.deductions.is_empty());
    let total: f64 = grade.deductions.iter().map(|d| d.points_deducted).sum();
    assert!(
        (total - (100.0 - grade.score as f64)).abs() < 1.5,
        "the breakdown should account for the deduction"
    );
}

// ---------------------------------------------------------------------------
// Calibration — the regression guard
// ---------------------------------------------------------------------------

/// Scenarios of known severity, with the band each must land in. The old model
/// drifted precisely because nothing pinned these down: a weight change that
/// pushes a cosmetic-only site out of A/B, or lets a mass-404 site keep a
/// passing grade, should fail here and have to be argued for deliberately.
#[test]
fn calibration_scenarios_land_in_the_expected_bands() {
    let n = 1000u32;
    let cases: Vec<(&str, Vec<Issue>, &str)> = vec![
        ("flawless", vec![], "A"),
        (
            "cosmetic only",
            vec![
                issue(
                    IssueCategory::MetaDescription,
                    "over_155_chars",
                    Severity::Info,
                    1000,
                ),
                issue(
                    IssueCategory::Headings,
                    "h1_missing",
                    Severity::Warning,
                    1000,
                ),
                issue(
                    IssueCategory::Images,
                    "missing_alt_attribute",
                    Severity::Warning,
                    1000,
                ),
            ],
            "AB",
        ),
        (
            "one dead nav link",
            vec![
                issue(
                    IssueCategory::Links,
                    "broken_internal",
                    Severity::Critical,
                    1000,
                ),
                issue(
                    IssueCategory::ResponseCodes,
                    "internal_client_error",
                    Severity::Critical,
                    1,
                ),
            ],
            "AB",
        ),
        (
            "duplicate content on 30%",
            vec![issue(
                IssueCategory::Content,
                "exact_duplicate",
                Severity::Warning,
                300,
            )],
            "B",
        ),
        (
            "25% of pages 404",
            vec![issue(
                IssueCategory::ResponseCodes,
                "internal_client_error",
                Severity::Critical,
                250,
            )],
            "C",
        ),
        (
            "40% of pages 404",
            vec![issue(
                IssueCategory::ResponseCodes,
                "internal_client_error",
                Severity::Critical,
                400,
            )],
            "D",
        ),
        (
            "sitewide indexability collapse",
            vec![issue(
                IssueCategory::Canonicals,
                "non_indexable_target",
                Severity::Critical,
                1000,
            )],
            "F",
        ),
    ];

    for (name, issues, expected) in cases {
        let grade = compute_health_score(&issues, n);
        let got = band(grade.score);
        assert!(
            expected.contains(got),
            "{name}: expected band {expected}, got {got} ({})",
            grade.score
        );
    }
}

// ---------------------------------------------------------------------------
// Unrelated helpers that live in the same module
// ---------------------------------------------------------------------------

#[test]
fn percentiles_handle_empty_and_single_values() {
    let (p50, p95) = compute_percentiles(&mut []);
    assert_eq!((p50, p95), (0, 0));
    let (p50, p95) = compute_percentiles(&mut [100]);
    assert_eq!((p50, p95), (100, 100));
}

#[test]
fn percentiles_are_computed_over_sorted_values() {
    let mut times: Vec<u64> = (1..=100).collect();
    let (p50, p95) = compute_percentiles(&mut times);
    assert_eq!(p50, 50);
    assert_eq!(p95, 95);
}

#[test]
fn severity_counts_sum_urls_per_severity() {
    let issues = vec![
        issue(
            IssueCategory::Links,
            "broken_internal",
            Severity::Critical,
            2,
        ),
        issue(IssueCategory::Links, "orphan_pages", Severity::Warning, 3),
        issue(IssueCategory::Links, "no_anchor_text", Severity::Info, 1),
    ];
    let (critical, warning, info) = count_urls_by_severity(&issues);
    assert_eq!((critical, warning, info), (2, 3, 1));
}

// ---------------------------------------------------------------------------
// Edge-case sweep regressions (scoring)
// ---------------------------------------------------------------------------

/// A follow-vs-nofollow mismatch leaves every page indexed. Scoring it in the
/// same 100-point bucket as an indexability contradiction drove a completely
/// indexable 15-page site to 0/F.
#[test]
fn a_follow_conflict_does_not_score_as_a_broken_page() {
    let issues = vec![issue(
        IssueCategory::Directives,
        "conflicting_follow_directives",
        Severity::Warning,
        15,
    )];
    let grade = compute_health_score(&issues, 15);
    assert!(
        grade.score >= 80,
        "a fully indexable site should not fail on a follow mismatch, got {}",
        grade.score
    );
}

/// The index/noindex contradiction genuinely removes pages from the index and
/// must keep its weight.
#[test]
fn an_index_conflict_still_scores_as_a_broken_page() {
    let issues = vec![issue(
        IssueCategory::Directives,
        "conflicting_directives",
        Severity::Warning,
        15,
    )];
    assert!(compute_health_score(&issues, 15).score < 30);
}

/// `missing_x_default` is Info-severity and Google calls it optional
/// ("consider adding a fallback"). A blanket `(Hreflang, _)` wildcard scored it
/// in the same tier as a missing canonical, costing 4 of 14 SiteMedium points
/// for declining a suggestion.
#[test]
fn optional_hreflang_suggestions_score_as_minor() {
    let issues = vec![issue(
        IssueCategory::Hreflang,
        "missing_x_default",
        Severity::Info,
        12,
    )];
    let grade = compute_health_score(&issues, 12);
    assert!(
        grade.score >= 97,
        "an optional Info suggestion should barely register, got {}",
        grade.score
    );
}

/// A real hreflang defect still carries weight.
#[test]
fn hreflang_reciprocity_failures_still_score() {
    let issues = vec![issue(
        IssueCategory::Hreflang,
        "missing_return_links",
        Severity::Warning,
        12,
    )];
    assert!(compute_health_score(&issues, 12).score < 97);
}

/// A bot-challenge wall served with HTTP 200 is invisible to a status-based
/// guard. Le Monde was graded F (26/100) for the properties of 17 Akamai
/// interstitials. The signature is that nearly every page is thin and
/// byte-identical, i.e. we never saw the site.
#[test]
fn a_200_status_bot_wall_is_not_graded_as_the_site() {
    let issues = vec![
        issue(IssueCategory::Content, "low_content", Severity::Warning, 18),
        issue(
            IssueCategory::Content,
            "exact_duplicate",
            Severity::Warning,
            19,
        ),
    ];
    let grade = compute_health_score(&issues, 22);
    assert_ne!(grade.letter, "F");
    assert!(
        grade.verdict.contains("readable") || grade.verdict.contains("blocked"),
        "expected an unscorable verdict, got {}",
        grade.verdict
    );
}

/// A client-rendered shell the crawler could not read reaches the same place by
/// a different route: thin, and with no links at all.
#[test]
fn a_client_rendered_shell_is_not_given_a_passing_grade() {
    let issues = vec![
        issue(IssueCategory::Content, "low_content", Severity::Warning, 1),
        issue(
            IssueCategory::Links,
            "no_internal_outlinks",
            Severity::Warning,
            1,
        ),
    ];
    let grade = compute_health_score(&issues, 1);
    assert_eq!(grade.letter, "–", "got {} ({})", grade.letter, grade.score);
}

/// A site with some thin pages among many good ones is still a gradeable site.
#[test]
fn a_few_thin_pages_do_not_void_the_grade() {
    let issues = vec![
        issue(IssueCategory::Content, "low_content", Severity::Warning, 5),
        issue(
            IssueCategory::Content,
            "exact_duplicate",
            Severity::Warning,
            4,
        ),
    ];
    let grade = compute_health_score(&issues, 100);
    assert_ne!(grade.letter, "–");
    assert!(grade.score > 60);
}

/// `affected_urls` summed issue rows, so a check emitting one row per
/// (heading, page) pair reported "676 URLs" on a 22-page crawl — a number that
/// cannot be true, printed on the face of a client deliverable.
#[test]
fn affected_urls_counts_distinct_urls() {
    use slither_core::analysis::scoring::build_category_summaries;

    // Three rows, all for the same two pages.
    let mut issues = Vec::new();
    for check in ["h2_duplicate", "h2_duplicate", "non_sequential"] {
        issues.push(issue(IssueCategory::Headings, check, Severity::Info, 2));
    }
    let summaries = build_category_summaries(&issues);
    let headings = summaries
        .values()
        .find(|_| true)
        .expect("one category summary");
    assert_eq!(
        headings.affected_urls, 2,
        "three rows over two pages is two affected URLs"
    );
}
