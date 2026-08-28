use std::collections::HashMap;

use crate::analysis::{is_indexable_html, AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

/// Character-count guidance ("under 30 is too short", "over 60 truncates") is
/// derived from Latin text. A full-width script packs far more meaning and
/// rendered width into each character, so a 19-character Chinese title is a
/// normal-length title, not a short one. Pixel width is the meaningful measure
/// there and is checked separately.
fn is_full_width_title(page: &crate::models::page::PageData) -> bool {
    page.title
        .as_deref()
        .is_some_and(crate::utils::pixel_width::is_full_width_text)
}

pub struct PageTitlesAnalyzer;

impl Analyzer for PageTitlesAnalyzer {
    fn name(&self) -> &str {
        "Page Titles"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::PageTitles
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_missing(ctx, &mut issues);
        self.check_duplicate(ctx, &mut issues);
        self.check_multiple_tags(ctx, &mut issues);
        self.check_outside_head(ctx, &mut issues);
        self.check_over_60_chars(ctx, &mut issues);
        self.check_below_30_chars(ctx, &mut issues);
        self.check_over_561_pixels(ctx, &mut issues);
        self.check_below_200_pixels(ctx, &mut issues);
        issues
    }
}

impl PageTitlesAnalyzer {
    fn check_missing(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.is_html_page())
            .filter(|p| p.title.as_ref().is_none_or(|t| t.is_empty()))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Missing title tag".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::PageTitles,
                check: "missing".to_string(),
                display_name: "Missing Title Tag".to_string(),
                severity: Severity::Warning,
                description: "Pages with missing title tags".to_string(),
                guidance: "Every page should have a unique, descriptive title tag. Title tags are a primary ranking factor and appear as the clickable headline in search results.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_duplicate(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        // Only indexable HTML pages can have a duplicate-title problem: five
        // 404s sharing the boilerplate "404 Not Found" title is not a defect
        // the site owner can fix, and noindex pages never reach the index.
        let mut title_map: HashMap<&str, Vec<&str>> = HashMap::new();
        for page in ctx.pages.iter().filter(|p| is_indexable_html(p)) {
            if let Some(ref title) = page.title {
                if !title.is_empty() {
                    title_map
                        .entry(title.as_str())
                        .or_default()
                        .push(page.url.as_str());
                }
            }
        }
        let affected: Vec<IssueUrl> = title_map
            .into_iter()
            .filter(|(_, urls)| urls.len() >= 2)
            .flat_map(|(title, urls)| {
                urls.into_iter().map(move |url| IssueUrl {
                    url: url.to_string(),
                    detail: Some(format!("Duplicate title: {}", title)),
                })
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::PageTitles,
                check: "duplicate".to_string(),
                display_name: "Duplicate Titles".to_string(),
                severity: Severity::Warning,
                description: "Pages with duplicate title tags".to_string(),
                guidance: "Each page should have a unique title that accurately describes its content. Duplicate titles make it harder for search engines to differentiate pages and reduce click-through rates.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_multiple_tags(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| is_indexable_html(p) && p.title_count > 1)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("{} title tags found", p.title_count)),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::PageTitles,
                check: "multiple_tags".to_string(),
                display_name: "Multiple Title Tags".to_string(),
                severity: Severity::Warning,
                description: "Pages with multiple title tags".to_string(),
                guidance: "Each page should have exactly one title tag. Multiple title tags confuse search engines about which title to display. Remove duplicate title tags.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_outside_head(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| is_indexable_html(p) && !p.title_in_head && p.title.is_some())
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Title tag found outside <head>".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::PageTitles,
                check: "outside_head".to_string(),
                display_name: "Title Outside Head".to_string(),
                severity: Severity::Warning,
                description: "Pages with title tags outside the <head> section".to_string(),
                guidance: "The title tag must be placed within the <head> section of the HTML document. Title tags outside <head> may be ignored by search engines.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_over_60_chars(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                is_indexable_html(p)
                    && !is_full_width_title(p)
                    && p.title_length.is_some_and(|len| len > 60)
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{} characters: {}",
                    p.title_length.unwrap_or(0),
                    p.title.as_deref().unwrap_or("")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::PageTitles,
                check: "over_60_chars".to_string(),
                display_name: "Over 60 Characters".to_string(),
                severity: Severity::Info,
                description: "Pages with title tags over 60 characters".to_string(),
                guidance: "Titles longer than 60 characters may be truncated in search results. Keep titles concise and place the most important keywords near the beginning.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_below_30_chars(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                let has_title = p.title.as_ref().is_some_and(|t| !t.is_empty());
                is_indexable_html(p)
                    && has_title
                    && !is_full_width_title(p)
                    && p.title_length.is_some_and(|len| len < 30)
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{} characters: {}",
                    p.title_length.unwrap_or(0),
                    p.title.as_deref().unwrap_or("")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::PageTitles,
                check: "below_30_chars".to_string(),
                display_name: "Below 30 Characters".to_string(),
                severity: Severity::Info,
                description: "Pages with title tags under 30 characters".to_string(),
                guidance: "Very short titles may not provide enough context for search engines or users. Expand titles to be more descriptive while staying under 60 characters.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_over_561_pixels(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| is_indexable_html(p) && p.title_pixel_width.is_some_and(|w| w > 561))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{}px wide: {}",
                    p.title_pixel_width.unwrap_or(0),
                    p.title.as_deref().unwrap_or("")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::PageTitles,
                check: "over_561_pixels".to_string(),
                display_name: "Over 561px Wide".to_string(),
                severity: Severity::Info,
                description: "Pages with title tags over 561 pixels wide".to_string(),
                guidance: "Google typically displays up to about 561 pixels of a title in desktop search results. Titles wider than this will be truncated with an ellipsis. Shorten the title or front-load important keywords.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_below_200_pixels(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                let has_title = p.title.as_ref().is_some_and(|t| !t.is_empty());
                is_indexable_html(p) && has_title && p.title_pixel_width.is_some_and(|w| w < 200)
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{}px wide: {}",
                    p.title_pixel_width.unwrap_or(0),
                    p.title.as_deref().unwrap_or("")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::PageTitles,
                check: "below_200_pixels".to_string(),
                display_name: "Below 200px Wide".to_string(),
                severity: Severity::Info,
                description: "Pages with title tags under 200 pixels wide".to_string(),
                guidance: "Very narrow titles may not take full advantage of search result display space. Consider expanding the title to be more descriptive and keyword-rich.".to_string(),
                urls: affected,
            });
        }
    }
}
