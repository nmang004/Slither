use slither_core::analysis::{AnalysisContext, AnalyzerRegistry};
use slither_core::models::issue::{IssueCategory, Severity};
use slither_core::models::page::{Heading, LinkData, PageData, SecurityHeaders};
use std::collections::HashMap;

fn make_page(url: &str) -> PageData {
    PageData {
        url: url.to_string(),
        status: 200,
        redirect_chain: None,
        response_time_ms: 100,
        content_type: Some("text/html".to_string()),
        depth: 1,
        title: Some("Test Page Title That Is Long Enough".to_string()),
        meta_description: Some(
            "A test page description that is long enough to pass the minimum length check easily"
                .to_string(),
        ),
        meta_robots: None,
        canonical: Some(url.to_string()),
        h1: vec!["Test Heading".to_string()],
        headings: vec![
            Heading {
                level: 1,
                text: "Test Heading".to_string(),
            },
            Heading {
                level: 2,
                text: "Sub Heading".to_string(),
            },
        ],
        word_count: 500,
        body_text: "This is test body text with enough words to avoid low content warnings. \
            The quick brown fox jumps over the lazy dog. "
            .repeat(10),
        internal_links: Vec::new(),
        external_links: Vec::new(),
        images: Vec::new(),
        schema_types: Vec::new(),
        og_tags: HashMap::new(),
        content_hash: "unique_hash_1".to_string(),
        is_https: true,
        security_headers: SecurityHeaders {
            has_hsts: true,
            has_csp: true,
            has_x_content_type_options: true,
            has_x_frame_options: true,
            has_referrer_policy: true,
            referrer_policy_value: Some("strict-origin-when-cross-origin".to_string()),
        },
        mixed_content: Vec::new(),
        insecure_forms: Vec::new(),
        url_length: url.len() as u32,
        has_parameters: false,
        has_underscores: false,
        has_uppercase: false,
        has_non_ascii: false,
        has_multiple_slashes: false,
        has_repetitive_path: false,
        title_length: Some(35),
        title_pixel_width: None,
        meta_description_length: Some(82),
        meta_description_pixel_width: None,
        title_count: 1,
        meta_description_count: 1,
        title_in_head: true,
        meta_desc_in_head: true,
        canonical_is_relative: false,
        canonical_count: 1,
        // The fixture models a clean page, whose canonical is in <head>. Every
        // production path that sets `canonical` also sets a source; leaving it
        // None here described a state the crawler cannot produce.
        canonical_source: Some(slither_core::models::page::CanonicalSource::Html),
        has_self_canonical: true,
        x_robots_tag: None,
        meta_robots_directives: Vec::new(),
        hreflang_tags: Vec::new(),
        pagination: None,
        readability_score: Some(65.0),
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
    }
}

fn make_context(pages: Vec<PageData>) -> AnalysisContext {
    AnalysisContext {
        seed_url: "https://example.com".to_string(),
        domain: "example.com".to_string(),
        sitemap_data: None,
        pages,
        robots_txt: None,
    }
}

#[test]
fn test_default_registry_has_17_analyzers() {
    let registry = AnalyzerRegistry::default_registry();
    assert_eq!(registry.analyzer_count(), 17);
}

#[test]
fn test_registry_handles_empty_input() {
    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![]);
    let issues = registry.run_all(&ctx);
    // With no pages, only sitemap-level issues can fire (no sitemap found)
    for issue in &issues {
        assert_eq!(
            issue.category,
            IssueCategory::Sitemaps,
            "Only sitemap issues expected with no pages, got: {} ({:?})",
            issue.check,
            issue.category
        );
    }
}

#[test]
fn test_clean_page_has_only_info_issues() {
    let page = make_page("https://example.com/clean");
    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page]);
    let issues = registry.run_all(&ctx);

    // A well-formed page should have no critical or warning issues
    // (only Info-level things like missing security headers, missing structured data)
    let critical = issues
        .iter()
        .filter(|i| i.severity == Severity::Critical)
        .collect::<Vec<_>>();
    let warnings = issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
        .collect::<Vec<_>>();

    assert!(
        critical.is_empty(),
        "Clean page should have no critical issues, got: {:?}",
        critical
            .iter()
            .map(|i| i.check.as_str())
            .collect::<Vec<_>>()
    );

    // Filter out expected warnings from limited test context:
    // - Sitemaps: no sitemap data in unit test
    // - Links: single-page test naturally has no inbound links (orphan_pages)
    //   and no internal outlinks (no_internal_outlinks)
    let unexpected_warnings: Vec<_> = warnings
        .iter()
        .filter(|i| i.category != IssueCategory::Sitemaps)
        .filter(|i| i.category != IssueCategory::Links)
        .collect();

    assert!(
        unexpected_warnings.is_empty(),
        "Clean page should have no unexpected warnings, got: {:?}",
        unexpected_warnings
            .iter()
            .map(|i| format!("{:?}: {}", i.category, i.check))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_missing_title_detected() {
    let mut page = make_page("https://example.com/no-title");
    page.title = None;
    page.title_length = None;
    page.title_count = 0;
    page.content_hash = "unique_no_title".to_string();

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page]);
    let issues = registry.run_all(&ctx);

    let title_issues: Vec<_> = issues
        .iter()
        .filter(|i| i.category == IssueCategory::PageTitles && i.check == "missing")
        .collect();
    assert_eq!(title_issues.len(), 1, "Should detect missing title");
    assert_eq!(title_issues[0].severity, Severity::Warning);
}

