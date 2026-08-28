use crate::analysis::{AnalysisContext, AnalyzerRegistry};
#[cfg(feature = "cloudflare")]
use crate::cloudflare::render::render_page;
/// The rendering client the rendered/compare modes need.
///
/// Without the `cloudflare` feature this is an uninhabited type, so
/// `Option<&RenderClient>` can only ever be `None` — the signature stays the
/// same for callers, and "no rendering backend compiled in" is unrepresentable
/// rather than a runtime state to guard.
#[cfg(feature = "cloudflare")]
pub type RenderClient = crate::cloudflare::CloudflareClient;
#[cfg(not(feature = "cloudflare"))]
pub enum RenderClient {}
use crate::crawler::fetcher::Fetcher;
use crate::crawler::parser::parse_html;
use crate::crawler::{
    apply_link_header_canonical, compute_url_metadata, extract_header_value,
    extract_security_headers,
};
use crate::models::issue::Issue;
use crate::models::page::PageData;
use anyhow::Result;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Determines what kind of inspection to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectMode {
    /// Fetch raw HTML only (no JS rendering).
    Static,
    /// Render via Cloudflare Browser Rendering.
    Rendered,
    /// Fetch both static and rendered, then compare.
    Compare,
}

/// The result of an inspect operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResult {
    pub url: String,
    pub mode: String,
    pub page: PageData,
    pub issues: Vec<Issue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub static_page: Option<PageData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub static_issues: Option<Vec<Issue>>,
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// Format a human-readable single-page inspection report for terminal output.
pub fn format_inspect_output(url: &str, page: &PageData, issues: &[Issue]) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!("\n  slither inspect  {}\n", url));
    out.push_str(&format!("  {}\n\n", "-".repeat(60)));

    // --- Page Data ---
    out.push_str("  Page Data\n");
    out.push_str(&format!(
        "    Title              {}\n",
        page.title.as_deref().unwrap_or("(none)")
    ));
    out.push_str(&format!(
        "    Title length       {} chars / {} px\n",
        page.title_length.unwrap_or(0),
        page.title_pixel_width.unwrap_or(0)
    ));
    out.push_str(&format!(
        "    Meta description   {}\n",
        truncate(page.meta_description.as_deref().unwrap_or("(none)"), 60)
    ));
    out.push_str(&format!(
        "    Desc length        {} chars / {} px\n",
        page.meta_description_length.unwrap_or(0),
        page.meta_description_pixel_width.unwrap_or(0)
    ));
    out.push_str(&format!(
        "    Canonical          {}\n",
        page.canonical.as_deref().unwrap_or("(none)")
    ));
    // The most consequential thing this command can tell you about a page, and
    // it was previously absent — a header-delivered noindex was invisible here.
    //
    // `analysis::is_indexable_html` is the shared rule (2xx HTML, not noindex).
    // Testing only the robots directives printed "Indexable  yes" on the line
    // directly above "Status  404".
    let directives = page.effective_robots_directives();
    out.push_str(&format!(
        "    Indexable          {}{}\n",
        yes_no(crate::analysis::is_indexable_html(page)),
        if directives.is_empty() {
            String::new()
        } else {
            format!("  ({})", directives.join(", "))
        }
    ));
    out.push_str(&format!("    Status             {}\n", page.status));
    out.push_str(&format!(
        "    Response time      {} ms\n",
        page.response_time_ms
    ));
    out.push_str(&format!("    Word count         {}\n", page.word_count));
    out.push_str(&format!(
        "    Readability        {}\n",
        page.readability_score
            .map(|s| format!("{:.1}", s))
            .unwrap_or_else(|| "n/a".to_string())
    ));
    out.push('\n');

    // --- Links & Media ---
    let images_with_alt = page.images.iter().filter(|i| i.alt.is_some()).count();
    out.push_str("  Links & Media\n");
    out.push_str(&format!(
        "    Internal links     {}\n",
        page.internal_links.len()
    ));
    out.push_str(&format!(
        "    External links     {}\n",
        page.external_links.len()
    ));
    out.push_str(&format!(
        "    Images             {} ({} with alt)\n",
        page.images.len(),
        images_with_alt
    ));
    out.push_str(&format!(
        "    Schema types       {}\n",
        if page.schema_types.is_empty() {
            "none".to_string()
        } else {
            page.schema_types.join(", ")
        }
    ));
    out.push('\n');

    // --- Security ---
    out.push_str("  Security\n");
    out.push_str(&format!(
        "    HTTPS              {}\n",
        yes_no(page.is_https)
    ));
    out.push_str(&format!(
        "    HSTS               {}\n",
        yes_no(page.security_headers.has_hsts)
    ));
    out.push_str(&format!(
        "    CSP                {}\n",
        yes_no(page.security_headers.has_csp)
    ));
    out.push_str(&format!(
        "    X-Frame-Options    {}\n",
        yes_no(page.security_headers.has_x_frame_options)
    ));
    out.push('\n');

    // --- Issues ---
    out.push_str(&format!("  Issues ({})\n", issues.len()));
    if issues.is_empty() {
        out.push_str("    None found\n");
    } else {
        for issue in issues {
            let icon = severity_icon(issue.severity);
            out.push_str(&format!("    {} {}\n", icon, issue.display_name));
        }
    }
    out.push('\n');

    out
}

