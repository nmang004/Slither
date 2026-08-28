use crate::analysis::{norm_url as norm, AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};
use crate::models::page::CanonicalSource;
use std::collections::HashMap;

pub struct CanonicalsAnalyzer;

impl Analyzer for CanonicalsAnalyzer {
    fn name(&self) -> &str {
        "Canonicals"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Canonicals
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_non_indexable_target(ctx, &mut issues);
        self.check_multiple_conflicting(ctx, &mut issues);
        self.check_canonical_relative(ctx, &mut issues);
        self.check_canonical_mismatch(ctx, &mut issues);
        self.check_missing(ctx, &mut issues);
        self.check_canonicalised(ctx, &mut issues);
        self.check_outside_head(ctx, &mut issues);
        self.check_chains_and_loops(ctx, &mut issues);
        // Self-referencing canonicals are a best practice, not a problem, so
        // they are no longer emitted as an issue (they previously deducted
        // points and saturated the Canonicals info cap).
        issues
    }
}

/// True if this page's canonical is one search engines will actually honour.
///
/// `canonical_source` is `None` only when a canonical was found outside
/// `<head>` and no `Link:` header supplied one. Google explicitly ignores those,
/// so treating them as authoritative reported the page as canonicalised — and
/// could raise a CRITICAL "canonical to non-indexable" — for a tag that has no
/// effect.
fn has_effective_canonical(page: &crate::models::page::PageData) -> bool {
    page.canonical.is_some() && page.canonical_source.is_some()
}