#[test]
fn test_broken_internal_link_detected() {
    let mut page = make_page("https://example.com/page");
    page.internal_links = vec![LinkData {
        url: "https://example.com/broken".to_string(),
        anchor: "Broken link".to_string(),
        nofollow: false,
    }];
    page.content_hash = "unique_broken_link".to_string();

    let mut broken = make_page("https://example.com/broken");
    broken.status = 404;
    broken.content_hash = "unique_404_page".to_string();

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page, broken]);
    let issues = registry.run_all(&ctx);

    let link_issues: Vec<_> = issues
        .iter()
        .filter(|i| i.category == IssueCategory::Links && i.check == "broken_internal")
        .collect();
    assert!(
        !link_issues.is_empty(),
        "Should detect broken internal links"
    );
}

#[test]
fn test_non_html_and_error_pages_not_flagged_for_missing_tags() {
    // A PDF asset and a 404 page must not be reported as "missing title/H1/etc".
    let mut pdf = make_page("https://example.com/doc.pdf");
    pdf.content_type = Some("application/pdf".to_string());
    pdf.title = None;
    pdf.meta_description = None;
    pdf.h1 = vec![];
    pdf.canonical = None;
    pdf.content_hash = "pdf_asset".to_string();

    let mut notfound = make_page("https://example.com/missing");
    notfound.status = 404;
    notfound.title = None;
    notfound.meta_description = None;
    notfound.h1 = vec![];
    notfound.canonical = None;
    notfound.content_hash = "notfound_page".to_string();

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![pdf, notfound]);
    let issues = registry.run_all(&ctx);

    for check in ["missing", "h1_missing", "h2_missing"] {
        for cat in [
            IssueCategory::PageTitles,
            IssueCategory::MetaDescription,
            IssueCategory::Headings,
            IssueCategory::Canonicals,
        ] {
            let flagged: Vec<_> = issues
                .iter()
                .filter(|i| i.category == cat && i.check == check)
                .flat_map(|i| i.urls.iter().map(|u| u.url.as_str()))
                .collect();
            assert!(
                !flagged.contains(&"https://example.com/doc.pdf"),
                "PDF should not be flagged for {cat:?}/{check}"
            );
            assert!(
                !flagged.contains(&"https://example.com/missing"),
                "404 should not be flagged for {cat:?}/{check}"
            );
        }
    }
}

#[test]
fn test_ordinary_redirect_not_flagged_as_loop() {
    use slither_core::models::page::RedirectHop;
    // A normal single-hop redirect (/x -> /x/) must NOT be reported as a loop.
    let mut page = make_page("https://example.com/x");
    page.redirect_chain = Some(vec![RedirectHop {
        status: 301,
        url: "https://example.com/x".to_string(),
    }]);
    page.content_hash = "redirect_single".to_string();

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page]);
    let issues = registry.run_all(&ctx);

    assert!(
        !issues.iter().any(|i| i.check == "internal_redirect_loop"),
        "an ordinary single redirect must not be flagged as a loop"
    );
}

#[test]
fn test_genuine_redirect_loop_detected() {
    use slither_core::models::page::RedirectHop;
    // A chain that revisits a URL is a genuine loop. /a and /b bounce off each
    // other until the hop budget runs out, so the record is filed at the last
    // URL the crawler requested — which is also the last hop, never a URL that
    // served anything.
    let mut page = make_page("https://example.com/b");
    page.redirect_chain = Some(vec![
        RedirectHop {
            status: 302,
            url: "https://example.com/a".to_string(),
        },
        RedirectHop {
            status: 302,
            url: "https://example.com/b".to_string(),
        },
        RedirectHop {
            status: 302,
            url: "https://example.com/a".to_string(),
        },
        RedirectHop {
            status: 302,
            url: "https://example.com/b".to_string(),
        },
    ]);
    page.content_hash = "redirect_loop".to_string();

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page]);
    let issues = registry.run_all(&ctx);

    let loops: Vec<_> = issues
        .iter()
        .filter(|i| i.check == "internal_redirect_loop")
        .collect();
    assert_eq!(
        loops.len(),
        1,
        "a chain revisiting a URL should be flagged as a loop"
    );
    assert_eq!(
        loops[0].urls[0].url, "https://example.com/a",
        "the loop belongs to the URL that enters it, not the hop we gave up on"
    );

    // The hop path appends the URL that served the response — but a chain that
    // never resolved ends on the last hop, and printing it twice reads as a hop
    // that is not there.
    let chain = issues
        .iter()
        .find(|i| i.check == "internal_redirect_chain")
        .expect("a 4-hop chain is also a long chain");
    let detail = chain.urls[0].detail.as_deref().unwrap_or_default();
    assert!(
        detail.ends_with("/a -> https://example.com/b"),
        "an unresolved chain must not repeat its last hop, got: {detail}"
    );
}

