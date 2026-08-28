use crate::models::issue::{Issue, IssueCategory, Severity};
use serde::Serialize;
use std::collections::HashMap;

pub struct HealthGrade {
    pub score: u32,
    pub letter: String,
    pub verdict: String,
    pub color: String,
    pub deductions: Vec<CategoryDeduction>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryDeduction {
    pub category: String,
    pub critical_count: u32,
    pub warning_count: u32,
    pub info_count: u32,
    pub points_deducted: f64,
}

/// How much a given check can cost the health score.
///
/// Tiers come from `docs/scoring-recalibration.md`, which grounds them in
/// Google Search Central guidance first and industry tiering (Screaming Frog,
/// Ahrefs, Lighthouse) where Google is silent. The short version: things that
/// remove a page from the index dominate; things Google has explicitly said are
/// not ranking factors do not score at all.
#[derive(Clone, Copy, PartialEq)]
enum Impact {
    /// The page itself is broken — Google drops or refuses to index it.
    PageBroken,
    /// The page itself is materially defective for ranking.
    PageDefective,
    /// A site- or link-level fault: real, but the page is not itself broken.
    SiteHigh,
    /// Secondary technical problems.
    SiteMedium,
    /// Real but minor. Collectively bounded so they cannot dominate a score.
    Minor,
    /// Reported, never scored — not a ranking factor.
    Unscored,
}

/// A broken page costs its full share of the site, matching the one published
/// industry formula (Ahrefs: health tracks the share of pages without errors).
const BROKEN_WEIGHT: f64 = 100.0;
/// A materially defective page costs about half what a broken one does.
const DEFECTIVE_WEIGHT: f64 = 55.0;

const SITE_HIGH_WEIGHT: f64 = 12.0;
const SITE_HIGH_CAP: f64 = 24.0;
const SITE_MEDIUM_WEIGHT: f64 = 4.0;
const SITE_MEDIUM_CAP: f64 = 14.0;
const MINOR_WEIGHT: f64 = 1.0;
/// Minor findings are near-universal — every real site has some long meta
/// descriptions and imperfect heading order. Capping their combined cost is what
/// keeps a well-built site in A/B territory instead of taxing it into a D.
const MINOR_CAP: f64 = 12.0;

/// A single instance still registers on a large site.
const COVERAGE_FLOOR: f64 = 0.12;
/// A proportion measured over one or two pages is noise, so the denominator has
/// a floor: one bad page on a tiny crawl is not "100% of the site".
const MIN_DENOMINATOR: u32 = 5;

fn impact_of(category: &IssueCategory, check: &str) -> Impact {
    use IssueCategory as C;
    match (category, check) {
        // --- Removed from or blocked out of the index -----------------------
        // Google: a 4xx URL "is removed from the index"; 5xx/429 URLs are
        // "eventually dropped".
        (C::ResponseCodes, "internal_server_error" | "internal_client_error")
        | (C::ResponseCodes, "internal_no_response" | "internal_redirect_loop")
        | (C::Canonicals, "non_indexable_target")
        // Only the index/noindex contradiction removes the page from the
        // index. `conflicting_follow_directives` leaves it fully indexed and is
        // scored far lower — scoring both here drove an entirely indexable site
        // to 0/F on a follow-vs-nofollow mismatch.
        | (C::Directives, "conflicting_directives") => Impact::PageBroken,

        // --- The page ranks materially worse than it should -----------------
        (C::PageTitles, "missing")
        | (C::Content, "exact_duplicate" | "soft_404")
        | (C::Security, "http_urls")
        | (C::Links, "orphan_pages") => Impact::PageDefective,

        // --- Site- or link-level faults -------------------------------------
        // A page linking to a dead URL is not itself broken, and the dead target
        // is already counted above — so this is bounded rather than a defect.
        (C::Links, "broken_internal")
        | (C::Canonicals, "multiple_conflicting")
        // Real and worth fixing, but a page loading an insecure subresource is
        // still indexed — unlike the page itself being served over HTTP.
        | (C::Security, "mixed_content" | "insecure_form_action" | "form_on_http") => {
            Impact::SiteHigh
        }

        (C::PageTitles, "duplicate" | "multiple_tags" | "outside_head")
        | (C::Canonicals, "missing" | "canonical_relative")
        | (C::Content, "low_content")
        | (C::ResponseCodes, "internal_redirect_chain" | "internal_access_restricted")
        // Enumerated, not a wildcard: the blanket `(C::Hreflang, _)` scored the
        // Info-severity, explicitly-optional `missing_x_default` in the same
        // tier as "Missing Canonical Tag", costing 4 of the 14 available
        // SiteMedium points for declining a Google suggestion. The Info-level
        // hreflang checks now fall through to Minor like every other category's.
        | (C::Hreflang, "missing_return_links" | "incorrect_lang_codes")
        | (C::Hreflang, "non_200_urls" | "non_canonical_urls" | "noindex_urls")
        | (C::Hreflang, "multiple_entries" | "conflicting_lang_declarations")
        | (C::Canonicals, "canonical_loop" | "canonical_chain" | "canonical_outside_head")
        | (C::Directives, "conflicting_follow_directives")
        | (C::Performance, "cwv_lcp_poor" | "cwv_inp_poor" | "cwv_cls_poor" | "cwv_score_poor")
        | (C::Performance, "slow_server_response")
        | (C::Links, "no_internal_outlinks" | "localhost_outlinks")
        // Google: a site that is small and "comprehensively linked internally"
        // may not need a sitemap at all, so its absence is a suggestion rather
        // than a fault. zh.wikipedia.org publishes none and indexes fine.
        | (C::Sitemaps, "no_sitemap_found")
        | (C::Sitemaps, "non_indexable_in_sitemap" | "over_50k_urls" | "over_50mb")
        | (C::StructuredData, "parse_errors")
        | (C::Url, "contains_space")
        | (C::JavaScript, "js_injected_title" | "js_injected_description")
        | (C::JavaScript, "js_injected_canonical" | "js_injected_h1")
        | (C::JavaScript, "js_injected_structured_data") => Impact::SiteMedium,

        // --- Not ranking factors --------------------------------------------
        // Mueller on response headers: "the security headers are more about,
        // well, security." HTTPS itself is scored above.
        (C::Security, "missing_hsts" | "missing_csp" | "missing_x_content_type_options")
        | (C::Security, "missing_x_frame_options" | "missing_referrer_policy")
        | (C::Security, "unsafe_cross_origin")
        // A deliberate policy choice, not a defect.
        | (C::Directives, "ai_crawlers_blocked" | "noai_directive")
        // Usually intentional.
        | (C::Url, "has_parameters" | "ga_tracking_params") => Impact::Unscored,

        // Everything else is real but minor: meta descriptions (snippet input,
        // not ranking), headings (Mueller: a site "will rank perfectly fine
        // with no H1 tags"), image alt (accessibility and Google Images),
        // structured data (rich-result eligibility), URL cosmetics.
        _ => Impact::Minor,
    }
}

/// A crawl that produced nothing analysable has no health to report. Returning
/// a number here is worse than returning none: a bot wall that answered every
/// request with 403 scored 92/A, which reads as "excellent site" when it
/// actually means "we were not allowed to look".
fn unscorable(reason: &str) -> HealthGrade {
    HealthGrade {
        score: 0,
        letter: "–".to_string(),
        verdict: reason.to_string(),
        color: "#64748b".to_string(),
        deductions: Vec::new(),
    }
}

/// How much of the site a bounded finding covers. Square-rooted so a problem on
/// a tenth of a large site still registers instead of rounding to nothing.
fn coverage(affected: usize, total_pages: u32) -> f64 {
    if affected == 0 {
        return 1.0; // a site-level finding with no URL list applies site-wide
    }
    let ratio = affected as f64 / total_pages.max(1) as f64;
    ratio.sqrt().clamp(COVERAGE_FLOOR, 1.0)
}

pub fn compute_health_score(issues: &[Issue], total_pages: u32) -> HealthGrade {
    use std::collections::HashSet;

    // Nothing was crawled, so there is nothing to grade. Reporting 100/A here
    // presented "we could not reach or were not allowed to crawl this site" as
    // a flawless result — observed on a seed blocked by robots.txt.
    if total_pages == 0 {
        return unscorable("No pages crawled");
    }

    // Pages are unioned, not summed: a page broken two different ways is still
    // one broken page, and summing per-check coverage double-counts it.
    let mut broken: HashSet<&str> = HashSet::new();
    let mut defective: HashSet<&str> = HashSet::new();
    let mut site_high = 0.0f64;
    let mut site_medium = 0.0f64;
    let mut minor = 0.0f64;
    let (mut n_broken, mut n_defective, mut n_site, mut n_minor) = (0u32, 0u32, 0u32, 0u32);

    for issue in issues {
        let impact = impact_of(&issue.category, &issue.check);
        match impact {
            Impact::Unscored => continue,
            Impact::PageBroken => {
                n_broken += 1;
                if issue.urls.is_empty() {
                    broken.insert(issue.check.as_str());
                } else {
                    broken.extend(issue.urls.iter().map(|u| u.url.as_str()));
                }
            }
            Impact::PageDefective => {
                n_defective += 1;
                if issue.urls.is_empty() {
                    defective.insert(issue.check.as_str());
                } else {
                    defective.extend(issue.urls.iter().map(|u| u.url.as_str()));
                }
            }
            Impact::SiteHigh | Impact::SiteMedium | Impact::Minor => {
                let cov = coverage(issue.urls.len(), total_pages);
                match impact {
                    Impact::SiteHigh => {
                        n_site += 1;
                        site_high += SITE_HIGH_WEIGHT * cov;
                    }
                    Impact::SiteMedium => {
                        n_site += 1;
                        site_medium += SITE_MEDIUM_WEIGHT * cov;
                    }
                    _ => {
                        n_minor += 1;
                        minor += MINOR_WEIGHT * cov;
                    }
                }
            }
        }
    }

    // If every page we saw was non-2xx, we never actually looked at the site.
    // A bot wall answering 403 to everything previously scored 92/A.
    let unreachable: HashSet<&str> = issues
        .iter()
        .filter(|i| {
            matches!(i.category, IssueCategory::ResponseCodes)
                && matches!(
                    i.check.as_str(),
                    "internal_server_error"
                        | "internal_client_error"
                        | "internal_no_response"
                        | "internal_access_restricted"
                )
        })
        .flat_map(|i| i.urls.iter().map(|u| u.url.as_str()))
        .collect();
    if !unreachable.is_empty() && unreachable.len() >= total_pages as usize {
        return unscorable("Crawl blocked — no page could be analysed");
    }

    // A bot wall served with HTTP 200, or a client-rendered shell, reaches this
    // point looking like an ordinary crawl. Status alone cannot see it: an
    // Akamai "Client Challenge" interstitial is a 200, and 17 of 22 such pages
    // got Le Monde graded F (26/100) for the wall's properties. The signature is
    // that nearly every page is thin AND either byte-identical to the others or
    // has no links at all — i.e. we never saw the site.
    let distinct = |check: &str| -> usize {
        issues
            .iter()
            .filter(|i| i.check == check)
            .flat_map(|i| i.urls.iter().map(|u| u.url.as_str()))
            .collect::<HashSet<_>>()
            .len()
    };
    let share = |n: usize| n as f64 / total_pages as f64;
    const UNREADABLE_SHARE: f64 = 0.8;
    if share(distinct("low_content")) >= UNREADABLE_SHARE
        && (share(distinct("exact_duplicate")) >= UNREADABLE_SHARE
            || share(distinct("no_internal_outlinks")) >= UNREADABLE_SHARE)
    {
        return unscorable("No readable content — crawl blocked or client-rendered");
    }

    // A page already counted as broken is not penalized again as defective.
    for url in &broken {
        defective.remove(url);
    }

    let denom = total_pages.max(MIN_DENOMINATOR) as f64;
    let broken_points = BROKEN_WEIGHT * (broken.len() as f64 / denom).min(1.0);
    let defective_points = DEFECTIVE_WEIGHT * (defective.len() as f64 / denom).min(1.0);
    let site_points = site_high.min(SITE_HIGH_CAP) + site_medium.min(SITE_MEDIUM_CAP);
    let minor_points = minor.min(MINOR_CAP);

    let total_deduction = broken_points + defective_points + site_points + minor_points;
    let score = (100.0 - total_deduction).max(0.0) as u32;

    // The breakdown explains the score by impact, which is what a reader needs
    // in order to act — grouping by analyzer category hid whether a deduction
    // came from broken pages or from cosmetic findings.
    let mut deductions = Vec::new();
    let mut push = |label: &str, count: u32, points: f64| {
        if points > 0.05 {
            deductions.push(CategoryDeduction {
                category: label.to_string(),
                critical_count: count,
                warning_count: 0,
                info_count: 0,
                points_deducted: points,
            });
        }
    };
    push(
        &format!("Broken pages ({} of {})", broken.len(), total_pages),
        n_broken,
        broken_points,
    );
    push(
        &format!("Pages with major defects ({})", defective.len()),
        n_defective,
        defective_points,
    );
    push("Site-level issues", n_site, site_points);
    push("Minor issues (capped)", n_minor, minor_points);

    let (letter, verdict, color) = match score {
        90..=100 => ("A", "Excellent", "#00b894"),
        80..=89 => ("B", "Good", "#55efc4"),
        70..=79 => ("C", "Needs Work", "#fdcb6e"),
        60..=69 => ("D", "Poor", "#e67e22"),
        _ => ("F", "Critical", "#e17055"),
    };

    // The letter comes from the score, but the verdict is the word a user reads
    // first — calling a site "Critical" when nothing critical was found
    // misrepresents the result.
    let has_critical = issues.iter().any(|i| i.severity == Severity::Critical);
    let has_warning = issues.iter().any(|i| i.severity == Severity::Warning);
    let verdict = if letter == "F" && !has_critical {
        if has_warning {
            "Widespread Warnings"
        } else {
            "Many Minor Issues"
        }
    } else {
        verdict
    };

    HealthGrade {
        score,
        letter: letter.to_string(),
        verdict: verdict.to_string(),
        color: color.to_string(),
        deductions,
    }
}

/// Build category summaries from issues.
pub fn build_category_summaries(
    issues: &[Issue],
) -> HashMap<String, crate::models::crawl_result::CategorySummary> {
    use crate::models::crawl_result::CategorySummary;

    use std::collections::HashSet;

    let mut summaries: HashMap<String, CategorySummary> = HashMap::new();
    // Distinct URLs per category. Summing `issue.urls.len()` counted rows, not
    // URLs — a check that emits one row per (heading text, page) pair reported
    // "Headings — 5 issues, 676 URLs" on a 22-page crawl, a figure that cannot
    // be true and appears on the face of a client deliverable.
    let mut seen: HashMap<String, HashSet<&str>> = HashMap::new();

    for issue in issues {
        let key = format!("{}", issue.category);
        let entry = summaries.entry(key.clone()).or_insert(CategorySummary {
            total_checks: 0,
            issues_found: 0,
            affected_urls: 0,
            critical: 0,
            warning: 0,
            info: 0,
        });

        entry.total_checks += 1;
        entry.issues_found += 1;
        match issue.severity {
            Severity::Critical => entry.critical += 1,
            Severity::Warning => entry.warning += 1,
            Severity::Info => entry.info += 1,
        }

        let urls = seen.entry(key).or_default();
        for u in &issue.urls {
            urls.insert(u.url.as_str());
        }
    }

    for (key, urls) in seen {
        if let Some(entry) = summaries.get_mut(&key) {
            entry.affected_urls = urls.len() as u32;
        }
    }

    summaries
}

pub fn count_urls_by_severity(issues: &[Issue]) -> (u32, u32, u32) {
    // Each issue counts as affecting at least one URL so site-level issues
    // (empty URL list) are represented in the totals.
    let count = |sev: Severity| -> u32 {
        issues
            .iter()
            .filter(|i| i.severity == sev)
            .map(|i| i.urls.len().max(1) as u32)
            .sum()
    };
    (
        count(Severity::Critical),
        count(Severity::Warning),
        count(Severity::Info),
    )
}

pub fn compute_percentiles(response_times: &mut [u64]) -> (u32, u32) {
    if response_times.is_empty() {
        return (0, 0);
    }
    response_times.sort();
    let len = response_times.len();
    let p50 = if len.is_multiple_of(2) {
        ((response_times[len / 2 - 1] + response_times[len / 2]) / 2) as u32
    } else {
        response_times[len / 2] as u32
    };
    let p95_idx = (len as f64 * 0.95).ceil() as usize - 1;
    let p95 = response_times[p95_idx.min(len - 1)] as u32;
    (p50, p95)
}
