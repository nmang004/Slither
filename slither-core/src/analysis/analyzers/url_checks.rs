use crate::analysis::{AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

/// True if the URL carries a `utm_*` *query parameter*. Matching the raw URL
/// as a substring also flagged paths like `/guide-to-utm_parameters`.
fn has_utm_query_param(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(parsed) => parsed
            .query_pairs()
            .any(|(key, _)| key.to_ascii_lowercase().starts_with("utm_")),
        Err(_) => false,
    }
}

pub struct UrlAnalyzer;

impl Analyzer for UrlAnalyzer {
    fn name(&self) -> &str {
        "URL"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Url
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_contains_space(ctx, &mut issues);
        self.check_over_115_chars(ctx, &mut issues);
        self.check_contains_uppercase(ctx, &mut issues);
        self.check_contains_underscores(ctx, &mut issues);
        self.check_has_parameters(ctx, &mut issues);
        self.check_multiple_slashes(ctx, &mut issues);
        self.check_non_ascii(ctx, &mut issues);
        self.check_repetitive_path(ctx, &mut issues);
        self.check_ga_tracking_params(ctx, &mut issues);
        issues
    }
}

impl UrlAnalyzer {
    fn check_contains_space(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.url.contains("%20") || p.url.contains(' '))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("URL contains spaces".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Url,
                check: "contains_space".to_string(),
                display_name: "URLs with Spaces".to_string(),
                severity: Severity::Warning,
                description: "URLs containing spaces".to_string(),
                guidance: "Spaces in URLs are encoded as %20 and can cause issues with linking and sharing. Replace spaces with hyphens for cleaner, more readable URLs.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_over_115_chars(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.url_length > 115)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("{} characters", p.url_length)),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Url,
                check: "over_115_chars".to_string(),
                display_name: "URLs Over 115 Characters".to_string(),
                severity: Severity::Info,
                description: "URLs over 115 characters".to_string(),
                guidance: "Very long URLs can be truncated in search results and are harder for users to share. Shorten URL paths where possible by using concise, descriptive slugs.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_contains_uppercase(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.has_uppercase)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("URL contains uppercase characters".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Url,
                check: "contains_uppercase".to_string(),
                display_name: "URLs with Uppercase".to_string(),
                severity: Severity::Info,
                description: "URLs containing uppercase characters".to_string(),
                guidance: "URLs are case-sensitive on most servers, which can lead to duplicate content. Use lowercase URLs consistently and redirect uppercase variants to lowercase.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_contains_underscores(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.has_underscores)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("URL contains underscores".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Url,
                check: "contains_underscores".to_string(),
                display_name: "URLs with Underscores".to_string(),
                severity: Severity::Info,
                description: "URLs containing underscores".to_string(),
                guidance: "Search engines treat hyphens as word separators but not underscores. Use hyphens instead of underscores in URL paths for better keyword recognition.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_has_parameters(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.has_parameters)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("URL contains query parameters".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Url,
                check: "has_parameters".to_string(),
                display_name: "URLs with Parameters".to_string(),
                severity: Severity::Info,
                description: "URLs containing query parameters".to_string(),
                guidance: "Query parameters can create duplicate content issues and make URLs less readable. Where possible, use clean URL paths and handle parameter-based URLs with canonical tags.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_multiple_slashes(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.has_multiple_slashes)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("URL path contains consecutive slashes".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Url,
                check: "multiple_slashes".to_string(),
                display_name: "Consecutive Slashes".to_string(),
                severity: Severity::Info,
                description: "URLs with consecutive slashes in the path".to_string(),
                guidance: "Multiple consecutive slashes can create duplicate content issues. Normalize URLs to use single slashes and set up redirects for the doubled versions.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_non_ascii(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.has_non_ascii)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("URL contains non-ASCII characters".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Url,
                check: "non_ascii".to_string(),
                display_name: "Non-ASCII Characters".to_string(),
                severity: Severity::Info,
                description: "URLs containing non-ASCII characters".to_string(),
                guidance: "Non-ASCII characters in URLs get percent-encoded, making them harder to read and share. Use ASCII-only characters in URL paths where possible.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_repetitive_path(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.has_repetitive_path)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("URL has repetitive path segments".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Url,
                check: "repetitive_path".to_string(),
                display_name: "Repetitive Path Segments".to_string(),
                severity: Severity::Info,
                description: "URLs with repetitive path segments".to_string(),
                guidance: "Repetitive path segments (e.g., /blog/blog/) suggest misconfiguration in URL routing. Clean up the URL structure to remove redundant segments.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_ga_tracking_params(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            // Inspect query parameter *names* only: a substring match also hit
            // ordinary paths such as /guide-to-utm_parameters.
            .filter(|p| has_utm_query_param(&p.url))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("URL contains UTM tracking parameters".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Url,
                check: "ga_tracking_params".to_string(),
                display_name: "GA Tracking Parameters".to_string(),
                severity: Severity::Info,
                description: "URLs containing Google Analytics tracking parameters".to_string(),
                guidance: "UTM parameters in crawled URLs can cause duplicate content. Ensure internal links do not include UTM parameters and use canonical tags to consolidate tracked variants.".to_string(),
                urls: affected,
            });
        }
    }
}