#[test]
fn test_5xx_server_error_detected() {
    let mut page = make_page("https://example.com/error");
    page.status = 500;
    page.content_hash = "unique_500".to_string();

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page]);
    let issues = registry.run_all(&ctx);

    let error_issues: Vec<_> = issues
        .iter()
        .filter(|i| {
            i.category == IssueCategory::ResponseCodes && i.check == "internal_server_error"
        })
        .collect();
    assert_eq!(error_issues.len(), 1, "Should detect 5xx server error");
    assert_eq!(error_issues[0].severity, Severity::Critical);
}

#[test]
fn test_multiple_pages_with_duplicate_titles() {
    let page_a = make_page("https://example.com/a");
    let mut page_b = make_page("https://example.com/b");
    page_b.content_hash = "unique_hash_2".to_string();
    // Both pages have the same title: "Test Page Title That Is Long Enough"

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page_a, page_b]);
    let issues = registry.run_all(&ctx);

    let dup_titles: Vec<_> = issues
        .iter()
        .filter(|i| i.category == IssueCategory::PageTitles && i.check == "duplicate")
        .collect();
    assert_eq!(dup_titles.len(), 1, "Should detect duplicate titles");
}

/// A-DUPGATE: error pages share boilerplate titles/descriptions/H1s that the
/// site owner cannot de-duplicate. Only indexable HTML should be grouped.
#[test]
fn test_duplicate_checks_ignore_error_pages() {
    let mut page_a = make_page("https://example.com/missing-1");
    let mut page_b = make_page("https://example.com/missing-2");
    for p in [&mut page_a, &mut page_b] {
        p.status = 404;
        p.title = Some("404 Not Found".to_string());
        p.meta_description = Some("The page you requested could not be found here".to_string());
        p.h1 = vec!["Page Not Found".to_string()];
    }
    page_b.content_hash = "unique_hash_2".to_string();

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page_a, page_b]);
    let issues = registry.run_all(&ctx);

    for (category, check) in [
        (IssueCategory::PageTitles, "duplicate"),
        (IssueCategory::MetaDescription, "duplicate"),
        (IssueCategory::Headings, "h1_duplicate"),
    ] {
        assert!(
            !issues
                .iter()
                .any(|i| i.category == category && i.check == check),
            "404 pages must not produce a {check} issue"
        );
    }
}

/// A noindex page cannot cause duplicate content, because it never enters the
/// index.
#[test]
fn test_duplicate_titles_ignore_noindex_pages() {
    let page_a = make_page("https://example.com/a");
    let mut page_b = make_page("https://example.com/b");
    page_b.content_hash = "unique_hash_2".to_string();
    page_b.meta_robots_directives = vec!["noindex".to_string()];

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page_a, page_b]);
    let issues = registry.run_all(&ctx);

    assert!(
        !issues
            .iter()
            .any(|i| i.category == IssueCategory::PageTitles && i.check == "duplicate"),
        "a noindex page must not create a duplicate-title pair"
    );
}

/// A-ORPHANSELF: a page linked only by its own nav/logo self-link has no real
/// inbound links and is still an orphan.
#[test]
fn test_self_link_does_not_rescue_an_orphan_page() {
    let seed = make_page("https://example.com/");
    let mut orphan = make_page("https://example.com/orphan");
    orphan.content_hash = "unique_hash_2".to_string();
    orphan.internal_links = vec![LinkData {
        url: "https://example.com/orphan".to_string(),
        anchor: "Home".to_string(),
        nofollow: false,
    }];

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![seed, orphan]);
    let issues = registry.run_all(&ctx);

    let orphans: Vec<_> = issues
        .iter()
        .filter(|i| i.category == IssueCategory::Links && i.check == "orphan_pages")
        .collect();
    assert_eq!(
        orphans.len(),
        1,
        "self-linked page should still be an orphan"
    );
    assert!(orphans[0]
        .urls
        .iter()
        .any(|u| u.url == "https://example.com/orphan"));
}