/// Format a side-by-side comparison table of static vs rendered page data.
pub fn format_compare_table(
    url: &str,
    static_page: &PageData,
    rendered_page: &PageData,
    static_issues: &[Issue],
    rendered_issues: &[Issue],
) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!("\n  slither inspect compare  {}\n", url));
    out.push_str(&format!("  {}\n\n", "-".repeat(60)));

    // Table header
    let col_metric = 22;
    let col_val = 16;

    out.push_str(&format!(
        "  {:width_m$} {:width_v$} {:width_v$} {}\n",
        "Metric",
        "Static",
        "Rendered",
        "Delta",
        width_m = col_metric,
        width_v = col_val,
    ));
    out.push_str(&format!(
        "  {}\n",
        "\u{2500}".repeat(col_metric + col_val * 2 + 12)
    ));

    // Rows
    let rows: Vec<(&str, String, String, String)> = vec![
        (
            "Title",
            present_check(static_page.title.is_some()),
            present_check(rendered_page.title.is_some()),
            delta_check(static_page.title.is_some(), rendered_page.title.is_some()),
        ),
        (
            "Meta description",
            present_check(static_page.meta_description.is_some()),
            present_check(rendered_page.meta_description.is_some()),
            delta_check(
                static_page.meta_description.is_some(),
                rendered_page.meta_description.is_some(),
            ),
        ),
        (
            "Word count",
            static_page.word_count.to_string(),
            rendered_page.word_count.to_string(),
            delta_i64(
                static_page.word_count as i64,
                rendered_page.word_count as i64,
            ),
        ),
        (
            "Internal links",
            static_page.internal_links.len().to_string(),
            rendered_page.internal_links.len().to_string(),
            delta_i64(
                static_page.internal_links.len() as i64,
                rendered_page.internal_links.len() as i64,
            ),
        ),
        (
            "External links",
            static_page.external_links.len().to_string(),
            rendered_page.external_links.len().to_string(),
            delta_i64(
                static_page.external_links.len() as i64,
                rendered_page.external_links.len() as i64,
            ),
        ),
        (
            "Images",
            static_page.images.len().to_string(),
            rendered_page.images.len().to_string(),
            delta_i64(
                static_page.images.len() as i64,
                rendered_page.images.len() as i64,
            ),
        ),
        (
            "Schema types",
            static_page.schema_types.len().to_string(),
            rendered_page.schema_types.len().to_string(),
            delta_i64(
                static_page.schema_types.len() as i64,
                rendered_page.schema_types.len() as i64,
            ),
        ),
        (
            "H1 tags",
            static_page.h1.len().to_string(),
            rendered_page.h1.len().to_string(),
            delta_i64(static_page.h1.len() as i64, rendered_page.h1.len() as i64),
        ),
        (
            "Issues",
            static_issues.len().to_string(),
            rendered_issues.len().to_string(),
            delta_i64(static_issues.len() as i64, rendered_issues.len() as i64),
        ),
    ];

    for (metric, static_val, rendered_val, delta) in &rows {
        out.push_str(&format!(
            "  {:width_m$} {:width_v$} {:width_v$} {}\n",
            metric,
            static_val,
            rendered_val,
            delta,
            width_m = col_metric,
            width_v = col_val,
        ));
    }

    out.push('\n');
    out
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

