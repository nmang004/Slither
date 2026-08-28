use crate::analysis::{AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

// The "Competing Primary Schema Types" check was removed: it asserted a rule
// Google does not have.
//
// Google's structured-data policies document multiple items on one page as
// supported — "a page could contain a recipe, a video that shows how to make
// that recipe, and breadcrumb information" — and name *both* nesting and
// separate individual blocks on the same page as valid implementations. The
// check therefore fired on standard, correct markup: NewsArticle + VideoObject,
// the Yoast + WP Recipe Maker stack (BlogPosting + Recipe), Product + FAQPage,
// and a JSON-LD block mirrored in microdata (one entity in two supported
// formats, not two competing entities).
//
// Structured data governs rich-result eligibility, not ranking, and Google's
// only stated caveat is that the markup should reflect the page's main content —
// which co-occurrence does not violate.

pub struct StructuredDataAnalyzer;

impl Analyzer for StructuredDataAnalyzer {
    fn name(&self) -> &str {
        "Structured Data"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::StructuredData
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_parse_errors(ctx, &mut issues);
        self.check_missing_required(ctx, &mut issues);
        self.check_missing(ctx, &mut issues);
        issues
    }
}

impl StructuredDataAnalyzer {
    fn check_parse_errors(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.structured_data.iter().any(|sd| sd.parse_error.is_some()))
            .map(|p| {
                let errors: Vec<String> = p
                    .structured_data
                    .iter()
                    .filter_map(|sd| sd.parse_error.clone())
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!("Parse errors: {}", errors.join("; "))),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::StructuredData,
                check: "parse_errors".to_string(),
                display_name: "Schema Parse Errors".to_string(),
                severity: Severity::Warning,
                description: "Pages with structured data parse errors".to_string(),
                guidance: "Structured data with parse errors will be ignored by search engines. Fix the JSON-LD or microdata syntax errors to ensure rich results can be generated.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing_required(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                p.structured_data
                    .iter()
                    .any(|sd| !sd.missing_required.is_empty())
            })
            .map(|p| {
                let missing: Vec<String> = p
                    .structured_data
                    .iter()
                    .filter(|sd| !sd.missing_required.is_empty())
                    .map(|sd| {
                        format!(
                            "{}: [{}]",
                            sd.schema_type.as_deref().unwrap_or("unknown"),
                            crate::analysis::detail_sample(
                                &sd.missing_required,
                                sd.missing_required.len()
                            )
                        )
                    })
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!("Missing required fields: {}", missing.join("; "))),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::StructuredData,
                check: "missing_required".to_string(),
                display_name: "Missing Required Fields".to_string(),
                severity: Severity::Warning,
                description: "Structured data missing required fields".to_string(),
                guidance: "Missing required fields prevent structured data from qualifying for rich results. Add the required properties to satisfy schema.org requirements.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.is_html_page() && p.structured_data.is_empty())
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: None,
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::StructuredData,
                check: "missing".to_string(),
                display_name: "No Structured Data".to_string(),
                severity: Severity::Info,
                description: "Pages without any structured data".to_string(),
                guidance: "Structured data helps search engines understand page content and can enable rich results. Consider adding JSON-LD structured data appropriate to your content type.".to_string(),
                urls: affected,
            });
        }
    }
}
