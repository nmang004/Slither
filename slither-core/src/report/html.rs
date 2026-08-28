use crate::crawler::detect_js_rendering;
use crate::models::crawl_result::CrawlResult;
use crate::report::svg;
use anyhow::{Context, Result};
use minijinja::{context, Environment};
use std::collections::HashMap;

const CSS: &str = include_str!("../../../templates/styles/report.css");
const TEMPLATE: &str = include_str!("../../../templates/report.html");
const HEADER_TEMPLATE: &str = include_str!("../../../templates/components/header.html");
const OVERVIEW_TEMPLATE: &str = include_str!("../../../templates/components/tab-overview.html");
const ISSUES_TEMPLATE: &str = include_str!("../../../templates/components/tab-issues.html");
const PAGES_TEMPLATE: &str = include_str!("../../../templates/components/tab-pages.html");
const STRUCTURE_TEMPLATE: &str = include_str!("../../../templates/components/tab-structure.html");
const MASCOT_SVG: &str = include_str!("../../../templates/mascot.svg");

/// Escape a string for safe interpolation into HTML text or double-quoted
/// attribute contexts. Used at the raw `format!`-based HTML sites that bypass
/// the template engine's auto-escaping.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn score_message(score: u32) -> &'static str {
    match score {
        90..=100 => "This site is in peak hunting form",
        80..=89 => "Looking strong — just a few scratches",
        70..=79 => "Some work needed to sharpen those claws",
        60..=69 => "This slither needs some TLC",
        _ => "Houston, we have a prehistoric problem",
    }
}

fn empty_message(domain: &str) -> &'static str {
    let messages: &[&str] = &[
        "No issues here — this slither's done hunting",
        "All clear! Nothing but tumbleweeds",
        "Clean as a fossil in a museum",
        "Zero issues? You're a natural predator",
    ];
    let hash: usize = domain.bytes().map(|b| b as usize).sum();
    messages[hash % messages.len()]
}

fn clean_message(category: &str) -> &'static str {
    let messages: &[&str] = &[
        "No issues found — looking sharp!",
        "This territory is well-defended",
        "All checks passed — impressive!",
        "Nothing to report — carry on!",
    ];
    let hash: usize = category.bytes().map(|b| b as usize).sum();
    messages[hash % messages.len()]
}

/// Render the crawl result as a self-contained HTML report.
pub fn render_html_report(result: &CrawlResult) -> Result<String> {
    let mut env = Environment::new();
    // Auto-escape all interpolated values by default. Crawled content (titles,
    // meta descriptions, URLs, etc.) is untrusted and must never be emitted raw.
    // Pre-rendered HTML blocks we build ourselves are explicitly marked `|safe`
    // in the templates.
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::Html);

    env.add_template("report", TEMPLATE)
        .context("Failed to load report template")?;
    env.add_template("header", HEADER_TEMPLATE)
        .context("Failed to load header template")?;
    env.add_template("tab-overview", OVERVIEW_TEMPLATE)
        .context("Failed to load overview template")?;
    env.add_template("tab-issues", ISSUES_TEMPLATE)
        .context("Failed to load issues template")?;
    env.add_template("tab-pages", PAGES_TEMPLATE)
        .context("Failed to load pages template")?;
    env.add_template("tab-structure", STRUCTURE_TEMPLATE)
        .context("Failed to load structure template")?;

    // Format duration and date
    let duration = format!("{:.1}s", result.crawl_metadata.duration_ms as f64 / 1000.0);
    // Take the YYYY-MM-DD prefix without panicking: a byte slice would abort
    // rendering on a short or multi-byte date (e.g. a hand-edited crawl.json).
    let crawl_date = result
        .crawl_metadata
        .crawl_date
        .get(..10)
        .unwrap_or(result.crawl_metadata.crawl_date.as_str());

    // Backend detection
    let backend = &result.crawl_metadata.backend;

    // JS rendering detection (for local backend warning)
    let (js_detection_warning, js_detection_count, js_detection_total) =
        if backend != "cloudflare" && backend != "playwright" {
            match detect_js_rendering(&result.pages) {
                Some((count, total)) => (true, count, total),
                None => (false, 0, 0),
            }
        } else {
            (false, 0, 0)
        };

    // Render header
    let header_html = env.get_template("header")?.render(context! {
        domain => result.crawl_metadata.domain,
        crawl_date => crawl_date,
        duration => duration,
        total_pages => result.summary.total_pages,
        mascot_svg => MASCOT_SVG,
        total_issues => result.summary.total_issues,
        backend => backend,
    })?;

    // Render tab content
    let tab_overview_html = render_overview_tab(
        &env,
        result,
        backend,
        js_detection_warning,
        js_detection_count,
        js_detection_total,
    )?;
    let tab_issues_html = render_issues_tab(&env, result)?;
    let tab_pages_html = render_pages_tab(&env, result)?;
    let tab_structure_html = render_structure_tab(&env, result)?;
    let tab_scripts = render_tab_scripts(result);

    // Render main template
    let rendered = env.get_template("report")?.render(context! {
        css => CSS,
        domain => result.crawl_metadata.domain,
        header_html => header_html,
        total_issues => result.summary.total_issues,
        tab_overview_html => tab_overview_html,
        tab_issues_html => tab_issues_html,
        tab_pages_html => tab_pages_html,
        tab_structure_html => tab_structure_html,
        tab_scripts => tab_scripts,
    })?;

    Ok(rendered)
}