/// Run an inspect operation against a single URL.
pub async fn run_inspect(
    url: &str,
    mode: InspectMode,
    wait_for: Option<&str>,
    cf_client: Option<&RenderClient>,
) -> Result<InspectResult> {
    match mode {
        InspectMode::Static => {
            let (page, issues) = fetch_and_analyze(url).await?;
            Ok(InspectResult {
                url: url.to_string(),
                mode: "static".to_string(),
                page,
                issues,
                static_page: None,
                static_issues: None,
            })
        }
        InspectMode::Rendered => {
            #[cfg(feature = "cloudflare")]
            {
                let client = cf_client.ok_or_else(|| {
                    anyhow::anyhow!("Cloudflare client required for rendered mode")
                })?;
                let (page, issues) = render_and_analyze(client, url, wait_for).await?;
                Ok(InspectResult {
                    url: url.to_string(),
                    mode: "rendered".to_string(),
                    page,
                    issues,
                    static_page: None,
                    static_issues: None,
                })
            }
            #[cfg(not(feature = "cloudflare"))]
            {
                let _ = (cf_client, wait_for);
                anyhow::bail!("rendered mode requires the 'cloudflare' feature")
            }
        }
        InspectMode::Compare => {
            #[cfg(feature = "cloudflare")]
            {
                let client = cf_client.ok_or_else(|| {
                    anyhow::anyhow!("Cloudflare client required for compare mode")
                })?;
                let (static_result, rendered_result) = tokio::join!(
                    fetch_and_analyze(url),
                    render_and_analyze(client, url, wait_for),
                );
                let (static_page, static_issues) = static_result?;
                let (rendered_page, rendered_issues) = rendered_result?;

                Ok(InspectResult {
                    url: url.to_string(),
                    mode: "compare".to_string(),
                    page: rendered_page,
                    issues: rendered_issues,
                    static_page: Some(static_page),
                    static_issues: Some(static_issues),
                })
            }
            #[cfg(not(feature = "cloudflare"))]
            {
                let _ = (cf_client, wait_for);
                anyhow::bail!("compare mode requires the 'cloudflare' feature")
            }
        }
    }
}

/// Fetch a URL with the local HTTP fetcher, parse, and analyze.
pub async fn fetch_and_analyze(url: &str) -> Result<(PageData, Vec<Issue>)> {
    let fetcher = Fetcher::new(
        concat!("Slither/", env!("CARGO_PKG_VERSION"), " (SEO inspect)"),
        15,
    );
    let (fetch_result, redirect_chain) = fetcher.fetch_with_redirects(url, 5).await?;

    // Parse against the URL that actually served the document, as the crawl path
    // does: relative links on a redirect target resolve against it (RFC 3986
    // §5.1.3), and reporting the page under the pre-redirect URL attributed the
    // destination's status to a URL that had really returned a 301.
    let final_url = fetch_result.url.clone();
    let mut page = parse_html(&fetch_result.body, &final_url);
    page.status = fetch_result.status;
    page.response_time_ms = fetch_result.response_time_ms;
    page.content_type = fetch_result.content_type.clone();
    page.redirect_chain = if redirect_chain.is_empty() {
        None
    } else {
        Some(redirect_chain)
    };

    compute_url_metadata(&mut page);
    page.security_headers = extract_security_headers(&fetch_result.headers);
    // Both of these were read on the crawl path and silently skipped here, so
    // the single-page audit — the command whose whole job is one page — could
    // not see a header-delivered noindex, and reported a header-canonicalised
    // page as missing its canonical.
    page.x_robots_tag = extract_header_value(&fetch_result.headers, "x-robots-tag");
    apply_link_header_canonical(&mut page, &fetch_result.headers);

    let issues = analyze_single_page(&page, &final_url);
    Ok((page, issues))
}

