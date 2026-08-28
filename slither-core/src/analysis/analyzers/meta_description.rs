use std::collections::HashMap;

use crate::analysis::{is_indexable_html, AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

/// See the note on title length: character-count thresholds assume Latin text.
fn is_full_width_description(page: &crate::models::page::PageData) -> bool {
    page.meta_description
        .as_deref()
        .is_some_and(crate::utils::pixel_width::is_full_width_text)
}

pub struct MetaDescriptionAnalyzer;

impl Analyzer for MetaDescriptionAnalyzer {
    fn name(&self) -> &str {
        "Meta Description"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::MetaDescription
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_missing(ctx, &mut issues);
        self.check_duplicate(ctx, &mut issues);
        self.check_multiple_tags(ctx, &mut issues);
        self.check_outside_head(ctx, &mut issues);
        self.check_over_155_chars(ctx, &mut issues);
        self.check_below_70_chars(ctx, &mut issues);
        self.check_over_985_pixels(ctx, &mut issues);
        issues
    }
}

impl MetaDescriptionAnalyzer {
    fn check_missing(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.is_html_page())
            .filter(|p| p.meta_description.as_ref().is_none_or(|d| d.is_empty()))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Missing meta description".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::MetaDescription,
                check: "missing".to_string(),
                display_name: "Missing Meta Description".to_string(),
                severity: Severity::Warning,
                description: "Pages with missing meta descriptions".to_string(),
                guidance: "Meta descriptions provide a summary that appears in search results below the title. Add a unique, compelling description of 70-155 characters to improve click-through rates.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_duplicate(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        // Error pages and noindex URLs share boilerplate descriptions that the
        // owner cannot meaningfully de-duplicate; only indexable HTML counts.
        let mut desc_map: HashMap<&str, Vec<&str>> = HashMap::new();
        for page in ctx.pages.iter().filter(|p| is_indexable_html(p)) {
            if let Some(ref desc) = page.meta_description {
                if !desc.is_empty() {
                    desc_map
                        .entry(desc.as_str())
                        .or_default()
                        .push(page.url.as_str());
                }
            }
        }
        let affected: Vec<IssueUrl> = desc_map
            .into_iter()
            .filter(|(_, urls)| urls.len() >= 2)
            .flat_map(|(desc, urls)| {
                let truncated = if desc.chars().count() > 80 {
                    format!("{}...", desc.chars().take(80).collect::<String>())
                } else {
                    desc.to_string()
                };
                urls.into_iter().map(move |url| IssueUrl {
                    url: url.to_string(),
                    detail: Some(format!("Duplicate description: {}", truncated)),
                })
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::MetaDescription,
                check: "duplicate".to_string(),
                display_name: "Duplicate Descriptions".to_string(),
                severity: Severity::Warning,
                description: "Pages with duplicate meta descriptions".to_string(),
                guidance: "Each page should have a unique meta description. Duplicate descriptions reduce search engines' ability to differentiate pages and can lower click-through rates.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_multiple_tags(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| is_indexable_html(p) && p.meta_description_count > 1)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{} meta description tags found",
                    p.meta_description_count
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::MetaDescription,
                check: "multiple_tags".to_string(),
                display_name: "Multiple Meta Descriptions".to_string(),
                severity: Severity::Warning,
                description: "Pages with multiple meta description tags".to_string(),
                guidance: "Each page should have exactly one meta description tag. Multiple meta descriptions confuse search engines about which description to display. Remove the duplicates.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_outside_head(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                is_indexable_html(p) && !p.meta_desc_in_head && p.meta_description.is_some()
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Meta description found outside <head>".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::MetaDescription,
                check: "outside_head".to_string(),
                display_name: "Meta Description Outside Head".to_string(),
                severity: Severity::Warning,
                description: "Pages with meta descriptions outside the <head> section".to_string(),
                guidance: "The meta description tag must be placed within the <head> section. Meta tags outside <head> may be ignored by search engines.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_over_155_chars(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                is_indexable_html(p)
                    && !is_full_width_description(p)
                    && p.meta_description_length.is_some_and(|len| len > 155)
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{} characters",
                    p.meta_description_length.unwrap_or(0)
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::MetaDescription,
                check: "over_155_chars".to_string(),
                display_name: "Over 155 Characters".to_string(),
                severity: Severity::Info,
                description: "Pages with meta descriptions over 155 characters".to_string(),
                guidance: "Meta descriptions longer than 155 characters are likely to be truncated in search results. Keep descriptions concise and within 70-155 characters.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_below_70_chars(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                let has_desc = p.meta_description.as_ref().is_some_and(|d| !d.is_empty());
                is_indexable_html(p)
                    && has_desc
                    && !is_full_width_description(p)
                    && p.meta_description_length.is_some_and(|len| len < 70)
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{} characters",
                    p.meta_description_length.unwrap_or(0)
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::MetaDescription,
                check: "below_70_chars".to_string(),
                display_name: "Below 70 Characters".to_string(),
                severity: Severity::Info,
                description: "Pages with meta descriptions under 70 characters".to_string(),
                guidance: "Short meta descriptions may not fully utilize the search result snippet space. Expand descriptions to 70-155 characters with compelling copy that encourages clicks.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_over_985_pixels(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                is_indexable_html(p) && p.meta_description_pixel_width.is_some_and(|w| w > 985)
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{}px wide",
                    p.meta_description_pixel_width.unwrap_or(0)
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::MetaDescription,
                check: "over_985_pixels".to_string(),
                display_name: "Over 985px Wide".to_string(),
                severity: Severity::Info,
                description: "Pages with meta descriptions over 985 pixels wide".to_string(),
                guidance: "Meta descriptions wider than 985 pixels will likely be truncated in search results. Shorten the description to fit within the display limit.".to_string(),
                urls: affected,
            });
        }
    }
}