fn render_overview_tab(
    env: &Environment,
    result: &CrawlResult,
    backend: &str,
    js_detection_warning: bool,
    js_detection_count: usize,
    js_detection_total: usize,
) -> Result<String> {
    // Compute the health score color from the grade
    let score_color = match result.summary.health_score {
        90..=100 => "#10b981",
        80..=89 => "#10b981",
        70..=79 => "#f59e0b",
        60..=69 => "#f97316",
        _ => "#ef4444",
    };

    // Generate SVG charts
    let score_gauge = svg::render_score_gauge(
        result.summary.health_score,
        &result.summary.grade,
        score_color,
    );

    // Compute deductions for score breakdown
    let grade = crate::analysis::scoring::compute_health_score(
        &result.issues.issues,
        result.summary.total_pages,
    );
    let deductions_ctx: Vec<minijinja::Value> = grade
        .deductions
        .iter()
        .map(|d| {
            context! {
                category => d.category.clone(),
                critical => d.critical_count,
                warning => d.warning_count,
                info => d.info_count,
                points => format!("{:.1}", d.points_deducted),
            }
        })
        .collect();
    let total_deducted: f64 = grade.deductions.iter().map(|d| d.points_deducted).sum();

    // Status distribution donut
    let total = result.summary.total_pages.max(1) as f64;
    let mut count_2xx: u32 = 0;
    let mut count_3xx: u32 = 0;
    let mut count_4xx: u32 = 0;
    let mut count_5xx: u32 = 0;
    for (status_str, &count) in &result.summary.by_status {
        if let Ok(status) = status_str.parse::<u16>() {
            match status {
                200..=299 => count_2xx += count,
                300..=399 => count_3xx += count,
                400..=499 => count_4xx += count,
                500..=599 => count_5xx += count,
                _ => {}
            }
        }
    }

    let mut donut_segments: Vec<(f64, &str, &str)> = Vec::new();
    if count_2xx > 0 {
        donut_segments.push((count_2xx as f64 / total * 100.0, "2xx", "#10b981"));
    }
    if count_3xx > 0 {
        donut_segments.push((count_3xx as f64 / total * 100.0, "3xx", "#f59e0b"));
    }
    if count_4xx > 0 {
        donut_segments.push((count_4xx as f64 / total * 100.0, "4xx", "#ef4444"));
    }
    if count_5xx > 0 {
        donut_segments.push((count_5xx as f64 / total * 100.0, "5xx", "#f97316"));
    }
    if donut_segments.is_empty() {
        // No status buckets parsed — an empty crawl, or one where every page
        // errored. Showing a full green "2xx" ring here would report a
        // fabricated clean bill of health for a crawl that returned nothing.
        donut_segments.push((100.0, "no data", "#94a3b8"));
    }
    let donut_chart = svg::render_donut_chart(&donut_segments, result.summary.total_pages);

    // Response time buckets
    let mut fast = 0u32;
    let mut moderate = 0u32;
    let mut slow = 0u32;
    let mut very_slow = 0u32;
    for page in &result.pages {
        match page.response_time_ms {
            0..=199 => fast += 1,
            200..=499 => moderate += 1,
            500..=999 => slow += 1,
            _ => very_slow += 1,
        }
    }
    let bar_buckets: Vec<(String, u32, &str)> = vec![
        ("0-200ms".to_string(), fast, "#10b981"),
        ("200-500ms".to_string(), moderate, "#f59e0b"),
        ("500-1000ms".to_string(), slow, "#f97316"),
        ("1000ms+".to_string(), very_slow, "#ef4444"),
    ];
    let bar_chart = svg::render_bar_chart(&bar_buckets);

    // Response time color
    let response_color = match result.summary.avg_response_time_ms {
        0..=299 => "green",
        300..=599 => "yellow",
        _ => "red",
    };

    // Top 3 issues from the new Issue model
    let mut top_issues: Vec<minijinja::Value> = Vec::new();
    {
        use crate::models::issue::Severity;

        let mut issue_entries: Vec<(&str, &str, &str, usize, &str)> = result
            .issues
            .issues
            .iter()
            .map(|issue| {
                let sev = match issue.severity {
                    Severity::Critical => "critical",
                    Severity::Warning => "warning",
                    Severity::Info => "info",
                };
                (
                    sev,
                    issue.display_name.as_str(),
                    issue.check.as_str(),
                    // A site-level finding carries no URL list. Counting it as
                    // zero excluded it from Top Issues entirely — the same way
                    // the Issues tab used to drop it — so a crawl whose only
                    // warning was "No Sitemap Found" showed no top issue at all
                    // while the severity bar beside it counted one. `max(1)` is
                    // the convention `scoring::count_urls_by_severity` already
                    // uses for exactly this case.
                    issue.urls.len().max(1),
                    issue.description.as_str(),
                )
            })
            .collect();

        issue_entries.sort_by(|a, b| {
            let sev_order = |s: &str| match s {
                "critical" => 0,
                "warning" => 1,
                _ => 2,
            };
            sev_order(a.0).cmp(&sev_order(b.0)).then(b.3.cmp(&a.3))
        });

        for (severity, display_name, _check, count, why) in issue_entries.into_iter().take(3) {
            top_issues.push(context! {
                severity => severity,
                display_name => display_name,
                count => count,
                why => why,
            });
        }
    }

    let mascot_mood = if result.summary.health_score >= 70 {
        "happy"
    } else {
        "concerned"
    };

    // CWV summary for overview
    let cwv_pages_tested = result.summary.cwv_pages_tested;
    let cwv_pct_good = if cwv_pages_tested > 0 {
        (result.summary.cwv_pages_good as f64 / cwv_pages_tested as f64 * 100.0) as u32
    } else {
        0
    };
    let avg_lcp = result
        .summary
        .avg_lcp_ms
        .map(|v| format!("{:.0}", v))
        .unwrap_or_else(|| "\u{2014}".to_string());
    let avg_cls = result
        .summary
        .avg_cls
        .map(|v| format!("{:.3}", v))
        .unwrap_or_else(|| "\u{2014}".to_string());

    let rendered = env.get_template("tab-overview")?.render(context! {
        score_gauge => score_gauge,
        score_color => score_color,
        grade_verdict => result.summary.grade_verdict,
        fun_verdict => score_message(result.summary.health_score),
        mascot_mood => mascot_mood,
        donut_chart => donut_chart,
        total_pages => result.summary.total_pages,
        pages_discovered => result.crawl_metadata.pages_discovered,
        pages_errored => result.crawl_metadata.pages_errored,
        avg_response_time => result.summary.avg_response_time_ms,
        response_color => response_color,
        p50 => result.summary.response_time_p50_ms,
        p95 => result.summary.response_time_p95_ms,
        total_issues => result.summary.total_issues,
        critical_issues => result.summary.critical_issues,
        warning_issues => result.summary.warning_issues,
        info_issues => result.summary.info_issues,
        avg_word_count => result.summary.avg_word_count,
        top_issues => top_issues,
        bar_chart => bar_chart,
        deductions => deductions_ctx,
        total_deducted => format!("{:.1}", total_deducted),
        has_deductions => !grade.deductions.is_empty(),
        health_score => result.summary.health_score,
        backend => backend,
        js_detection_warning => js_detection_warning,
        js_detection_count => js_detection_count,
        js_detection_total => js_detection_total,
        cwv_pages_tested => cwv_pages_tested,
        cwv_pct_good => cwv_pct_good,
        avg_lcp => avg_lcp,
        avg_cls => avg_cls,
    })?;

    Ok(rendered)
}

/// The report is a single self-contained file that has to open in a browser.
/// A 12,000-page crawl rendered every affected URL for every issue and every
/// page row, producing a 170 MB file no browser will load comfortably — the
/// same unbounded-list problem the MCP tools had. The full data is always in
/// the JSON and CSV exports alongside it.
const MAX_URLS_PER_ISSUE: usize = 100;
const MAX_REPORT_PAGES: usize = 2_000;

fn render_issues_tab(env: &Environment, result: &CrawlResult) -> Result<String> {
    use crate::models::issue::{IssueCategory, Severity};

    // Define category display order and names
    let category_order = [
        (IssueCategory::ResponseCodes, "Response Codes"),
        (IssueCategory::Security, "Security"),
        (IssueCategory::Url, "URL"),
        (IssueCategory::PageTitles, "Page Titles"),
        (IssueCategory::MetaDescription, "Meta Description"),
        (IssueCategory::Headings, "Headings"),
        (IssueCategory::Content, "Content"),
        (IssueCategory::Images, "Images"),
        (IssueCategory::Canonicals, "Canonicals"),
        (IssueCategory::Directives, "Directives"),
        (IssueCategory::Hreflang, "Hreflang"),
        (IssueCategory::Links, "Links"),
        (IssueCategory::StructuredData, "Structured Data"),
        (IssueCategory::Sitemaps, "Sitemaps"),
        (IssueCategory::Performance, "Performance"),
        (IssueCategory::JavaScript, "JavaScript"),
    ];

    let category_descriptions: &[&str] = &[
        "HTTP status codes, redirects, and server errors across crawled pages",
        "HTTPS usage, security headers, mixed content, and cross-origin safety",
        "URL structure, length, encoding, and SEO-friendly formatting",
        "Title tag presence, uniqueness, length, and pixel width for SERP display",
        "Meta description presence, uniqueness, and length for SERP snippets",
        "Heading hierarchy (H1-H6), uniqueness, and semantic structure",
        "Page content quality, duplicate detection, and thin content identification",
        "Image alt text, dimensions, and accessibility compliance",
        "Canonical tag presence, self-referencing, and consistency",
        "Robots directives (meta robots, X-Robots-Tag) and crawl control",
        "International targeting tags, language codes, and return link validation",
        "Internal/external link health, orphan pages, and crawl coverage",
        "Schema.org markup, JSON-LD validation, and required field checks",
        "Sitemap presence, URL coverage, and consistency with crawled pages",
        "Core Web Vitals and performance metrics from PageSpeed Insights",
        "JavaScript-dependent SEO tags, render-blocking scripts, third-party weight, and console errors",
    ];

    let mut categories: Vec<minijinja::Value> = Vec::new();

    for (idx, (cat, display_name)) in category_order.iter().enumerate() {
        // Site-level findings (`no_sitemap_found`, `ai_crawlers_blocked`, ...)
        // legitimately carry no URL list — they are about the site, not a page.
        // Dropping them here made the HTML the only artifact that hid them: the
        // JSON, CLI and MCP all reported the Sitemaps warning while the report
        // rendered "0 issues found", a green dot and "All checks passed" for the
        // same crawl, with the severity bar above still counting the issue.
        let mut cat_issues: Vec<_> = result
            .issues
            .issues
            .iter()
            .filter(|i| &i.category == cat)
            .collect();

        cat_issues.sort_by(|a, b| {
            a.severity
                .cmp(&b.severity)
                .then(b.urls.len().cmp(&a.urls.len()))
        });

        // Distinct affected URLs, the same de-duplication
        // `analysis::scoring::build_category_summaries` performs for the JSON.
        // Summing `urls.len()` counted issue rows, not URLs: a check that emits
        // one row per (page, defect) pair reported "Images - 2 issues found ·
        // 20 URLs affected" on a 10-page crawl, a number that cannot be true and
        // that contradicted the JSON/CLI/MCP figure of 10 in the same run.
        let issue_count: usize = cat_issues
            .iter()
            .flat_map(|i| i.urls.iter().map(|u| u.url.as_str()))
            .collect::<std::collections::HashSet<&str>>()
            .len();
        let has_critical = cat_issues.iter().any(|i| i.severity == Severity::Critical);
        let has_warning = cat_issues.iter().any(|i| i.severity == Severity::Warning);

        let status = if has_critical {
            "critical"
        } else if has_warning {
            "warning"
        } else if !cat_issues.is_empty() {
            "info"
        } else {
            "clean"
        };

        let checks: Vec<minijinja::Value> = cat_issues
            .iter()
            .map(|issue| {
                let severity = match issue.severity {
                    Severity::Critical => "critical",
                    Severity::Warning => "warning",
                    Severity::Info => "info",
                };

                let domain = &result.crawl_metadata.domain;
                let items: Vec<minijinja::Value> = issue
                    .urls
                    .iter()
                    .take(MAX_URLS_PER_ISSUE)
                    .map(|u| {
                        let path = u
                            .url
                            .replace(&format!("https://{}", domain), "")
                            .replace(&format!("http://{}", domain), "");
                        let path = if path.is_empty() {
                            "/".to_string()
                        } else {
                            path
                        };

                        context! {
                            path => path,
                            full_url => u.url.clone(),
                            detail => u.detail.clone(),
                        }
                    })
                    .collect();

                // A site-level finding has no URL list to count; rendering a
                // bare "0" next to "No Sitemap Found" reads as a broken report,
                // so name what it applies to instead.
                let count = if issue.urls.is_empty() {
                    minijinja::Value::from("Site-wide")
                } else {
                    minijinja::Value::from(issue.urls.len())
                };

                context! {
                    severity => severity,
                    display_name => issue.display_name,
                    count => count,
                    why => issue.description,
                    fix => issue.guidance,
                    items => items,
                    truncated => issue.urls.len() > items.len(),
                    shown => items.len(),
                }
            })
            .collect();

        let cat_id = format!("{}", cat).replace(' ', "_").to_lowercase();
        let cat_name = *display_name;

        categories.push(context! {
            id => cat_id,
            name => cat_name,
            description => category_descriptions[idx],
            status => status,
            issue_count => issue_count,
            checks_count => cat_issues.len(),
            checks => checks,
            clean_msg => clean_message(cat_name),
        });
    }

    let rendered = env.get_template("tab-issues")?.render(context! {
        critical_issues => result.summary.critical_issues,
        warning_issues => result.summary.warning_issues,
        info_issues => result.summary.info_issues,
        categories => categories,
        empty_msg => empty_message(&result.crawl_metadata.domain),
    })?;

    Ok(rendered)
}