// ---------------------------------------------------------------------------
// Redirect-aware URL joins.
//
// A page is recorded at the URL that finally served it, with the requested
// address at `redirect_chain[0].url`. Every check that joins an `<a href>`
// against a crawled page must resolve the href through the redirect first, or
// it silently misses every page reached via a redirect.
// ---------------------------------------------------------------------------

/// B2: a link that 301s into a 404 is a broken link. The href names `/moved`,
/// which has no record of its own — the 404 was filed at `/gone` — so the
/// unresolved lookup found nothing and the page's only bad link produced no
/// Critical finding at all.
#[test]
fn test_link_redirecting_into_a_404_is_reported_broken() {
    let mut home = make_page("https://example.com/");
    home.depth = 0;
    home.content_hash = "unique_home".to_string();
    home.internal_links = vec![LinkData {
        url: "https://example.com/moved".to_string(),
        anchor: "Relocated document".to_string(),
        nofollow: false,
    }];

    // /moved 301s to /gone, which 404s, so the record lives at /gone.
    let mut gone = make_page("https://example.com/gone");
    gone.status = 404;
    gone.content_hash = "unique_gone".to_string();
    gone.redirect_chain = Some(vec![slither_core::models::page::RedirectHop {
        status: 301,
        url: "https://example.com/moved".to_string(),
    }]);

    let registry = AnalyzerRegistry::default_registry();
    let issues = registry.run_all(&make_context(vec![home, gone]));

    let broken: Vec<_> = issues
        .iter()
        .filter(|i| i.category == IssueCategory::Links && i.check == "broken_internal")
        .collect();
    assert_eq!(
        broken.len(),
        1,
        "a link that redirects into a 404 is broken and must be reported"
    );
    assert_eq!(broken[0].severity, Severity::Critical);
    assert_eq!(broken[0].urls[0].url, "https://example.com/");

    let detail = broken[0].urls[0].detail.as_deref().unwrap_or_default();
    assert!(
        detail.contains("https://example.com/moved"),
        "the detail must name the href the author has to change, got: {detail}"
    );
    assert!(
        detail.contains("https://example.com/gone") && detail.contains("404"),
        "the detail must name where the link lands and why it is broken, got: {detail}"
    );
}

/// A link that redirects to a healthy page is not broken — the resolution must
/// not turn every redirect into a Critical finding.
#[test]
fn test_link_redirecting_into_a_200_is_not_reported_broken() {
    let mut home = make_page("https://example.com/");
    home.depth = 0;
    home.content_hash = "unique_home".to_string();
    home.internal_links = vec![LinkData {
        url: "https://example.com/old".to_string(),
        anchor: "Alpha section".to_string(),
        nofollow: false,
    }];

    let mut new = make_page("https://example.com/new");
    new.content_hash = "unique_new".to_string();
    new.redirect_chain = Some(vec![slither_core::models::page::RedirectHop {
        status: 301,
        url: "https://example.com/old".to_string(),
    }]);

    let registry = AnalyzerRegistry::default_registry();
    let issues = registry.run_all(&make_context(vec![home, new]));

    assert!(
        !issues.iter().any(|i| i.check == "broken_internal"),
        "a link that redirects to a 200 is not broken"
    );
}

/// B3: a page linked only through a redirect has an inbound link. Joining the
/// href `/old` against `page.url` (`/new`) matched nothing, so the destination
/// was reported as an orphan "with no incoming internal links" while the
/// homepage linked straight at it.
#[test]
fn test_redirect_destination_is_not_an_orphan() {
    let mut home = make_page("https://example.com/");
    home.depth = 0;
    home.content_hash = "unique_home".to_string();
    home.internal_links = vec![LinkData {
        url: "https://example.com/old-a".to_string(),
        anchor: "Alpha section".to_string(),
        nofollow: false,
    }];

    let mut new_a = make_page("https://example.com/new-a");
    new_a.content_hash = "unique_new_a".to_string();
    new_a.redirect_chain = Some(vec![slither_core::models::page::RedirectHop {
        status: 301,
        url: "https://example.com/old-a".to_string(),
    }]);

    let registry = AnalyzerRegistry::default_registry();
    let issues = registry.run_all(&make_context(vec![home, new_a]));

    let orphaned: Vec<&str> = issues
        .iter()
        .filter(|i| i.check == "orphan_pages")
        .flat_map(|i| i.urls.iter().map(|u| u.url.as_str()))
        .collect();
    assert!(
        !orphaned.contains(&"https://example.com/new-a"),
        "a page linked through a 301 is not an orphan, got orphans: {orphaned:?}"
    );
}