/// Render a URL via Cloudflare, parse the result, and analyze.
#[cfg(feature = "cloudflare")]
pub async fn render_and_analyze(
    client: &RenderClient,
    url: &str,
    wait_for: Option<&str>,
) -> Result<(PageData, Vec<Issue>)> {
    let html = render_page(client, url, wait_for, None).await?;
    let mut page = parse_html(&html, url);
    page.status = 200;
    // A rendered page is HTML by construction, but the render API returns no
    // Content-Type. Leaving the field empty makes `is_html_page` (and with it
    // the shared `is_indexable_html` rule used for the Indexable line) answer
    // false for every rendered page. The Playwright backend sets it for the same
    // reason.
    page.content_type = Some("text/html".to_string());

    compute_url_metadata(&mut page);

    let issues = analyze_single_page(&page, url);
    Ok((page, issues))
}

/// Run all analyzers on a single page and return only issues that reference the given URL.
pub fn analyze_single_page(page: &PageData, url: &str) -> Vec<Issue> {
    let registry = AnalyzerRegistry::default_registry();
    let ctx = AnalysisContext {
        seed_url: url.to_string(),
        domain: extract_domain(url).unwrap_or_default(),
        sitemap_data: None,
        pages: vec![page.clone()],
        robots_txt: None,
    };
    let all_issues = registry.run_all(&ctx);

    // Filter to issues that reference this URL (or have no URL filter).
    //
    // Matching on the string alone is not enough. A page reached through a
    // redirect is recorded at the URL that finally served it, while the redirect
    // finding is filed against the URL that redirected — so inspecting a
    // redirecting URL dropped the one finding that explains the redirect.
    // Resolving both sides through the alias map accepts either address.
    let aliases = crate::analysis::UrlAliases::build(&ctx.pages);
    let wanted = aliases.resolve(url);
    all_issues
        .into_iter()
        .filter(|issue| {
            issue.urls.is_empty() || issue.urls.iter().any(|u| aliases.resolve(&u.url) == wanted)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn extract_domain(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
}

fn truncate(s: &str, max: usize) -> String {
    // Count and slice by characters, not bytes: `&s[..n]` panics if byte `n`
    // lands mid-UTF-8-sequence (em-dashes, curly quotes, accents, emoji in a
    // meta description are common and were crashing `slither inspect`).
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let keep = max.saturating_sub(3);
        let truncated: String = s.chars().take(keep).collect();
        format!("{truncated}...")
    }
}

fn yes_no(val: bool) -> &'static str {
    if val {
        "yes"
    } else {
        "no"
    }
}

fn severity_icon(severity: crate::models::issue::Severity) -> &'static str {
    use crate::models::issue::Severity;
    match severity {
        Severity::Critical => "\u{2717}", // ✗
        Severity::Warning => "\u{26A0}",  // ⚠
        Severity::Info => "\u{2139}",     // ℹ
    }
}

fn present_check(present: bool) -> String {
    if present {
        "\u{2713}".to_string() // ✓
    } else {
        "\u{2717}".to_string() // ✗
    }
}

fn delta_check(static_val: bool, rendered_val: bool) -> String {
    if static_val == rendered_val {
        "-".to_string()
    } else if rendered_val {
        "+".to_string()
    } else {
        "-".to_string()
    }
}

fn delta_i64(static_val: i64, rendered_val: i64) -> String {
    let diff = rendered_val - static_val;
    if diff == 0 {
        "-".to_string()
    } else if diff > 0 {
        format!("+{}", diff)
    } else {
        format!("{}", diff)
    }
}

#[cfg(test)]
mod truncate_tests {
    use super::truncate;

    #[test]
    fn does_not_panic_on_multibyte() {
        // A 57th byte would land mid-character; this must not panic.
        let s = "Café über —".repeat(10);
        let out = truncate(&s, 60);
        assert!(out.ends_with("..."));
    }

    #[test]
    fn short_string_unchanged() {
        assert_eq!(truncate("hello", 60), "hello");
    }
}

#[cfg(test)]
mod redirect_filter_tests {
    use super::analyze_single_page;
    use crate::models::page::{PageData, RedirectHop};