fn render_pages_tab(env: &Environment, result: &CrawlResult) -> Result<String> {
    use crate::models::issue::Severity;

    // Redirect findings are filed against the URL that redirects, which has no
    // page row of its own — the page is recorded at the URL that finally served
    // it. Resolving each issue URL through the crawl's alias map keeps those
    // findings attached to a real row; without it the Pages tab silently shows
    // zero redirect badges and the Issues tab links to a row that isn't there.
    let aliases = crate::analysis::UrlAliases::build(&result.pages);
    let page_key: HashMap<String, String> = result
        .pages
        .iter()
        .map(|p| (aliases.resolve(&p.url), p.url.clone()))
        .collect();
    let issue_page_key = |raw: &str| -> String {
        page_key
            .get(&aliases.resolve(raw))
            .cloned()
            .unwrap_or_else(|| raw.to_string())
    };

    // Build per-page issue counts and detail lists
    let mut page_issues: HashMap<String, (u32, u32, u32)> = HashMap::new();
    let mut page_issue_details: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for issue in &result.issues.issues {
        let sev_str = match issue.severity {
            Severity::Critical => "critical",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        for u in &issue.urls {
            let key = issue_page_key(&u.url);
            let entry = page_issues.entry(key.clone()).or_insert((0, 0, 0));
            match issue.severity {
                Severity::Critical => entry.0 += 1,
                Severity::Warning => entry.1 += 1,
                Severity::Info => entry.2 += 1,
            }
            page_issue_details
                .entry(key)
                .or_default()
                .push((issue.display_name.clone(), sev_str.to_string()));
        }
    }

    // Worst pages first, so a truncated table is still the useful half.
    let mut ordered: Vec<&crate::models::page::PageData> = result.pages.iter().collect();
    ordered.sort_by_key(|p| {
        let (c, w, i) = page_issues.get(&p.url).copied().unwrap_or((0, 0, 0));
        std::cmp::Reverse((c, w, i))
    });
    let pages_truncated = ordered.len() > MAX_REPORT_PAGES;
    let pages: Vec<minijinja::Value> = ordered
        .into_iter()
        .take(MAX_REPORT_PAGES)
        .map(|p| {
            let status_class = match p.status {
                200..=299 => "2xx",
                300..=399 => "3xx",
                400..=499 => "4xx",
                _ => "5xx",
            };
            let link_count = p.internal_links.len() + p.external_links.len();
            let og_display = if p.og_tags.is_empty() {
                "—".to_string()
            } else {
                p.og_tags
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let redirect_display = format_redirect_chain(&p.redirect_chain, &p.url);
            context! {
                url => p.url,
                status => p.status,
                status_class => status_class,
                title => p.title.as_deref().unwrap_or("—"),
                meta_description => p.meta_description.as_deref().unwrap_or("—"),
                h1 => if p.h1.is_empty() { "—".to_string() } else { p.h1.join(", ") },
                canonical => p.canonical.as_deref().unwrap_or("—"),
                schema_types => if p.schema_types.is_empty() { "—".to_string() } else { p.schema_types.join(", ") },
                og_tags_display => og_display,
                word_count => p.word_count,
                internal_link_count => p.internal_links.len(),
                external_link_count => p.external_links.len(),
                image_count => p.images.len(),
                // Same rule as the issue check, the crawl summary and the CSV:
                // `data:` URI placeholders and 1x1 pixels are decorative. Using
                // a raw `alt.is_none()` here made the Pages tab report 311 for a
                // page the Issues tab in the same file reported as 3.
                images_no_alt => p
                    .images
                    .iter()
                    .filter(|i| i.alt.is_none() && i.needs_alt_text())
                    .count(),
                link_count => link_count,
                response_time_ms => p.response_time_ms,
                depth => p.depth,
                content_type => p.content_type.as_deref().unwrap_or("—"),
                has_redirect => p.redirect_chain.is_some(),
                redirect_chain_display => redirect_display,
                // Issue indicators
                issue_critical => page_issues.get(&p.url).map(|c| c.0).unwrap_or(0),
                issue_warning => page_issues.get(&p.url).map(|c| c.1).unwrap_or(0),
                issue_info => page_issues.get(&p.url).map(|c| c.2).unwrap_or(0),
                has_issues => page_issues.get(&p.url).map(|c| c.0 + c.1 + c.2 > 0).unwrap_or(false),
                issue_badges => {
                    let details = page_issue_details.get(&p.url).cloned().unwrap_or_default();
                    let badges: Vec<minijinja::Value> = details.iter().map(|(name, sev)| {
                        context! { display_name => name.clone(), severity => sev.clone() }
                    }).collect();
                    badges
                },
                // Richer detail fields
                // `analysis::is_indexable_html` is the one place this rule
                // lives: 2xx HTML that is not noindex. Testing only
                // `is_noindex()` made every 404, 500 and 403 render a green
                // "Indexable" badge — the report said 8 pages were indexable
                // while `slither sitemap`, which uses the shared rule, emitted 5.
                is_indexable => crate::analysis::is_indexable_html(p),
                title_length => p.title_length.unwrap_or(0),
                title_pixel_width => p.title_pixel_width.unwrap_or(0),
                meta_desc_length => p.meta_description_length.unwrap_or(0),
                meta_desc_pixel_width => p.meta_description_pixel_width.unwrap_or(0),
                readability_score => p.readability_score.map(|s| format!("{:.0}", s)).unwrap_or_else(|| "\u{2014}".to_string()),
                lcp_display => p.lcp_ms.map(|v| format!("{:.0}ms", v)).unwrap_or_else(|| "\u{2014}".to_string()),
                cls_display => p.cls.map(|v| format!("{:.3}", v)).unwrap_or_else(|| "\u{2014}".to_string()),
                perf_display => p.performance_score.map(|v| v.to_string()).unwrap_or_else(|| "\u{2014}".to_string()),
                has_cwv => p.performance_score.is_some(),
                lcp_detail => p.lcp_ms.map(|v| format!("{:.0}ms", v)).unwrap_or_else(|| "\u{2014}".to_string()),
                inp_detail => p.inp_ms.map(|v| format!("{:.0}ms", v)).unwrap_or_else(|| "\u{2014}".to_string()),
                cls_detail => p.cls.map(|v| format!("{:.3}", v)).unwrap_or_else(|| "\u{2014}".to_string()),
                fcp_detail => p.fcp_ms.map(|v| format!("{:.0}ms", v)).unwrap_or_else(|| "\u{2014}".to_string()),
                ttfb_detail => p.ttfb_ms.map(|v| format!("{:.0}ms", v)).unwrap_or_else(|| "\u{2014}".to_string()),
                perf_score_detail => p.performance_score.map(|v| v.to_string()).unwrap_or_else(|| "\u{2014}".to_string()),
                is_https => p.is_https,
                has_hsts => p.security_headers.has_hsts,
                has_csp => p.security_headers.has_csp,
            }
        })
        .collect();

    let rendered = env.get_template("tab-pages")?.render(context! {
        pages => pages,
        pages_truncated => pages_truncated,
        pages_shown => pages.len(),
        total_pages => result.summary.total_pages,
    })?;

    Ok(rendered)
}

/// Render `a --301--> b --302--> c`, where `final_url` is `c`.
///
/// Each [`RedirectHop`](crate::models::page::RedirectHop) records the URL that
/// *served* the redirect and the status it answered with, so the destination of
/// the last hop is not in the chain at all — it is the page's own URL, which
/// since the page-identity change is the final URL after redirects. Emitting
/// only the hops left every chain in the report ending in a dangling arrow
/// pointing at nothing.
fn format_redirect_chain(
    chain: &Option<Vec<crate::models::page::RedirectHop>>,
    final_url: &str,
) -> String {
    match chain {
        None => String::new(),
        Some(hops) if hops.is_empty() => String::new(),
        Some(hops) => {
            let mut out = String::new();
            for hop in hops {
                out.push_str(&format!(
                    "<span class=\"chain-url\">{}</span><span class=\"chain-arrow\"> --{}--> </span>",
                    html_escape(&hop.url),
                    hop.status
                ));
            }
            out.push_str(&format!(
                "<span class=\"chain-url\">{}</span>",
                html_escape(final_url)
            ));
            out
        }
    }
}

fn render_structure_tab(env: &Environment, result: &CrawlResult) -> Result<String> {
    use crate::models::issue::Severity;

    // 1. Depth distribution
    let mut depth_counts: HashMap<u32, u32> = HashMap::new();
    let mut max_depth: u32 = 0;
    for page in &result.pages {
        *depth_counts.entry(page.depth).or_insert(0) += 1;
        if page.depth > max_depth {
            max_depth = page.depth;
        }
    }

    let max_count = depth_counts.values().copied().max().unwrap_or(1);
    let mut depth_bars: Vec<minijinja::Value> = Vec::new();
    for d in 0..=max_depth {
        let count = depth_counts.get(&d).copied().unwrap_or(0);
        let pct = if max_count > 0 {
            (count as f64 / max_count as f64 * 100.0) as u32
        } else {
            0
        };
        depth_bars.push(context! {
            depth => d,
            count => count,
            pct => pct,
        });
    }

    // 2. Build per-page issue map. The detail lists are accumulated in this
    // same pass: building them later by rescanning every issue for every page
    // is O(pages x issue-URL entries), which on a large crawl (10k pages,
    // ~100k entries) turns report generation into a multi-minute stall.
    // Redirect findings are filed against the URL that redirects, which has no
    // page row of its own — the page is recorded at the URL that finally served
    // it. Resolving each issue URL through the crawl's alias map keeps those
    // findings attached to a real row; without it the Pages tab silently shows
    // zero redirect badges and the Issues tab links to a row that isn't there.
    let aliases = crate::analysis::UrlAliases::build(&result.pages);
    let page_key: HashMap<String, String> = result
        .pages
        .iter()
        .map(|p| (aliases.resolve(&p.url), p.url.clone()))
        .collect();
    let issue_page_key = |raw: &str| -> String {
        page_key
            .get(&aliases.resolve(raw))
            .cloned()
            .unwrap_or_else(|| raw.to_string())
    };

    let mut page_issues: HashMap<String, (u32, u32, u32)> = HashMap::new();
    let mut page_issue_list: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for issue in &result.issues.issues {
        let sev = match issue.severity {
            Severity::Critical => "critical",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        for u in &issue.urls {
            let key = issue_page_key(&u.url);
            let entry = page_issues.entry(key.clone()).or_insert((0, 0, 0));
            match issue.severity {
                Severity::Critical => entry.0 += 1,
                Severity::Warning => entry.1 += 1,
                Severity::Info => entry.2 += 1,
            }
            page_issue_list
                .entry(key)
                .or_default()
                .push(serde_json::json!({ "name": issue.display_name, "severity": sev }));
        }
    }

    // 3. Build directory tree structure
    #[derive(Default)]
    struct DirNode {
        children: HashMap<String, DirNode>,
        pages: Vec<usize>, // indices into result.pages
    }

    // The tree is walked by several recursive helpers below, so its depth is
    // bounded here at construction: a hostile site serving `/a/a/a/…` with
    // thousands of segments would otherwise nest that deeply and overflow the
    // stack while rendering. Pages deeper than this are grouped under the
    // deepest directory shown.
    const MAX_TREE_DEPTH: usize = 24;

    let mut dir_root = DirNode::default();
    for (idx, page) in result.pages.iter().enumerate() {
        let path = url::Url::parse(&page.url)
            .map(|u| u.path().to_string())
            .unwrap_or_else(|_| "/".to_string());

        let segments: Vec<&str> = path
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        if segments.is_empty() || segments.len() == 1 {
            dir_root.pages.push(idx);
        } else {
            // Navigate to parent directory, last segment is the page
            let dir_segments = &segments[..segments.len() - 1];
            let dir_segments = &dir_segments[..dir_segments.len().min(MAX_TREE_DEPTH)];
            let mut node = &mut dir_root;
            for seg in dir_segments {
                node = node.children.entry(seg.to_string()).or_default();
            }
            node.pages.push(idx);
        }
    }

    // 4. Render directory nodes as HTML
    fn count_all_pages(node: &DirNode) -> usize {
        let mut total = node.pages.len();
        for child in node.children.values() {
            total += count_all_pages(child);
        }
        total
    }

    fn collect_all_page_indices(node: &DirNode) -> Vec<usize> {
        let mut indices = node.pages.clone();
        for child in node.children.values() {
            indices.extend(collect_all_page_indices(child));
        }
        indices
    }

    fn render_dir_html(
        name: &str,
        path: &str,
        node: &DirNode,
        pages: &[crate::models::page::PageData],
        page_issues: &HashMap<String, (u32, u32, u32)>,
        is_root: bool,
    ) -> String {
        let mut html = String::new();
        let total_pages = count_all_pages(node);
        let all_indices = collect_all_page_indices(node);

        // Aggregate stats for directory
        let mut total_crit: u32 = 0;
        let mut total_warn: u32 = 0;
        let mut total_info: u32 = 0;
        let mut total_time: u64 = 0;
        let mut time_count: u32 = 0;

        for &idx in &all_indices {
            let p = &pages[idx];
            let (c, w, i) = page_issues.get(&p.url).copied().unwrap_or((0, 0, 0));
            total_crit += c;
            total_warn += w;
            total_info += i;
            total_time += p.response_time_ms;
            time_count += 1;
        }
        let avg_time = if time_count > 0 {
            total_time / time_count as u64
        } else {
            0
        };

        let display_name = if is_root { "/" } else { name };
        let dir_path = if is_root {
            "/".to_string()
        } else {
            path.to_string()
        };

        // Build issue dots HTML
        let mut dots = String::new();
        if total_crit > 0 {
            dots.push_str(&format!(
                "<span class=\"issue-dot dot-critical\">{}</span>",
                total_crit
            ));
        }
        if total_warn > 0 {
            dots.push_str(&format!(
                "<span class=\"issue-dot dot-warning\">{}</span>",
                total_warn
            ));
        }
        if total_info > 0 {
            dots.push_str(&format!(
                "<span class=\"issue-dot dot-info\">{}</span>",
                total_info
            ));
        }

        let has_children = !node.children.is_empty() || !node.pages.is_empty();

        if has_children {
            html.push_str(&format!(
                "<div class=\"dir-node\" data-path=\"{}\"><div class=\"dir-header\" onclick=\"toggleDir(this); selectDir(this)\"><span class=\"dir-toggle\">\u{25BC}</span><span class=\"dir-icon\"><svg width=\"14\" height=\"14\" viewBox=\"0 0 16 16\" fill=\"none\"><path d=\"M2 4.5A1.5 1.5 0 013.5 3h3.293a1 1 0 01.707.293L8.707 4.5H12.5A1.5 1.5 0 0114 6v6.5a1.5 1.5 0 01-1.5 1.5h-9A1.5 1.5 0 012 12.5v-8z\" stroke=\"currentColor\" stroke-width=\"1.2\" fill=\"none\"/></svg></span><span class=\"dir-name\">{}</span><span class=\"dir-stats\"><span class=\"dir-count\">{} page{}</span>{}<span class=\"dir-time\">{}ms avg</span></span></div><div class=\"dir-children\">",
                html_escape(&dir_path),
                html_escape(display_name),
                total_pages,
                if total_pages != 1 { "s" } else { "" },
                dots,
                avg_time,
            ));

            // Render child directories first (sorted)
            let mut dir_keys: Vec<&String> = node.children.keys().collect();
            dir_keys.sort();
            let base_path = if path.ends_with('/') {
                path.to_string()
            } else {
                format!("{}/", path)
            };
            for key in dir_keys {
                let child_path = format!("{}{}/", base_path, key);
                html.push_str(&render_dir_html(
                    key,
                    &child_path,
                    &node.children[key],
                    pages,
                    page_issues,
                    false,
                ));
            }

            // Render leaf pages
            // The tree exists to show the site's shape, not to list every
            // page — that is what the Pages tab and the CSV are for. Without a
            // per-directory cap a 12,000-page crawl rendered a 20 MB tree.
            const MAX_PAGES_PER_DIR: usize = 50;
            let shown = node.pages.len().min(MAX_PAGES_PER_DIR);
            if node.pages.len() > shown {
                html.push_str(&format!(
                    r#"<div class="tree-note">Showing {} of {} pages here — see the Pages tab or the CSV for the full list.</div>"#,
                    shown,
                    node.pages.len()
                ));
            }
            for &idx in node.pages.iter().take(MAX_PAGES_PER_DIR) {
                let p = &pages[idx];
                let page_path = url::Url::parse(&p.url)
                    .map(|u| u.path().to_string())
                    .unwrap_or_else(|_| p.url.clone());
                let leaf_name = page_path.split('/').rfind(|s| !s.is_empty()).unwrap_or("/");
                let (c, w, i) = page_issues.get(&p.url).copied().unwrap_or((0, 0, 0));

                let status_class = match p.status {
                    200..=299 => "node-ok",
                    300..=399 => "node-redirect",
                    _ => "node-error",
                };

                let mut page_dots = String::new();
                if c > 0 {
                    page_dots.push_str(&format!(
                        "<span class=\"issue-dot dot-critical\">{}</span>",
                        c
                    ));
                }
                if w > 0 {
                    page_dots.push_str(&format!(
                        "<span class=\"issue-dot dot-warning\">{}</span>",
                        w
                    ));
                }
                if i > 0 {
                    page_dots.push_str(&format!("<span class=\"issue-dot dot-info\">{}</span>", i));
                }

                html.push_str(&format!(
                    "<div class=\"page-node {}\" data-url=\"{}\" data-depth=\"{}\" onclick=\"selectPage(this)\"><span class=\"page-name\">{}</span><span class=\"page-stats\"><span class=\"page-words\">{} words</span>{}<span class=\"page-time\">{}ms</span></span></div>",
                    status_class,
                    html_escape(&p.url),
                    p.depth,
                    html_escape(if leaf_name == "/" || leaf_name.is_empty() {
                        "/"
                    } else {
                        leaf_name
                    }),
                    p.word_count,
                    page_dots,
                    p.response_time_ms,
                ));
            }

            html.push_str("</div></div>");
        }

        html
    }

    let tree_html = render_dir_html("/", "/", &dir_root, &result.pages, &page_issues, true);

    // 5. Count directories
    fn count_dirs(node: &DirNode) -> usize {
        let mut count = node.children.len();
        for child in node.children.values() {
            count += count_dirs(child);
        }
        count
    }
    let dir_count = count_dirs(&dir_root);

    // 6. Build page data JSON for the detail panel. Built with serde_json so
    // untrusted crawled strings are correctly escaped, then escaped again for
    // the `<script>` context (`</`, U+2028/2029) so a title containing
    // `</script>` cannot break out of the element.
    // Bounded to the same worst-first set the Pages tab shows. Embedding a
    // detail record for every page put 15 MB of JSON in a 12,000-page report,
    // most of it for rows the bounded tree never renders. The click handler
    // already no-ops on a missing entry.
    let mut detail_pages: Vec<&crate::models::page::PageData> = result.pages.iter().collect();
    detail_pages.sort_by_key(|p| {
        let (c, w, i) = page_issues.get(&p.url).copied().unwrap_or((0, 0, 0));
        std::cmp::Reverse((c, w, i))
    });
    detail_pages.truncate(MAX_REPORT_PAGES);

    let mut page_data_map = serde_json::Map::new();
    for page in detail_pages {
        let (c, w, i) = page_issues.get(&page.url).copied().unwrap_or((0, 0, 0));
        // Same rule as the Pages tab — see the note there.
        let indexable = crate::analysis::is_indexable_html(page);

        let issues: Vec<serde_json::Value> = page_issue_list
            .get(page.url.as_str())
            .cloned()
            .unwrap_or_default();

        let h1 = if page.h1.is_empty() {
            "\u{2014}".to_string()
        } else {
            page.h1.join(", ")
        };

        page_data_map.insert(
            page.url.clone(),
            serde_json::json!({
                "url": page.url,
                "title": page.title.as_deref().unwrap_or("\u{2014}"),
                "status": page.status,
                "words": page.word_count,
                "depth": page.depth,
                "time": page.response_time_ms,
                "critical": c,
                "warning": w,
                "info": i,
                "indexable": indexable,
                "https": page.is_https,
                "hsts": page.security_headers.has_hsts,
                "csp": page.security_headers.has_csp,
                "h1": h1,
                "canonical": page.canonical.as_deref().unwrap_or("\u{2014}"),
                "metaDesc": page.meta_description.as_deref().unwrap_or("\u{2014}"),
                "titleLen": page.title_length.unwrap_or(0),
                "titlePx": page.title_pixel_width.unwrap_or(0),
                "metaDescLen": page.meta_description_length.unwrap_or(0),
                "metaDescPx": page.meta_description_pixel_width.unwrap_or(0),
                "readability": page
                    .readability_score
                    .map(|s| format!("{:.0}", s))
                    .unwrap_or_else(|| "\u{2014}".to_string()),
                "issues": issues,
            }),
        );
    }
    let page_data_json = serde_json::to_string(&serde_json::Value::Object(page_data_map))
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</", "<\\/")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");

    let rendered = env.get_template("tab-structure")?.render(context! {
        total_pages => result.summary.total_pages,
        dir_count => dir_count,
        max_depth => max_depth,
        depth_bars => depth_bars,
        tree_html => tree_html,
        page_data_json => page_data_json,
    })?;

    Ok(rendered)
}

fn render_tab_scripts(_result: &CrawlResult) -> String {
    r#"
function toggleIssue(header) {
    header.classList.toggle('open');
    var body = header.nextElementSibling;
    body.classList.toggle('open');
}

// Page detail expansion
function toggleDetail(row) {
    var idx = row.getAttribute('data-idx');
    var detail = document.getElementById('detail-' + idx);
    if (detail) {
        detail.classList.toggle('open');
    }
}

// Filtering
function filterPages() {
    updateFilteredRows();
    currentPage = 0;
    applyPagination();
}

// Sorting
var sortDir = {};
function sortTable(col) {
    var tbody = document.querySelector('#pages-table tbody');
    var rowPairs = [];
    var trs = tbody.querySelectorAll('tr');
    for (var i = 0; i < trs.length; i += 2) {
        rowPairs.push([trs[i], trs[i+1]]);
    }
    var dir = sortDir[col] = !sortDir[col];
    rowPairs.sort(function(a, b) {
        var va = a[0].cells[col].textContent.trim();
        var vb = b[0].cells[col].textContent.trim();
        var na = parseFloat(va.replace(/[^0-9.\-]/g, ''));
        var nb = parseFloat(vb.replace(/[^0-9.\-]/g, ''));
        if (!isNaN(na) && !isNaN(nb)) {
            return dir ? na - nb : nb - na;
        }
        return dir ? va.localeCompare(vb) : vb.localeCompare(va);
    });
    rowPairs.forEach(function(pair) {
        tbody.appendChild(pair[0]);
        tbody.appendChild(pair[1]);
    });
    updateFilteredRows();
    applyPagination();
}

// Pagination
var currentPage = 0;
var pageSize = 25;
var filteredRows = [];

function updateFilteredRows() {
    var query = document.getElementById('page-filter').value.toLowerCase();
    var statusFilter = document.getElementById('status-filter').value;
    var rows = document.querySelectorAll('#pages-table tbody tr.expandable');
    filteredRows = [];
    rows.forEach(function(row) {
        var text = row.textContent.toLowerCase();
        var statusCell = row.cells[1].textContent.trim();
        var matchText = !query || text.includes(query);
        var matchStatus = !statusFilter || statusCell.charAt(0) === statusFilter;
        if (matchText && matchStatus) {
            filteredRows.push(row);
        }
    });
}

function applyPagination() {
    var allRows = document.querySelectorAll('#pages-table tbody tr.expandable');
    // Hide all rows and their details first
    allRows.forEach(function(row) {
        row.style.display = 'none';
        var idx = row.getAttribute('data-idx');
        var detail = document.getElementById('detail-' + idx);
        if (detail) detail.classList.remove('open');
    });
    // Show only rows in current page of filtered results
    var totalPages = Math.max(1, Math.ceil(filteredRows.length / pageSize));
    if (currentPage >= totalPages) currentPage = totalPages - 1;
    if (currentPage < 0) currentPage = 0;
    var start = currentPage * pageSize;
    var end = Math.min(start + pageSize, filteredRows.length);
    for (var i = start; i < end; i++) {
        filteredRows[i].style.display = '';
    }
    document.getElementById('page-info').textContent =
        'Page ' + (currentPage + 1) + ' of ' + totalPages + ' (' + filteredRows.length + ' rows)';
    document.getElementById('prev-btn').disabled = currentPage === 0;
    document.getElementById('next-btn').disabled = currentPage >= totalPages - 1;
}
function goToPage(p) { currentPage = p; applyPagination(); }
function changePageSize(s) { pageSize = parseInt(s); currentPage = 0; applyPagination(); }

// Structure tab: directory toggle
function toggleDir(header) {
    var node = header.parentElement;
    var children = node.querySelector('.dir-children');
    var toggle = header.querySelector('.dir-toggle');
    if (children.style.display === 'none') {
        children.style.display = '';
        toggle.textContent = '\u25BC';
    } else {
        children.style.display = 'none';
        toggle.textContent = '\u25B6';
    }
}

function expandAllDirs() {
    document.querySelectorAll('.dir-children').forEach(function(el) { el.style.display = ''; });
    document.querySelectorAll('.dir-toggle').forEach(function(el) { el.textContent = '\u25BC'; });
}
function collapseAllDirs() {
    document.querySelectorAll('.dir-children').forEach(function(el) {
        if (el.parentElement.parentElement.classList.contains('dir-node')) { return; }
        el.style.display = 'none';
    });
    document.querySelectorAll('.dir-toggle').forEach(function(el) { el.textContent = '\u25B6'; });
}

function searchStructure(query) {
    query = query.toLowerCase();
    document.querySelectorAll('.page-node').forEach(function(el) {
        var url = (el.getAttribute('data-url') || '').toLowerCase();
        var name = el.querySelector('.page-name').textContent.toLowerCase();
        el.style.display = (!query || url.includes(query) || name.includes(query)) ? '' : 'none';
    });
}

function filterByDepth(depth) {
    var activeBar = document.querySelector('.depth-bar-row.active');
    if (activeBar) activeBar.classList.remove('active');
    if (depth === null) {
        document.querySelectorAll('.page-node').forEach(function(el) { el.style.display = ''; });
        return;
    }
    var bar = document.querySelector('.depth-bar-row[data-depth="' + depth + '"]');
    if (bar) bar.classList.add('active');
    document.querySelectorAll('.page-node').forEach(function(el) {
        el.style.display = el.getAttribute('data-depth') == depth ? '' : 'none';
    });
}

// Structure detail panel
function selectPage(el) {
    document.querySelectorAll('.page-node.selected, .dir-header.selected').forEach(function(s) { s.classList.remove('selected'); });
    el.classList.add('selected');
    var url = el.getAttribute('data-url');
    var data = window.__structurePageData[url];
    if (!data) return;
    renderPageDetail(data);
}

function selectDir(header) {
    document.querySelectorAll('.page-node.selected, .dir-header.selected').forEach(function(s) { s.classList.remove('selected'); });
    header.classList.add('selected');
    var path = header.parentElement.getAttribute('data-path');
    renderDirDetail(path, header);
}

function renderPageDetail(data) {
    var panel = document.getElementById('structure-detail');
    while (panel.firstChild) panel.removeChild(panel.firstChild);

    var title = document.createElement('h3');
    title.className = 'detail-title';
    title.textContent = data.title || data.url;
    panel.appendChild(title);

    var url = document.createElement('div');
    url.className = 'detail-url';
    url.textContent = data.url;
    panel.appendChild(url);

    var grid = document.createElement('div');
    grid.className = 'detail-grid';

    function addField(label, value, className) {
        var d = document.createElement('div');
        var lbl = document.createElement('span');
        lbl.className = 'label';
        lbl.textContent = label;
        d.appendChild(lbl);
        d.appendChild(document.createElement('br'));
        var val = document.createElement('span');
        if (className) val.className = className;
        val.textContent = value;
        d.appendChild(val);
        grid.appendChild(d);
    }

    addField('Status', data.status);
    addField('Words', data.words);
    addField('Depth', data.depth);
    addField('Response Time', data.time + 'ms');
    addField('Indexability', data.indexable ? 'Indexable' : 'Noindex', data.indexable ? 'idx-yes' : 'idx-no');
    addField('Security', 'HTTPS ' + (data.https ? '\u2713' : '\u2717') + ' \u00b7 HSTS ' + (data.hsts ? '\u2713' : '\u2717') + ' \u00b7 CSP ' + (data.csp ? '\u2713' : '\u2717'));
    addField('H1', data.h1);
    addField('Canonical', data.canonical);
    if (data.titleLen > 0) addField('Title Length', data.titleLen + ' chars \u00b7 ' + data.titlePx + 'px');
    if (data.metaDescLen > 0) addField('Meta Desc Length', data.metaDescLen + ' chars \u00b7 ' + data.metaDescPx + 'px');
    addField('Readability', data.readability);

    panel.appendChild(grid);

    if (data.issues && data.issues.length > 0) {
        var issueDiv = document.createElement('div');
        issueDiv.className = 'detail-issues';
        var issueLabel = document.createElement('span');
        issueLabel.className = 'label';
        issueLabel.textContent = 'Issues';
        issueDiv.appendChild(issueLabel);
        var badges = document.createElement('div');
        badges.className = 'page-issue-badges';
        badges.style.marginTop = '0.3rem';
        data.issues.forEach(function(iss) {
            var badge = document.createElement('span');
            badge.className = 'page-badge badge-' + iss.severity;
            badge.textContent = iss.name;
            badge.style.cursor = 'pointer';
            badge.onclick = function() { showIssueFromPage(iss.name); };
            badges.appendChild(badge);
        });
        issueDiv.appendChild(badges);
        panel.appendChild(issueDiv);
    }
}

function renderDirDetail(path, header) {
    var panel = document.getElementById('structure-detail');
    while (panel.firstChild) panel.removeChild(panel.firstChild);

    var title = document.createElement('h3');
    title.className = 'detail-title';
    title.textContent = path;
    panel.appendChild(title);

    // Get stats from the header
    var countEl = header.querySelector('.dir-count');
    var timeEl = header.querySelector('.dir-time');

    var stats = document.createElement('div');
    stats.className = 'dir-detail-stats';
    if (countEl) {
        var s = document.createElement('span');
        s.textContent = countEl.textContent;
        stats.appendChild(s);
    }
    if (timeEl) {
        var dot = document.createTextNode(' \u00b7 ');
        stats.appendChild(dot);
        var t = document.createElement('span');
        t.textContent = timeEl.textContent;
        stats.appendChild(t);
    }
    panel.appendChild(stats);

    // List child pages
    var childPages = header.parentElement.querySelectorAll('.page-node');
    if (childPages.length > 0) {
        var listLabel = document.createElement('div');
        listLabel.className = 'label';
        listLabel.style.marginTop = '1rem';
        listLabel.textContent = 'Pages in this section';
        panel.appendChild(listLabel);

        var list = document.createElement('div');
        list.className = 'dir-page-list';
        childPages.forEach(function(pn) {
            var item = document.createElement('div');
            item.className = 'dir-page-item';
            var name = document.createElement('span');
            name.className = 'dir-page-name';
            name.textContent = pn.querySelector('.page-name').textContent;
            item.appendChild(name);
            var pageStats = document.createElement('span');
            pageStats.className = 'dir-page-stats';
            pageStats.textContent = pn.querySelector('.page-words').textContent + ' \u00b7 ' + pn.querySelector('.page-time').textContent;
            item.appendChild(pageStats);
            list.appendChild(item);
        });
        panel.appendChild(list);
    }
}

// Score breakdown toggle
function toggleScoreBreakdown() {
    var el = document.getElementById('score-breakdown');
    var arrow = document.getElementById('score-toggle-arrow');
    if (el.style.display === 'none') {
        el.style.display = 'block';
        arrow.textContent = '\u25BC';
    } else {
        el.style.display = 'none';
        arrow.textContent = '\u25B6';
    }
}

// Cross-tab navigation
function showPageFromIssue(url) {
    // Switch to Pages tab
    var pagesTab = document.getElementById('nav-pages');
    switchTab('pages', pagesTab);

    // Set filter to URL and apply
    var filter = document.getElementById('page-filter');
    var path = url;
    try { path = new URL(url).pathname; } catch(e) {}
    filter.value = path;
    filterPages();

    // Find and highlight matching row
    setTimeout(function() {
        var rows = document.querySelectorAll('#pages-table tbody tr.expandable');
        for (var i = 0; i < rows.length; i++) {
            if (rows[i].style.display !== 'none' && rows[i].cells[0].textContent.trim().includes(path)) {
                rows[i].scrollIntoView({ behavior: 'smooth', block: 'center' });
                rows[i].style.outline = '2px solid var(--accent-light)';
                rows[i].style.outlineOffset = '-2px';
                setTimeout(function(r) { r.style.outline = ''; r.style.outlineOffset = ''; }, 2000, rows[i]);
                break;
            }
        }
    }, 100);
}

function showIssueFromPage(displayName) {
    // Switch to Issues tab
    var issuesTab = document.getElementById('nav-issues');
    switchTab('issues', issuesTab);

    // Find the issue header matching the display name
    setTimeout(function() {
        var headers = document.querySelectorAll('.issue-header');
        for (var i = 0; i < headers.length; i++) {
            if (headers[i].textContent.includes(displayName)) {
                // Find the category panel this header belongs to
                var panel = headers[i].closest('.category-panel');
                if (panel) {
                    var panelId = panel.id.replace('panel-', '');
                    showCategory(panelId);
                }
                // Open the issue if not already open
                if (!headers[i].classList.contains('open')) {
                    toggleIssue(headers[i]);
                }
                // Scroll and highlight
                headers[i].scrollIntoView({ behavior: 'smooth', block: 'center' });
                headers[i].style.outline = '2px solid var(--accent-light)';
                headers[i].style.outlineOffset = '-2px';
                setTimeout(function(h) { h.style.outline = ''; h.style.outlineOffset = ''; }, 2000, headers[i]);
                break;
            }
        }
    }, 100);
}

// Init pagination on load
updateFilteredRows();
applyPagination();

// Category sidebar navigation
function showCategory(id) {
    document.querySelectorAll('.category-panel').forEach(function(p) { p.style.display = 'none'; });
    document.querySelectorAll('.category-nav-item').forEach(function(n) { n.classList.remove('active'); });
    var panel = document.getElementById('panel-' + id);
    var nav = document.getElementById('nav-' + id);
    if (panel) panel.style.display = 'block';
    if (nav) nav.classList.add('active');
}
// Open the most important category first: prefer critical, then warning, then
// info, then a clean one — so the reader lands on actionable issues, not an
// empty "well-defended" panel.
(function () {
    var order = ['status-critical', 'status-warning', 'status-info', 'status-clean'];
    var target = null;
    for (var i = 0; i < order.length && !target; i++) {
        target = document.querySelector('.category-nav-item.' + order[i]);
    }
    target = target || document.querySelector('.category-nav-item');
    if (target) {
        var id = target.id.replace(/^nav-/, '');
        showCategory(id);
    }
})();
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::config::CrawlConfig;
    use crate::models::crawl_result::{CrawlIssues, CrawlMetadata, ExportSettings};
    use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};
    use crate::models::page::{PageData, RedirectHop};

    fn page(url: &str, status: u16, html: &str) -> PageData {
        let mut p = crate::crawler::parser::parse_html(html, url);
        p.status = status;
        p.content_type = Some("text/html".to_string());
        p
    }

    fn issue(
        category: IssueCategory,
        check: &str,
        display_name: &str,
        severity: Severity,
        urls: &[&str],
    ) -> Issue {
        Issue {
            category,
            check: check.to_string(),
            display_name: display_name.to_string(),
            severity,
            description: "why".to_string(),
            guidance: "how".to_string(),
            urls: urls
                .iter()
                .map(|u| IssueUrl {
                    url: (*u).to_string(),
                    detail: None,
                })
                .collect(),
        }
    }

    fn result_with(pages: Vec<PageData>, issues: Vec<Issue>) -> CrawlResult {
        let mut summary = crate::crawler::build_summary(&pages);
        summary.issues_by_category = crate::analysis::scoring::build_category_summaries(&issues);
        let (c, w, i) = crate::analysis::scoring::count_urls_by_severity(&issues);
        summary.critical_issues = c;
        summary.warning_issues = w;
        summary.info_issues = i;
        summary.total_issues = issues.len() as u32;

        CrawlResult {
            slither_version: "test".to_string(),
            crawl_metadata: CrawlMetadata {
                domain: "test.com".to_string(),
                seed_url: "https://test.com/".to_string(),
                crawl_date: "2026-01-01T00:00:00Z".to_string(),
                duration_ms: 0,
                pages_discovered: pages.len() as u32,
                pages_crawled: pages.len() as u32,
                pages_skipped_robots: 0,
                pages_errored: 0,
                settings: CrawlConfig::default(),
                backend: "local".to_string(),
            },
            export_settings: ExportSettings::default(),
            pages,
            issues: CrawlIssues { issues },
            summary,
            robots_txt: None,
            sitemap_data: None,
        }
    }

    /// Everything the renderer emits for one category panel, so an assertion
    /// about "Sitemaps" cannot accidentally be satisfied by another category.
    fn panel<'a>(html: &'a str, id: &str) -> &'a str {
        let start = html
            .find(&format!(r#"id="panel-{id}""#))
            .unwrap_or_else(|| panic!("no panel for {id}"));
        let rest = &html[start..];
        let end = rest
            .find(r#"<div class="category-panel""#)
            .unwrap_or(rest.len());
        &rest[..end]
    }

    /// Regression (B5): the affected-URL figure summed one row per issue rather
    /// than counting distinct URLs, so a category with two checks over the same
    /// two pages claimed four URLs — on a real crawl "Images — 2 issues found ·
    /// 20 URLs affected" over 10 pages, contradicting the JSON, CLI and MCP,
    /// which all de-duplicate via `scoring::build_category_summaries`.
    /// Regression: pages are recorded at the URL that finally served them, but a
    /// redirect finding is filed against the URL that *redirects* — which has no
    /// page row of its own. Joining issue URLs to page rows by string alone
    /// silently dropped every redirect badge from the Pages tab and left the
    /// Issues tab linking to a row that does not exist.
    #[test]
    fn a_finding_filed_against_a_redirecting_url_lands_on_the_page_that_served_it() {
        let mut landed = page(
            "https://test.com/new/",
            200,
            "<html><head></head><body></body></html>",
        );
        landed.redirect_chain = Some(vec![RedirectHop {
            status: 301,
            url: "https://test.com/old".to_string(),
        }]);

        let issues = vec![issue(
            IssueCategory::ResponseCodes,
            "internal_redirect",
            "3xx Redirects",
            Severity::Warning,
            &["https://test.com/old"],
        )];

        let html = render_html_report(&result_with(vec![landed], issues)).unwrap();

        // Asserting the name merely appears is not enough: the Overview and
        // Issues tabs render it straight from the issue list, with no page join
        // at all, so a broken join still leaves two copies in the file. Only the
        // Pages-tab badge and the structure island go through the join.
        assert!(
            html.contains(r#"<span class="page-badge badge-warning">3xx Redirects</span>"#),
            "the Pages tab shows no redirect badge, so the finding never reached \
             the row for the page that served the redirect"
        );

        let island = html
            .split(r#""https://test.com/new/":"#)
            .nth(1)
            .expect("no structure-island entry for the page that served the redirect");
        let entry = &island[..island.len().min(600)];
        assert!(
            entry.contains(r#""name":"3xx Redirects""#),
            "the structure island records no redirect finding against the serving \
             page; got: {entry}"
        );
    }

    #[test]
    fn affected_url_counts_are_distinct_urls_not_issue_rows() {
        let pages = vec![
            page(
                "https://test.com/a",
                200,
                "<html><head></head><body></body></html>",
            ),
            page(
                "https://test.com/b",
                200,
                "<html><head></head><body></body></html>",
            ),
        ];
        let urls = ["https://test.com/a", "https://test.com/b"];
        let issues = vec![
            issue(
                IssueCategory::Images,
                "missing_alt",
                "Missing Alt Text",
                Severity::Warning,
                &urls,
            ),
            issue(
                IssueCategory::Images,
                "oversized",
                "Oversized Images",
                Severity::Info,
                &urls,
            ),
        ];
        let expected =
            crate::analysis::scoring::build_category_summaries(&issues)["Images"].affected_urls;
        assert_eq!(expected, 2, "the shared rule counts distinct URLs");

        let html = render_html_report(&result_with(pages, issues)).unwrap();
        let images = panel(&html, "images");
        assert!(
            images.contains("2 issues found &middot; 2 URLs affected"),
            "the HTML must report the same distinct-URL count as the JSON, got: {}",
            images
                .lines()
                .find(|l| l.contains("category-stats"))
                .unwrap_or("<no stats line>")
        );
    }

    /// Regression (B6): site-level findings legitimately carry no URL list, and
    /// a `!urls.is_empty()` filter dropped every one of them — the HTML was the
    /// only artifact that hid the Sitemaps warning, rendering "0 issues found",
    /// a green dot and "All checks passed" while the severity bar above still
    /// counted the issue.
    #[test]
    fn site_level_issues_are_rendered_rather_than_dropped() {
        let pages = vec![page(
            "https://test.com/",
            200,
            "<html><head></head><body></body></html>",
        )];
        let issues = vec![issue(
            IssueCategory::Sitemaps,
            "no_sitemap_found",
            "No Sitemap Found",
            Severity::Warning,
            &[],
        )];

        let html = render_html_report(&result_with(pages, issues)).unwrap();
        let sitemaps = panel(&html, "sitemaps");

        assert!(
            html.contains("No Sitemap Found"),
            "a site-level issue must appear in the report at all"
        );
        assert!(
            sitemaps.contains("1 issue found"),
            "the category must own up to holding an issue"
        );
        assert!(
            !sitemaps.contains("category-clean"),
            "a category holding an issue must not claim all checks passed"
        );
        assert!(
            html.contains(r#"status-warning" onclick="showCategory('sitemaps')"#),
            "the sidebar dot must carry the issue's severity, not read clean"
        );
    }

    /// The same for an info-severity site-level finding, which is the other
    /// shape this bug took (`ai_crawlers_blocked`).
    #[test]
    fn info_severity_site_level_issues_are_rendered_too() {
        let pages = vec![page(
            "https://test.com/",
            200,
            "<html><head></head><body></body></html>",
        )];
        let issues = vec![issue(
            IssueCategory::Directives,
            "ai_crawlers_blocked",
            "AI Crawlers Blocked in robots.txt",
            Severity::Info,
            &[],
        )];

        let html = render_html_report(&result_with(pages, issues)).unwrap();
        let directives = panel(&html, "directives");
        assert!(html.contains("AI Crawlers Blocked in robots.txt"));
        assert!(!directives.contains("category-clean"));
        assert!(html.contains(r#"status-info" onclick="showCategory('directives')"#));
    }

    /// A genuinely clean category must still say so — the fix must not make
    /// every panel claim it holds issues.
    #[test]
    fn a_category_with_no_issues_still_reads_clean() {
        let pages = vec![page(
            "https://test.com/",
            200,
            "<html><head></head><body></body></html>",
        )];
        let html = render_html_report(&result_with(pages, Vec::new())).unwrap();
        let sitemaps = panel(&html, "sitemaps");
        assert!(sitemaps.contains("0 issues found &middot; 0 URLs affected"));
        assert!(sitemaps.contains("category-clean"));
    }

    /// Regression (B9): indexability is the shared `is_indexable_html` rule
    /// (2xx HTML, not noindex). Testing only the robots directives rendered a
    /// green "Indexable" badge on every 404, 500 and 403.
    #[test]
    fn error_pages_are_not_labelled_indexable() {
        for status in [404u16, 500, 403] {
            let pages = vec![page(
                "https://test.com/gone",
                status,
                "<html><head><title>Error</title></head><body></body></html>",
            )];
            let html = render_html_report(&result_with(pages, Vec::new())).unwrap();
            assert!(
                html.contains(r#"<span class="idx-no">Noindex</span>"#),
                "a {status} page must not read as indexable"
            );
            assert!(
                !html.contains(r#"<span class="idx-yes">Indexable</span>"#),
                "a {status} page must not read as indexable"
            );
            assert!(
                html.contains("\"indexable\":false"),
                "the Structure tab's detail island must agree for {status}"
            );
        }
    }

    /// Regression: every redirect chain rendered with a dangling arrow pointing
    /// at nothing. Each hop records the URL that *served* the redirect, so the
    /// destination of the final hop is the page's own (post-redirect) URL and
    /// was never emitted.
    #[test]
    fn a_redirect_chain_ends_at_its_destination() {
        let chain = Some(vec![
            RedirectHop {
                status: 301,
                url: "https://test.com/old".to_string(),
            },
            RedirectHop {
                status: 302,
                url: "https://test.com/mid".to_string(),
            },
        ]);
        let rendered = format_redirect_chain(&chain, "https://test.com/final");

        assert!(
            rendered.contains("https://test.com/final"),
            "the destination must be rendered: {rendered}"
        );
        assert!(
            !rendered.trim_end().ends_with("</span>--302--> ")
                && !rendered.contains("--302--> </span></div>"),
            "the chain must not end on an arrow: {rendered}"
        );
        assert!(
            rendered.trim_end().ends_with("</span>"),
            "the chain must end on a URL, not an arrow: {rendered}"
        );
        // Order: old --301--> mid --302--> final
        let old = rendered.find("/old").unwrap();
        let mid = rendered.find("/mid").unwrap();
        let fin = rendered.find("/final").unwrap();
        assert!(old < mid && mid < fin, "hops out of order: {rendered}");
    }

    /// A page fetched without any redirect must render nothing at all, not a
    /// lone URL that reads as a redirect to itself.
    #[test]
    fn a_page_with_no_redirect_renders_no_chain() {
        assert_eq!(format_redirect_chain(&None, "https://test.com/"), "");
        assert_eq!(
            format_redirect_chain(&Some(Vec::new()), "https://test.com/"),
            ""
        );
    }

    /// A redirect destination is crawled content and must be escaped like any
    /// other, since `redirect_chain_display` is emitted through `|safe`.
    #[test]
    fn the_redirect_destination_is_html_escaped() {
        let chain = Some(vec![RedirectHop {
            status: 301,
            url: "https://test.com/old".to_string(),
        }]);
        let rendered =
            format_redirect_chain(&chain, "https://test.com/\"><script>alert(1)</script>");
        assert!(!rendered.contains("<script>"), "unescaped: {rendered}");
        assert!(rendered.contains("&lt;script&gt;"));
    }
}