/// A self-link that only reaches the page through a redirect is still a self
/// link and must not rescue the page from the orphan check.
#[test]
fn test_self_link_through_a_redirect_does_not_rescue_an_orphan() {
    let seed = make_page("https://example.com/");
    let mut orphan = make_page("https://example.com/new-a");
    orphan.content_hash = "unique_hash_2".to_string();
    // The page's own nav points at the pre-redirect address of itself.
    orphan.internal_links = vec![LinkData {
        url: "https://example.com/old-a".to_string(),
        anchor: "Alpha".to_string(),
        nofollow: false,
    }];
    orphan.redirect_chain = Some(vec![slither_core::models::page::RedirectHop {
        status: 301,
        url: "https://example.com/old-a".to_string(),
    }]);

    let registry = AnalyzerRegistry::default_registry();
    let issues = registry.run_all(&make_context(vec![seed, orphan]));

    let orphaned: Vec<&str> = issues
        .iter()
        .filter(|i| i.check == "orphan_pages")
        .flat_map(|i| i.urls.iter().map(|u| u.url.as_str()))
        .collect();
    assert!(
        orphaned.contains(&"https://example.com/new-a"),
        "a self-link through a redirect is still a self-link, got orphans: {orphaned:?}"
    );
}

/// B4: a 404 reached only behind a 301 is fixed by repointing links at the
/// *redirecting* address, which appears nowhere in the page record.
#[test]
fn test_client_error_names_the_url_that_redirected_into_it() {
    let mut gone = make_page("https://example.com/gone");
    gone.status = 404;
    gone.content_hash = "unique_gone".to_string();
    gone.redirect_chain = Some(vec![slither_core::models::page::RedirectHop {
        status: 301,
        url: "https://example.com/moved".to_string(),
    }]);

    let registry = AnalyzerRegistry::default_registry();
    let issues = registry.run_all(&make_context(vec![gone]));

    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.check == "internal_client_error")
        .collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].urls[0].url, "https://example.com/gone");
    let detail = errors[0].urls[0].detail.as_deref().unwrap_or_default();
    assert!(
        detail.contains("https://example.com/moved"),
        "the 404 detail must name the URL that redirected into it, got: {detail}"
    );
}

/// C-1HOP: a redirecting URL never gets a record of its own — the page is
/// filed at the URL that finally served it, with the requested address at
/// `redirect_chain[0]`. So redirects must be detected from the chain, not the
/// status, and reported against `chain[0].url`.
///
/// The fixture below is the load-bearing part of this test: an earlier version
/// set `page.url == chain[0].url`, which is a shape the crawler cannot produce.
/// It passed against code that named the redirect *destination* as the URL that
/// redirects, because with the two equal there was nothing to tell apart.
#[test]
fn test_single_hop_redirect_is_reported() {
    // https://example.com/old 301s to https://example.com/home.
    let mut page = make_page("https://example.com/home");
    page.redirect_chain = Some(vec![slither_core::models::page::RedirectHop {
        status: 301,
        url: "https://example.com/old".to_string(),
    }]);

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page]);
    let issues = registry.run_all(&ctx);

    let redirects: Vec<_> = issues
        .iter()
        .filter(|i| i.category == IssueCategory::ResponseCodes && i.check == "internal_redirect")
        .collect();
    assert_eq!(redirects.len(), 1, "single-hop redirect should be reported");

    let flagged: Vec<&str> = redirects[0].urls.iter().map(|u| u.url.as_str()).collect();
    assert_eq!(
        flagged,
        vec!["https://example.com/old"],
        "the URL that 301s must be named, not the destination it lands on"
    );
    let detail = redirects[0].urls[0].detail.as_deref().unwrap_or_default();
    assert!(
        detail.contains("https://example.com/home"),
        "the detail must name where the redirect goes, got: {detail}"
    );

    // A 2-hop chain belongs to the long-chain check only — no double reporting.
    let mut chained = make_page("https://example.com/final");
    chained.content_hash = "unique_hash_3".to_string();
    chained.redirect_chain = Some(vec![
        slither_core::models::page::RedirectHop {
            status: 301,
            url: "https://example.com/old".to_string(),
        },
        slither_core::models::page::RedirectHop {
            status: 301,
            url: "https://example.com/mid".to_string(),
        },
    ]);
    let issues = registry.run_all(&make_context(vec![chained]));
    assert!(
        !issues.iter().any(|i| i.check == "internal_redirect"),
        "multi-hop chains are reported by the long-chain check only"
    );
    let chain_issue = issues
        .iter()
        .find(|i| i.check == "internal_redirect_chain")
        .expect("a 2-hop chain should be reported");
    assert_eq!(
        chain_issue.urls[0].url, "https://example.com/old",
        "a chain is reported against the URL that starts it"
    );
    let detail = chain_issue.urls[0].detail.as_deref().unwrap_or_default();
    assert!(
        detail.contains("https://example.com/final"),
        "the hop path must end at the destination, got: {detail}"
    );
}

