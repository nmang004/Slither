use crate::analysis::{AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

pub struct ImagesAnalyzer;

impl Analyzer for ImagesAnalyzer {
    fn name(&self) -> &str {
        "Images"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Images
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_missing_alt_attribute(ctx, &mut issues);
        self.check_alt_over_100_chars(ctx, &mut issues);
        self.check_missing_dimensions(ctx, &mut issues);
        issues
    }
}

impl ImagesAnalyzer {
    fn check_missing_alt_attribute(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| (200..300).contains(&p.status))
            .filter(|p| {
                p.images
                    .iter()
                    .any(|img| img.alt.is_none() && img.needs_alt_text())
            })
            .map(|p| {
                let missing: Vec<String> = p
                    .images
                    .iter()
                    .filter(|img| img.alt.is_none() && img.needs_alt_text())
                    .map(|img| img.src.clone())
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "{} image(s) missing alt: {}",
                        missing.len(),
                        crate::analysis::detail_sample(&missing, missing.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Images,
                check: "missing_alt_attribute".to_string(),
                display_name: "Missing Alt Text".to_string(),
                severity: Severity::Warning,
                description: "Images missing the alt attribute".to_string(),
                guidance: "Every image should have an alt attribute to describe its content for screen readers and search engines. Add descriptive alt text to all images.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_alt_over_100_chars(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| (200..300).contains(&p.status))
            .filter(|p| {
                p.images
                    .iter()
                    .any(|img| img.alt.as_ref().is_some_and(|a| a.chars().count() > 100))
            })
            .map(|p| {
                let long: Vec<String> = p
                    .images
                    .iter()
                    .filter(|img| img.alt.as_ref().is_some_and(|a| a.chars().count() > 100))
                    .map(|img| {
                        format!(
                            "{} ({} chars)",
                            img.src,
                            img.alt.as_ref().map(|a| a.chars().count()).unwrap_or(0)
                        )
                    })
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "{} image(s): {}",
                        long.len(),
                        crate::analysis::detail_sample(&long, long.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Images,
                check: "alt_over_100_chars".to_string(),
                display_name: "Alt Text Over 100 Characters".to_string(),
                severity: Severity::Info,
                description: "Images with alt text over 100 characters".to_string(),
                guidance: "Keep alt text concise and descriptive. Alt text over 100 characters may be truncated by screen readers and can appear spammy to search engines.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing_dimensions(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| (200..300).contains(&p.status))
            .filter(|p| {
                p.images
                    .iter()
                    .any(|img| img.width.is_none() || img.height.is_none())
            })
            .map(|p| {
                let missing: Vec<String> = p
                    .images
                    .iter()
                    .filter(|img| img.width.is_none() || img.height.is_none())
                    .map(|img| img.src.clone())
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "{} image(s) missing dimensions: {}",
                        missing.len(),
                        crate::analysis::detail_sample(&missing, missing.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Images,
                check: "missing_dimensions".to_string(),
                display_name: "Missing Image Dimensions".to_string(),
                severity: Severity::Info,
                description: "Images missing width or height attributes".to_string(),
                guidance: "Specifying image dimensions helps browsers allocate space before images load, preventing layout shifts (CLS). Add width and height attributes to all images.".to_string(),
                urls: affected,
            });
        }
    }
}
