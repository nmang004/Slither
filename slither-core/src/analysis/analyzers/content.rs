use std::collections::HashMap;

use crate::analysis::{is_indexable_html, AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

pub struct ContentAnalyzer;

impl Analyzer for ContentAnalyzer {
    fn name(&self) -> &str {
        "Content"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Content
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_exact_duplicate(ctx, &mut issues);
        self.check_soft_404(ctx, &mut issues);
        self.check_low_content(ctx, &mut issues);
        self.check_readability_very_difficult(ctx, &mut issues);
        self.check_readability_difficult(ctx, &mut issues);
        issues
    }
}

impl ContentAnalyzer {
    fn check_exact_duplicate(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let mut hash_map: HashMap<&str, Vec<&str>> = HashMap::new();
        for page in &ctx.pages {
            // Skip empty pages — they all hash the same and are not true
            // duplicates. Noindex pages are skipped too: a page kept out of the
            // index cannot dilute ranking signals, so pairing one with an
            // indexable page is not a duplicate-content problem to fix.
            // A URL that redirected is not a separate page from its
            // destination — svelte.dev's /docs/svelte 307s to
            // /docs/svelte/overview, and pairing the two produced false
            // "Exact duplicate content" (plus duplicate title, description and
            // H1) for a redirect the site had implemented correctly.
            if is_indexable_html(page)
                && page.redirect_chain.is_none()
                && !page.content_hash.is_empty()
                && !page.body_text.is_empty()
                && page.word_count > 0
            {
                hash_map
                    .entry(page.content_hash.as_str())
                    .or_default()
                    .push(page.url.as_str());
            }
        }
        let affected: Vec<IssueUrl> = hash_map
            .into_iter()
            .filter(|(_, urls)| urls.len() >= 2)
            .flat_map(|(hash, urls)| {
                let count = urls.len();
                urls.into_iter().map(move |url| IssueUrl {
                    url: url.to_string(),
                    detail: Some(format!(
                        "Exact duplicate content ({} pages share hash {})",
                        count,
                        &hash[..hash.len().min(12)]
                    )),
                })
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Content,
                check: "exact_duplicate".to_string(),
                display_name: "Duplicate Content".to_string(),
                severity: Severity::Warning,
                description: "Pages with exact duplicate content".to_string(),
                guidance: "Multiple pages with identical content waste crawl budget and dilute ranking signals. Consolidate duplicate pages using canonical tags, 301 redirects, or by removing the duplicates.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_soft_404(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.is_soft_404)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Page appears to be a soft 404".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Content,
                check: "soft_404".to_string(),
                display_name: "Soft 404 Pages".to_string(),
                severity: Severity::Warning,
                description: "Pages detected as soft 404s".to_string(),
                guidance: "Soft 404 pages return a 200 status but display error or placeholder content. Either return a proper 404 status code, add real content, or redirect to a relevant page.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_low_content(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                p.word_count < 100
                    && (200..300).contains(&p.status)
                    && p.content_type
                        .as_ref()
                        .is_some_and(|ct| crate::models::page::is_html_content_type(Some(ct)))
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("{} words", p.word_count)),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Content,
                check: "low_content".to_string(),
                display_name: "Low Word Count".to_string(),
                severity: Severity::Warning,
                description: "Pages with very low word count (under 100 words)".to_string(),
                guidance: "Thin content pages may be seen as low-quality by search engines. Add substantive, valuable content or consider consolidating thin pages with related content.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_readability_very_difficult(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.readability_score.is_some_and(|s| s < 10.0))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "Readability score: {:.1}",
                    p.readability_score.unwrap_or(0.0)
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Content,
                check: "readability_very_difficult".to_string(),
                display_name: "Very Difficult Readability".to_string(),
                severity: Severity::Info,
                description: "Pages with very difficult readability (score below 10)".to_string(),
                guidance: "Content with extremely low readability scores is very difficult for most users to understand. Simplify sentence structure, use shorter words, and break up long paragraphs.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_readability_difficult(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                p.readability_score
                    .is_some_and(|s| (10.0..30.0).contains(&s))
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "Readability score: {:.1}",
                    p.readability_score.unwrap_or(0.0)
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Content,
                check: "readability_difficult".to_string(),
                display_name: "Difficult Readability".to_string(),
                severity: Severity::Info,
                description: "Pages with difficult readability (score 10-30)".to_string(),
                guidance: "Content with low readability scores may be hard for many users to understand. Consider simplifying language where appropriate for your target audience.".to_string(),
                urls: affected,
            });
        }
    }
}