/// 401/403/429 are usually deliberate or self-inflicted, not broken pages.
#[test]
fn test_access_restricted_codes_are_not_critical_4xx() {
    let mut page = make_page("https://example.com/admin");
    page.status = 403;

    let registry = AnalyzerRegistry::default_registry();
    let issues = registry.run_all(&make_context(vec![page]));

    assert!(
        !issues.iter().any(|i| i.check == "internal_client_error"),
        "403 must not be reported as a broken 4xx page"
    );
    let restricted: Vec<_> = issues
        .iter()
        .filter(|i| i.check == "internal_access_restricted")
        .collect();
    assert_eq!(restricted.len(), 1);
    assert_eq!(restricted[0].severity, Severity::Warning);
}

/// A-HREFNORM: hreflang joins must normalize both sides, or a reciprocal pair
/// written with different host casing / trailing slash looks broken.
#[test]
fn test_hreflang_return_links_survive_url_formatting_differences() {
    use slither_core::models::page::HreflangTag;

    let mut en = make_page("https://example.com/en/");
    let mut de = make_page("https://example.com/de/");
    de.content_hash = "unique_hash_2".to_string();

    // Each points at the other, but with a different textual form.
    en.hreflang_tags = vec![
        HreflangTag {
            lang: "en".to_string(),
            url: "https://example.com/en/".to_string(),
        },
        HreflangTag {
            lang: "de".to_string(),
            url: "https://EXAMPLE.com/de/".to_string(),
        },
    ];
    de.hreflang_tags = vec![
        HreflangTag {
            lang: "de".to_string(),
            url: "https://example.com/de/".to_string(),
        },
        HreflangTag {
            lang: "en".to_string(),
            url: "https://EXAMPLE.com/en/".to_string(),
        },
    ];

    let registry = AnalyzerRegistry::default_registry();
    let issues = registry.run_all(&make_context(vec![en, de]));

    assert!(
        !issues
            .iter()
            .any(|i| i.category == IssueCategory::Hreflang && i.check == "missing_return_links"),
        "reciprocal hreflang must not be reported as missing"
    );
    assert!(
        !issues
            .iter()
            .any(|i| i.category == IssueCategory::Hreflang && i.check == "missing_self_reference"),
        "self-reference in a different textual form must be recognized"
    );
}

/// A noindex page never enters the index, so it cannot dilute ranking signals.
/// Pairing one with an indexable page is not a duplicate-content finding.
#[test]
fn test_duplicate_content_ignores_noindex_pages() {
    // make_page gives every page the same content hash, so these two are an
    // exact-duplicate pair until one is excluded from the index.
    let page_a = make_page("https://example.com/a");
    let mut page_b = make_page("https://example.com/b");
    page_b.meta_robots_directives = vec!["noindex".to_string()];

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page_a, page_b]);
    let issues = registry.run_all(&ctx);

    assert!(
        !issues
            .iter()
            .any(|i| i.category == IssueCategory::Content && i.check == "exact_duplicate"),
        "a noindex page must not create a duplicate-content pair"
    );
}

/// The control for the test above: two indexable pages sharing content are
/// still a real duplicate.
#[test]
fn test_duplicate_content_still_reported_between_indexable_pages() {
    let page_a = make_page("https://example.com/a");
    let page_b = make_page("https://example.com/b");

    let registry = AnalyzerRegistry::default_registry();
    let ctx = make_context(vec![page_a, page_b]);
    let issues = registry.run_all(&ctx);

    assert!(
        issues
            .iter()
            .any(|i| i.category == IssueCategory::Content && i.check == "exact_duplicate"),
        "two indexable pages with identical content are a duplicate"
    );
}

// ---------------------------------------------------------------------------
// Edge-case sweep regressions (analyzers)
// ---------------------------------------------------------------------------

mod sweep_regressions {
    use super::*;
    use slither_core::models::page::HreflangTag;

    fn issues_for(pages: Vec<PageData>) -> Vec<slither_core::models::issue::Issue> {
        AnalyzerRegistry::default_registry().run_all(&make_context(pages))
    }

    fn checks(pages: Vec<PageData>) -> Vec<String> {
        issues_for(pages).into_iter().map(|i| i.check).collect()
    }

    /// Google's structured-data policies document multiple items on one page as
    /// supported, both nested and as separate blocks. Flagging NewsArticle +
    /// VideoObject — or the Yoast + WP Recipe Maker BlogPosting + Recipe stack —
    /// asserted a rule Google does not have.
    #[test]
    fn co_occurring_schema_types_are_not_an_issue() {
        let mut p = make_page("https://example.com/article-with-video");
        p.schema_types = vec!["NewsArticle".to_string(), "VideoObject".to_string()];
        assert!(
            !checks(vec![p]).contains(&"multiple_conflicting".to_string()),
            "co-occurring primary types are valid markup"
        );
    }