    fn redirecting_page(requested: &str, served: &str) -> PageData {
        let mut p = crate::crawler::parser::parse_html(
            "<html><head><title>A page that a redirect led to</title></head>\
             <body><h1>Served</h1><p>Body copy.</p></body></html>",
            served,
        );
        p.status = 200;
        p.content_type = Some("text/html".to_string());
        p.redirect_chain = Some(vec![RedirectHop {
            status: 301,
            url: requested.to_string(),
        }]);
        p
    }

    /// Regression: a page reached through a redirect is recorded at the URL that
    /// finally served it, while the redirect finding is filed against the URL
    /// that redirected. Filtering on string equality with the served URL dropped
    /// the one finding that explains the redirect, so `slither inspect` on a
    /// redirecting URL reported everything about the destination and never
    /// mentioned that the URL asked for had moved.
    #[test]
    fn inspecting_a_redirecting_url_keeps_the_redirect_finding() {
        let page = redirecting_page("https://test.com/old", "https://test.com/new/");

        let by_requested = analyze_single_page(&page, "https://test.com/old");
        assert!(
            by_requested.iter().any(|i| i.check.contains("redirect")),
            "inspecting the URL that redirects dropped the redirect finding; got: {:?}",
            by_requested.iter().map(|i| &i.check).collect::<Vec<_>>()
        );

        // The destination address must still resolve to the same page.
        let by_served = analyze_single_page(&page, "https://test.com/new/");
        assert!(
            by_served.iter().any(|i| i.check.contains("redirect")),
            "inspecting the served URL dropped the redirect finding; got: {:?}",
            by_served.iter().map(|i| &i.check).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod indexability_tests {
    use super::format_inspect_output;
    use crate::models::page::PageData;

    fn page(url: &str, status: u16) -> PageData {
        let mut p = crate::crawler::parser::parse_html(
            "<html><head><title>T</title></head><body><h1>T</h1></body></html>",
            url,
        );
        p.status = status;
        p.content_type = Some("text/html".to_string());
        p
    }

    fn field(out: &str, label: &str) -> String {
        out.lines()
            .find(|l| l.trim_start().starts_with(label))
            .unwrap_or_else(|| panic!("no {label} line in:\n{out}"))
            .trim_start()
            .trim_start_matches(label)
            .trim()
            .to_string()
    }

    /// Regression: the Indexable line tested only the robots directives, so
    /// `slither inspect` printed "Indexable  yes" on the line directly above
    /// "Status  404". Indexability is the shared `analysis::is_indexable_html`
    /// rule: a 2xx HTML document that is not noindex.
    #[test]
    fn an_error_page_is_not_reported_indexable() {
        for status in [404u16, 500, 403, 301] {
            let out = format_inspect_output(
                "https://test.com/x",
                &page("https://test.com/x", status),
                &[],
            );
            assert_eq!(
                field(&out, "Indexable"),
                "no",
                "a {status} response must not be reported indexable:\n{out}"
            );
        }
    }

    /// An ordinary page still reads as indexable — the fix must not invert.
    #[test]
    fn an_ordinary_page_is_reported_indexable() {
        let out = format_inspect_output("https://test.com/", &page("https://test.com/", 200), &[]);
        assert_eq!(field(&out, "Indexable"), "yes");
    }

    /// A noindex 200 is still not indexable, and the directive list that
    /// explains why is still printed beside it.
    #[test]
    fn a_noindex_page_keeps_its_directive_list() {
        let mut p = page("https://test.com/", 200);
        p.x_robots_tag = Some("NOINDEX".to_string());
        let out = format_inspect_output("https://test.com/", &p, &[]);
        assert!(
            field(&out, "Indexable").starts_with("no"),
            "expected noindex:\n{out}"
        );
        assert!(out.contains("(noindex)"), "expected the directive:\n{out}");
    }

    /// A non-HTML 200 (a PDF served at a crawled URL) is not an indexable HTML
    /// document either.
    #[test]
    fn a_non_html_response_is_not_reported_indexable() {
        let mut p = page("https://test.com/doc.pdf", 200);
        p.content_type = Some("application/pdf".to_string());
        let out = format_inspect_output("https://test.com/doc.pdf", &p, &[]);
        assert_eq!(field(&out, "Indexable"), "no");
    }
}