impl CanonicalsAnalyzer {
    /// A canonical placed outside `<head>` is ignored by Google, so the page is
    /// effectively unannotated — worth saying, since the author believed
    /// otherwise.
    fn check_outside_head(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| crate::analysis::is_indexable_html(p))
            .filter(|p| p.canonical.is_some() && p.canonical_source.is_none())
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "Canonical to {} is outside <head> and will be ignored",
                    p.canonical.as_deref().unwrap_or("(unknown)")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Canonicals,
                check: "canonical_outside_head".to_string(),
                display_name: "Canonical Outside Head".to_string(),
                severity: Severity::Warning,
                description: "Canonical link found outside the <head> section".to_string(),
                guidance: "Google only honours rel=canonical inside <head>. A canonical placed in the body is ignored, leaving the page unannotated. Move the <link rel=\"canonical\"> into <head>, or check whether an invalid element earlier in the head is terminating it early.".to_string(),
                urls: affected,
            });
        }
    }

    /// Canonical chains (A -> B -> C) and loops (A -> B -> A).
    ///
    /// A flat per-page check cannot tell these apart from a correct
    /// `A -> B (self)` annotation, so all three looked identical in the report.
    /// A loop is unresolvable — no page in the cycle declares itself canonical,
    /// so the annotation is discarded and the pages are silently dropped from
    /// the generated sitemap with no explanation.
    fn check_chains_and_loops(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let page_map: HashMap<String, &crate::models::page::PageData> =
            ctx.pages.iter().map(|p| (norm(&p.url), p)).collect();

        let mut loops: Vec<IssueUrl> = Vec::new();
        let mut chains: Vec<IssueUrl> = Vec::new();

        for page in ctx.pages.iter().filter(|p| has_effective_canonical(p)) {
            let start = norm(&page.url);
            let mut seen: Vec<String> = vec![start.clone()];
            let mut current = start.clone();

            // Walk the canonical pointers. Bounded by the crawl size, and the
            // cycle check terminates before that in practice.
            while let Some(node) = page_map.get(&current) {
                if !has_effective_canonical(node) {
                    break;
                }
                let Some(next) = node.canonical.as_ref().map(|c| norm(c)) else {
                    break;
                };
                if next == current {
                    break; // self-canonical: the chain resolves here
                }
                if seen.contains(&next) {
                    if next == start {
                        loops.push(IssueUrl {
                            url: page.url.clone(),
                            detail: Some(format!(
                                "Canonical loop: {}",
                                crate::analysis::detail_sample(&seen, seen.len())
                            )),
                        });
                    }
                    break;
                }
                seen.push(next.clone());
                current = next;
                if seen.len() > 2 {
                    chains.push(IssueUrl {
                        url: page.url.clone(),
                        detail: Some(format!(
                            "{} hops: {}",
                            seen.len() - 1,
                            crate::analysis::detail_sample(&seen, seen.len())
                        )),
                    });
                    break;
                }
            }
        }

        if !loops.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Canonicals,
                check: "canonical_loop".to_string(),
                display_name: "Canonical Loop".to_string(),
                severity: Severity::Warning,
                description: "Pages whose canonical tags point at each other in a cycle".to_string(),
                guidance: "No page in this cycle declares itself canonical, so search engines cannot resolve which URL to index and will fall back to their own choice. Pick one preferred URL and make every page in the group — including that one — point at it.".to_string(),
                urls: loops,
            });
        }
        if !chains.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Canonicals,
                check: "canonical_chain".to_string(),
                display_name: "Canonical Chain".to_string(),
                severity: Severity::Warning,
                description: "Canonical tags pointing to a page that is itself canonicalised".to_string(),
                guidance: "Canonical tags should point directly at the final preferred URL. A chain requires search engines to follow multiple hops and may not be followed at all. Point each page straight at the destination.".to_string(),
                urls: chains,
            });
        }
    }

    fn check_non_indexable_target(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        // Normalize both sides: a canonical written with a different trailing
        // slash than the crawled URL points at the same page, and comparing raw
        // strings silently skips genuinely broken canonical targets.
        let page_map: HashMap<String, &crate::models::page::PageData> =
            ctx.pages.iter().map(|p| (norm(&p.url), p)).collect();

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| (200..300).contains(&p.status))
            // A canonical Google ignores cannot point the page anywhere.
            .filter(|p| has_effective_canonical(p))
            .filter(|p| {
                if let Some(canonical) = &p.canonical {
                    if norm(canonical) == norm(&p.url) {
                        return false;
                    }
                    if let Some(target) = page_map.get(&norm(canonical)) {
                        let is_error = target.status >= 400;
                        is_error || target.is_noindex()
                    } else {
                        false
                    }
                } else {
                    false
                }
            })
            .map(|p| {
                let canonical = p.canonical.as_ref().unwrap();
                let target = page_map.get(&norm(canonical)).unwrap();
                let reason = if target.status >= 400 {
                    format!("Target status {}", target.status)
                } else {
                    "Target has noindex".to_string()
                };
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!("Canonical {} - {}", canonical, reason)),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Canonicals,
                check: "non_indexable_target".to_string(),
                display_name: "Canonical to Non-Indexable".to_string(),
                severity: Severity::Critical,
                description: "Canonical URLs pointing to non-indexable pages".to_string(),
                guidance: "Canonical tags should point to indexable, 200-status pages. Pointing to 4xx/5xx pages or noindexed pages wastes crawl budget and confuses search engines about the preferred URL.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_multiple_conflicting(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.canonical_count > 1)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("{} canonical tags found", p.canonical_count)),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Canonicals,
                check: "multiple_conflicting".to_string(),
                display_name: "Multiple Canonical Tags".to_string(),
                severity: Severity::Warning,
                description: "Pages with multiple canonical tags".to_string(),
                guidance: "Each page should have exactly one canonical tag. Multiple canonical tags confuse search engines about the preferred URL. Remove duplicate canonical tags and keep only one.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_canonical_relative(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.canonical_is_relative)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "Relative canonical: {}",
                    p.canonical.as_deref().unwrap_or("(unknown)")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Canonicals,
                check: "canonical_relative".to_string(),
                display_name: "Relative Canonical URLs".to_string(),
                severity: Severity::Warning,
                description: "Pages with relative canonical URLs".to_string(),
                guidance: "Canonical URLs should be absolute (starting with https://) to avoid ambiguity. Relative canonicals may be resolved incorrectly by search engines.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_canonical_mismatch(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| matches!(p.canonical_source, Some(CanonicalSource::Both)))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Canonical specified in both HTML and HTTP header".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Canonicals,
                check: "canonical_mismatch".to_string(),
                // Only one canonical value is retained, so we cannot tell
                // whether the two sources agree. Report the duplication rather
                // than asserting a conflict that may not exist.
                display_name: "Canonical Declared Twice".to_string(),
                severity: Severity::Info,
                description: "Pages declaring a canonical in both the HTML and the HTTP header"
                    .to_string(),
                guidance: "This page declares a canonical URL in both the HTML <link> tag and the HTTP Link header. That is valid as long as the two agree, but it is easy for them to drift apart during a redesign or migration. Prefer declaring the canonical in one place.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.is_html_page() && p.canonical.is_none())
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: None,
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Canonicals,
                check: "missing".to_string(),
                display_name: "Missing Canonical Tag".to_string(),
                severity: Severity::Warning,
                description: "Pages missing a canonical tag".to_string(),
                guidance: "Every indexable page should have a canonical tag (usually self-referencing) to signal the preferred URL to search engines and prevent duplicate content issues.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_canonicalised(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| has_effective_canonical(p) && !p.has_self_canonical)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "Canonicalised to {}",
                    p.canonical.as_deref().unwrap_or("(unknown)")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Canonicals,
                check: "canonicalised".to_string(),
                display_name: "Canonicalised to Different URL".to_string(),
                severity: Severity::Info,
                description: "Pages canonicalised to a different URL".to_string(),
                guidance: "These pages declare a different URL as the canonical version. Verify this is intentional and that the canonical target is the correct preferred URL.".to_string(),
                urls: affected,
            });
        }
    }
}