    /// `en-UK` is the most common real hreflang error — the ISO 3166-1 code for
    /// the United Kingdom is GB. Shape-only validation accepted it silently.
    #[test]
    fn invalid_region_subtags_are_rejected() {
        let mut p = make_page("https://example.com/en");
        p.hreflang_tags = vec![
            HreflangTag {
                lang: "en-UK".into(),
                url: "https://example.com/uk".into(),
            },
            HreflangTag {
                lang: "en-EU".into(),
                url: "https://example.com/eu".into(),
            },
        ];
        assert!(checks(vec![p]).contains(&"incorrect_lang_codes".to_string()));
    }

    /// Real region and script subtags must still pass, or the fix would be a
    /// false-positive machine of its own.
    #[test]
    fn valid_region_and_script_subtags_are_accepted() {
        let mut p = make_page("https://example.com/en");
        p.hreflang_tags = vec![
            HreflangTag {
                lang: "en-GB".into(),
                url: "https://example.com/en".into(),
            },
            HreflangTag {
                lang: "es-419".into(),
                url: "https://example.com/es".into(),
            },
            HreflangTag {
                lang: "zh-Hant".into(),
                url: "https://example.com/zh".into(),
            },
            HreflangTag {
                lang: "sr-Latn-RS".into(),
                url: "https://example.com/sr".into(),
            },
        ];
        assert!(!checks(vec![p]).contains(&"incorrect_lang_codes".to_string()));
    }

    /// Google requires each version to list itself, so a page's own
    /// self-reference is never the bad target — the page pointing at it is.
    #[test]
    fn a_pages_own_hreflang_self_reference_is_not_a_bad_target() {
        let mut noindexed = make_page("https://example.com/it");
        noindexed.meta_robots = Some("noindex".to_string());
        noindexed.hreflang_tags = vec![HreflangTag {
            lang: "it".into(),
            url: "https://example.com/it".into(),
        }];

        let flagged: Vec<String> = issues_for(vec![noindexed])
            .into_iter()
            .filter(|i| i.check == "noindex_urls")
            .flat_map(|i| i.urls.into_iter().map(|u| u.url))
            .collect();
        assert!(
            !flagged.contains(&"https://example.com/it".to_string()),
            "a page must not be reported for pointing hreflang at itself"
        );
    }

    /// Reciprocity is checked on URLs only, so partners that disagree about a
    /// page's language satisfied it while the cluster was self-contradictory.
    #[test]
    fn partners_disagreeing_about_a_language_are_reported() {
        let mut a = make_page("https://example.com/a");
        a.hreflang_tags = vec![
            HreflangTag {
                lang: "en".into(),
                url: "https://example.com/a".into(),
            },
            HreflangTag {
                lang: "de".into(),
                url: "https://example.com/b".into(),
            },
        ];
        let mut b = make_page("https://example.com/b");
        b.content_hash = "unique_hash_2".to_string();
        b.hreflang_tags = vec![
            HreflangTag {
                lang: "en".into(),
                url: "https://example.com/a".into(),
            },
            HreflangTag {
                lang: "fr".into(),
                url: "https://example.com/b".into(),
            },
        ];
        assert!(checks(vec![a, b]).contains(&"conflicting_lang_declarations".to_string()));
    }

    /// A canonical outside <head> is ignored by Google, so it must not be
    /// treated as authoritative — it previously reported the page as
    /// canonicalised and could raise a critical "canonical to non-indexable".
    #[test]
    fn a_canonical_outside_head_is_reported_and_not_honoured() {
        let mut p = make_page("https://example.com/body-canonical");
        p.canonical = Some("https://example.com/elsewhere".to_string());
        p.canonical_source = None; // found in <body>
        p.has_self_canonical = false;

        let found = checks(vec![p]);
        assert!(found.contains(&"canonical_outside_head".to_string()));
        assert!(
            !found.contains(&"canonicalised".to_string()),
            "an ignored canonical must not be reported as canonicalising the page"
        );
    }

    /// A -> B -> A is unresolvable: no page declares itself canonical, so the
    /// annotation is discarded. It used to look identical to a correct one.
    #[test]
    fn canonical_loops_are_detected() {
        let mut a = make_page("https://example.com/loop-a");
        a.canonical = Some("https://example.com/loop-b".to_string());
        a.canonical_source = Some(slither_core::models::page::CanonicalSource::Html);
        a.has_self_canonical = false;
        let mut b = make_page("https://example.com/loop-b");
        b.content_hash = "unique_hash_2".to_string();
        b.canonical = Some("https://example.com/loop-a".to_string());
        b.canonical_source = Some(slither_core::models::page::CanonicalSource::Html);
        b.has_self_canonical = false;

        assert!(checks(vec![a, b]).contains(&"canonical_loop".to_string()));
    }

