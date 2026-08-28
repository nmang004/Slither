use std::collections::HashMap;

use crate::analysis::{is_indexable_html, AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

pub struct HeadingsAnalyzer;

impl Analyzer for HeadingsAnalyzer {
    fn name(&self) -> &str {
        "Headings"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Headings
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_h1_missing(ctx, &mut issues);
        self.check_h1_multiple(ctx, &mut issues);
        self.check_h1_duplicate(ctx, &mut issues);
        self.check_h1_same_as_title(ctx, &mut issues);
        self.check_h1_over_70_chars(ctx, &mut issues);
        self.check_h2_missing(ctx, &mut issues);
        self.check_h2_duplicate(ctx, &mut issues);
        self.check_h2_over_70_chars(ctx, &mut issues);
        self.check_non_sequential(ctx, &mut issues);
        issues
    }
}

impl HeadingsAnalyzer {
    fn check_h1_missing(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.is_html_page())
            .filter(|p| p.h1.iter().all(|h| h.trim().is_empty()))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("No H1 heading found".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Headings,
                check: "h1_missing".to_string(),
                display_name: "Missing H1".to_string(),
                severity: Severity::Warning,
                description: "Pages missing an H1 heading".to_string(),
                guidance: "Every page should have exactly one H1 heading that describes the main topic. The H1 helps search engines understand page content and improves accessibility.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_h1_multiple(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| is_indexable_html(p) && p.h1.len() > 1)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{} H1 tags: {}",
                    p.h1.len(),
                    p.h1.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Headings,
                check: "h1_multiple".to_string(),
                display_name: "Multiple H1 Tags".to_string(),
                severity: Severity::Warning,
                description: "Pages with multiple H1 headings".to_string(),
                guidance: "While HTML5 allows multiple H1s, best practice for SEO is to use a single H1 per page. Use H2-H6 for subheadings to create a clear content hierarchy.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_h1_duplicate(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        // Error and noindex pages share boilerplate headings ("Page not found")
        // that are not a duplicate-content defect on the indexable site.
        let mut h1_map: HashMap<&str, Vec<&str>> = HashMap::new();
        for page in ctx.pages.iter().filter(|p| is_indexable_html(p)) {
            if let Some(first_h1) = page.h1.first() {
                if !first_h1.is_empty() {
                    h1_map
                        .entry(first_h1.as_str())
                        .or_default()
                        .push(page.url.as_str());
                }
            }
        }
        let affected: Vec<IssueUrl> = h1_map
            .into_iter()
            .filter(|(_, urls)| urls.len() >= 2)
            .flat_map(|(h1_text, urls)| {
                urls.into_iter().map(move |url| IssueUrl {
                    url: url.to_string(),
                    detail: Some(format!("Duplicate H1: {}", h1_text)),
                })
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Headings,
                check: "h1_duplicate".to_string(),
                display_name: "Duplicate H1 Headings".to_string(),
                severity: Severity::Warning,
                description: "Pages with duplicate H1 headings across the site".to_string(),
                guidance: "Each page should have a unique H1 that differentiates it from other pages. Duplicate H1s make it harder for search engines to understand what makes each page unique.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_h1_same_as_title(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                if let (Some(first_h1), Some(title)) = (p.h1.first(), &p.title) {
                    !first_h1.is_empty() && !title.is_empty() && first_h1 == title
                } else {
                    false
                }
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "H1 and title are identical: {}",
                    p.title.as_deref().unwrap_or("")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Headings,
                check: "h1_same_as_title".to_string(),
                display_name: "H1 Same as Title".to_string(),
                severity: Severity::Info,
                description: "Pages where the H1 is identical to the title tag".to_string(),
                guidance: "While not a critical issue, differentiating your H1 from the title tag provides an opportunity to target additional keywords and provide more context to users on the page.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_h1_over_70_chars(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| is_indexable_html(p) && p.h1.iter().any(|h| h.chars().count() > 70))
            .map(|p| {
                let long_h1s: Vec<String> =
                    p.h1.iter()
                        .filter(|h| h.chars().count() > 70)
                        .map(|h| format!("{} chars", h.chars().count()))
                        .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Long H1(s): {}",
                        crate::analysis::detail_sample(&long_h1s, long_h1s.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Headings,
                check: "h1_over_70_chars".to_string(),
                display_name: "H1 Over 70 Characters".to_string(),
                severity: Severity::Info,
                description: "Pages with H1 headings over 70 characters".to_string(),
                guidance: "Very long H1 headings can dilute keyword focus and may not display well on all devices. Keep H1 headings concise and focused on the main topic.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_h2_missing(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.is_html_page())
            .filter(|p| !p.headings.iter().any(|h| h.level == 2))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("No H2 headings found".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Headings,
                check: "h2_missing".to_string(),
                display_name: "Missing H2".to_string(),
                severity: Severity::Info,
                description: "Pages missing H2 headings".to_string(),
                guidance: "H2 headings help structure content into logical sections. Adding H2s improves readability, accessibility, and helps search engines understand content organization.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_h2_duplicate(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let mut h2_map: HashMap<&str, Vec<&str>> = HashMap::new();
        for page in ctx.pages.iter().filter(|p| is_indexable_html(p)) {
            for heading in &page.headings {
                if heading.level == 2 && !heading.text.is_empty() {
                    h2_map
                        .entry(heading.text.as_str())
                        .or_default()
                        .push(page.url.as_str());
                }
            }
        }
        // Deduplicate: a page might have the same H2 multiple times, but we only
        // want to list the page once per H2 text for cross-page duplicate detection.
        let affected: Vec<IssueUrl> = h2_map
            .into_iter()
            .filter(|(_, urls)| {
                let mut deduped = urls.clone();
                deduped.sort();
                deduped.dedup();
                deduped.len() >= 2
            })
            .flat_map(|(h2_text, urls)| {
                let mut deduped = urls;
                deduped.sort();
                deduped.dedup();
                deduped.into_iter().map(move |url| IssueUrl {
                    url: url.to_string(),
                    detail: Some(format!("Duplicate H2: {}", h2_text)),
                })
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Headings,
                check: "h2_duplicate".to_string(),
                display_name: "Duplicate H2 Headings".to_string(),
                severity: Severity::Info,
                description: "H2 headings duplicated across multiple pages".to_string(),
                guidance: "While less critical than H1 duplicates, unique H2 headings help differentiate page sections across the site and can improve keyword targeting.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_h2_over_70_chars(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                is_indexable_html(p)
                    && p.headings
                        .iter()
                        .any(|h| h.level == 2 && h.text.chars().count() > 70)
            })
            .map(|p| {
                let long_h2s: Vec<String> = p
                    .headings
                    .iter()
                    .filter(|h| h.level == 2 && h.text.chars().count() > 70)
                    .map(|h| format!("{} chars", h.text.chars().count()))
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Long H2(s): {}",
                        crate::analysis::detail_sample(&long_h2s, long_h2s.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Headings,
                check: "h2_over_70_chars".to_string(),
                display_name: "H2 Over 70 Characters".to_string(),
                severity: Severity::Info,
                description: "Pages with H2 headings over 70 characters".to_string(),
                guidance: "Long H2 headings can dilute keyword focus. Keep subheadings concise and descriptive for better readability and SEO.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_non_sequential(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        // `p.headings` is the full heading outline in document order (h1–h6),
        // so the sequence is evaluated exactly as it appears on the page.
        let is_non_sequential = |levels: &[u8]| -> bool {
            if levels.is_empty() {
                return false;
            }
            // A heading that skips more than one level below the previous one.
            for w in levels.windows(2) {
                if w[1] > w[0] + 1 {
                    return true;
                }
            }
            // Outline starts deeper than H1.
            levels[0] > 1
        };

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| is_non_sequential(&p.headings.iter().map(|h| h.level).collect::<Vec<_>>()))
            .map(|p| {
                // Report the trigger, not the transcript. Dumping the whole
                // outline produced 3 KB of "H1 -> H4 -> H1 -> H4 ..." for one
                // page, and on the most common trigger (an outline that starts
                // below H1) the dump contained no level skip at all while the
                // guidance asserted one.
                let levels: Vec<u8> = p.headings.iter().map(|h| h.level).collect();
                let detail = if levels.first().is_some_and(|&l| l > 1) {
                    format!("Outline starts at H{} instead of H1", levels[0])
                } else {
                    let skips: Vec<String> = levels
                        .windows(2)
                        .enumerate()
                        .filter(|(_, w)| w[1] > w[0] + 1)
                        .map(|(i, w)| format!("H{} -> H{} at heading {}", w[0], w[1], i + 2))
                        .collect();
                    crate::analysis::detail_sample(&skips, skips.len())
                };
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(detail),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Headings,
                check: "non_sequential".to_string(),
                display_name: "Non-Sequential Headings".to_string(),
                severity: Severity::Info,
                description: "Pages with non-sequential heading levels".to_string(),
                guidance: "Heading levels should follow a logical hierarchy without skipping levels (e.g., H1 -> H3 without H2). Fix the heading structure for better accessibility and content organization.".to_string(),
                urls: affected,
            });
        }
    }
}