    /// `X-Robots-Tag: bingbot: noindex` scopes the directive to Bingbot. For
    /// Google the page reads index,follow — it is neither noindexed nor in
    /// conflict, and treating it as both invented a maximum-severity finding.
    #[test]
    fn a_crawler_scoped_x_robots_tag_is_not_applied_to_google() {
        let mut p = make_page("https://example.com/bing-only");
        p.meta_robots_directives = vec!["index".to_string(), "follow".to_string()];
        p.x_robots_tag = Some("bingbot: noindex".to_string());

        let found = checks(vec![p]);
        assert!(!found.contains(&"conflicting_directives".to_string()));
        assert!(!found.contains(&"noindex".to_string()));
    }

    /// Google documents `none` as noindex,nofollow and `all` as index,follow, so
    /// index+none is a real contradiction the check used to miss.
    #[test]
    fn none_and_all_aliases_are_expanded_when_detecting_conflicts() {
        let mut p = make_page("https://example.com/none-vs-index");
        p.meta_robots_directives = vec!["index".to_string(), "follow".to_string()];
        p.x_robots_tag = Some("none".to_string());
        assert!(checks(vec![p]).contains(&"conflicting_directives".to_string()));
    }

    /// A follow/nofollow mismatch leaves the page fully indexed, so it must be
    /// a separate, far cheaper finding than an index/noindex contradiction.
    #[test]
    fn a_follow_conflict_is_separated_from_an_index_conflict() {
        let mut p = make_page("https://example.com/follow-conflict");
        p.meta_robots_directives = vec!["index".to_string(), "follow".to_string()];
        p.x_robots_tag = Some("nofollow".to_string());

        let found = checks(vec![p]);
        assert!(found.contains(&"conflicting_follow_directives".to_string()));
        assert!(
            !found.contains(&"conflicting_directives".to_string()),
            "a follow mismatch is not an indexability contradiction"
        );
    }

    /// A record carrying a redirect chain *is* the destination of that
    /// redirect, reached from `chain[0].url` — one page, not two. Comparing its
    /// body against another record produced false duplicate-content findings for
    /// a redirect the site had implemented correctly (svelte.dev's
    /// /docs/svelte 307 to /docs/svelte/overview).
    ///
    /// The fixture used to put the *requested* URL in `page.url` as well as in
    /// the chain, which the crawler never produces; the shape below is the real
    /// one, with the destination in `page.url`.
    #[test]
    fn a_page_reached_through_a_redirect_is_not_a_duplicate() {
        // /old 301s to /new, so the record is filed at /new.
        let mut redirected = make_page("https://example.com/new");
        redirected.redirect_chain = Some(vec![slither_core::models::page::RedirectHop {
            status: 301,
            url: "https://example.com/old".to_string(),
        }]);
        redirected.content_hash = "shared_hash".to_string();
        let mut other = make_page("https://example.com/other");
        other.content_hash = "shared_hash".to_string();

        assert!(
            !checks(vec![redirected, other]).contains(&"exact_duplicate".to_string()),
            "a page reached through a redirect is not a second copy of itself"
        );
    }

    /// The redirect-path details are UI text too, and a chain of six 2 KB URLs
    /// is a 12 KB table cell — the same unbounded join every other detail in
    /// the codebase is capped against.
    #[test]
    fn redirect_path_details_are_bounded() {
        let long = |n: usize| format!("https://example.com/{}", "s".repeat(1_800) + &n.to_string());
        let mut p = make_page("https://example.com/end-of-a-long-chain");
        p.redirect_chain = Some(
            (0..6)
                .map(|i| slither_core::models::page::RedirectHop {
                    status: 301,
                    url: long(i),
                })
                .collect(),
        );

        for issue in issues_for(vec![p]) {
            for u in &issue.urls {
                if let Some(d) = &u.detail {
                    assert!(
                        d.len() < 600,
                        "detail for {} is {} bytes",
                        issue.check,
                        d.len()
                    );
                }
            }
        }
    }

    /// Issue details are UI text. Joining every match on a page produced a
    /// single 1.2 MB span in the report from one link-heavy page.
    #[test]
    fn issue_details_are_bounded() {
        let mut p = make_page("https://example.com/many-links");
        p.internal_links = (0..5_000)
            .map(|i| LinkData {
                url: format!("https://example.com/facet/{i}"),
                anchor: "x".to_string(),
                nofollow: true,
            })
            .collect();

        for issue in issues_for(vec![p]) {
            for u in &issue.urls {
                if let Some(d) = &u.detail {
                    assert!(
                        d.len() < 600,
                        "detail for {} is {} bytes: {}",
                        issue.check,
                        d.len(),
                        &d[..120.min(d.len())]
                    );
                }
            }
        }
    }
}
