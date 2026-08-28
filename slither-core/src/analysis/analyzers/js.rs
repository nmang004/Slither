use crate::analysis::{AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

const EXCESSIVE_SCRIPT_COUNT: usize = 20;
const EXCESSIVE_INLINE_BYTES: u32 = 100 * 1024;
const THIRD_PARTY_THRESHOLD: usize = 10;
const CRITICAL_CONSOLE_ERROR_THRESHOLD: usize = 5;

pub struct JsAnalyzer;

impl Analyzer for JsAnalyzer {
    fn name(&self) -> &str {
        "JavaScript"
    }

    fn category(&self) -> IssueCategory {
        IssueCategory::JavaScript
    }

    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();

        // JS-dependent SEO elements (5 checks)
        self.check_injected_title(ctx, &mut issues);
        self.check_injected_description(ctx, &mut issues);
        self.check_injected_canonical(ctx, &mut issues);
        self.check_injected_h1(ctx, &mut issues);
        self.check_injected_structured_data(ctx, &mut issues);

        // JS resource analysis (3 checks)
        self.check_render_blocking(ctx, &mut issues);
        self.check_excessive_scripts(ctx, &mut issues);
        self.check_heavy_third_party(ctx, &mut issues);

        // Console errors (2 checks)
        self.check_console_errors(ctx, &mut issues);
        self.check_critical_console_errors(ctx, &mut issues);

        issues
    }
}

impl JsAnalyzer {
    fn check_injected_title(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.js_injected_title)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Title absent in static HTML, present after JS render".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::JavaScript,
                check: "js_injected_title".to_string(),
                display_name: "Title injected by JavaScript".to_string(),
                severity: Severity::Warning,
                description: "Pages where the <title> tag is injected by JavaScript and missing in the static HTML".to_string(),
                guidance: "Search engine crawlers that do not execute JavaScript will see no title. Render the title server-side or use static HTML so it is available in the initial response.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_injected_description(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.js_injected_description)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(
                    "Meta description absent in static HTML, present after JS render".to_string(),
                ),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::JavaScript,
                check: "js_injected_description".to_string(),
                display_name: "Meta description injected by JavaScript".to_string(),
                severity: Severity::Warning,
                description: "Pages where the meta description is injected by JavaScript and missing in the static HTML".to_string(),
                guidance: "Meta descriptions drive SERP snippet quality. Render the meta description in the initial HTML response to ensure crawlers without JavaScript see it.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_injected_canonical(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.js_injected_canonical)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(
                    "Canonical tag absent in static HTML, present after JS render".to_string(),
                ),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::JavaScript,
                check: "js_injected_canonical".to_string(),
                display_name: "Canonical injected by JavaScript".to_string(),
                severity: Severity::Critical,
                description: "Pages where the canonical link is injected by JavaScript and missing in the static HTML".to_string(),
                guidance: "Google may disregard canonical tags that only appear after JavaScript execution, leading to duplicate-content and indexing confusion. Emit canonical tags server-side in the initial HTML.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_injected_h1(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.js_injected_h1)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("H1 absent in static HTML, present after JS render".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::JavaScript,
                check: "js_injected_h1".to_string(),
                display_name: "H1 injected by JavaScript".to_string(),
                severity: Severity::Warning,
                description: "Pages where the primary H1 heading is injected by JavaScript and missing in the static HTML".to_string(),
                guidance: "Crawlers without JavaScript will not see the H1. Render primary headings server-side to guarantee they appear in the initial response.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_injected_structured_data(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.js_injected_structured_data)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(
                    "Structured data absent in static HTML, present after JS render".to_string(),
                ),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::JavaScript,
                check: "js_injected_structured_data".to_string(),
                display_name: "Structured data injected by JavaScript".to_string(),
                severity: Severity::Warning,
                description: "Pages where schema.org structured data is injected by JavaScript and missing in the static HTML".to_string(),
                guidance: "Some crawlers do not wait for JavaScript when discovering structured data. Emit JSON-LD blocks in the initial HTML to maximise rich-result eligibility.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_render_blocking(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                // Only an external script in <head> without async/defer/module
                // truly blocks first paint. async/defer are ignored on inline
                // scripts, so counting those produced false positives on nearly
                // every page (analytics/dataLayer snippets).
                let blocking = p
                    .scripts
                    .iter()
                    .filter(|s| {
                        s.src.is_some() && s.in_head && !s.is_async && !s.is_defer && !s.is_module
                    })
                    .count();
                if blocking == 0 {
                    return None;
                }
                Some(IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!("{} render-blocking script(s)", blocking)),
                })
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::JavaScript,
                check: "js_render_blocking".to_string(),
                display_name: "Render-blocking scripts".to_string(),
                severity: Severity::Warning,
                description: "Pages with <script> tags that block HTML parsing (no async, defer, or type=\"module\")".to_string(),
                guidance: "Render-blocking scripts delay First Contentful Paint. Add the async or defer attribute, or convert the tag to type=\"module\" so the browser can continue parsing HTML while the script downloads.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_excessive_scripts(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let count = p.scripts.len();
                let inline_bytes: u32 = p.scripts.iter().map(|s| s.size_bytes).sum();
                if count > EXCESSIVE_SCRIPT_COUNT || inline_bytes > EXCESSIVE_INLINE_BYTES {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!(
                            "{} script(s), {} KB inline",
                            count,
                            inline_bytes / 1024
                        )),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::JavaScript,
                check: "js_excessive_scripts".to_string(),
                display_name: "Excessive JavaScript".to_string(),
                severity: Severity::Warning,
                description: "Pages with more than 20 script tags or more than 100 KB of inline JavaScript".to_string(),
                guidance: "Large amounts of JavaScript slow down parsing, execution, and Time to Interactive. Audit third-party tags, consolidate bundles, code-split by route, and lazy-load non-critical scripts.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_heavy_third_party(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let third_party = p.scripts.iter().filter(|s| s.is_third_party).count();
                if third_party > THIRD_PARTY_THRESHOLD {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("{} third-party script(s)", third_party)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::JavaScript,
                check: "js_third_party_heavy".to_string(),
                display_name: "Heavy third-party scripts".to_string(),
                severity: Severity::Warning,
                description: "Pages loading more than 10 third-party scripts".to_string(),
                guidance: "Third-party scripts (analytics, ads, widgets, tag managers) often degrade performance and privacy. Audit each vendor, remove unused tags, and load non-critical ones via a tag manager with consent-aware deferral.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_console_errors(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            // Pages above the threshold are reported by the Critical check;
            // listing them here too deducted twice for one condition.
            .filter(|p| {
                !p.console_errors.is_empty()
                    && p.console_errors.len() <= CRITICAL_CONSOLE_ERROR_THRESHOLD
            })
            .map(|p| {
                let sample: String = p
                    .console_errors
                    .iter()
                    .take(2)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" | ");
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "{} console error(s): {}",
                        p.console_errors.len(),
                        sample
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::JavaScript,
                check: "js_console_errors".to_string(),
                display_name: "JavaScript console errors".to_string(),
                severity: Severity::Warning,
                description: "Pages that threw uncaught JavaScript exceptions during rendering".to_string(),
                guidance: "Console errors may indicate broken features or failed third-party integrations. Open DevTools, reproduce the error, and fix the underlying exception or remove the offending script.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_critical_console_errors(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.console_errors.len() > CRITICAL_CONSOLE_ERROR_THRESHOLD)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("{} console error(s)", p.console_errors.len())),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::JavaScript,
                check: "js_console_errors_critical".to_string(),
                display_name: "Critical JavaScript errors".to_string(),
                severity: Severity::Critical,
                description: "Pages with more than 5 uncaught JavaScript exceptions during rendering".to_string(),
                guidance: "A high volume of JS errors typically indicates a broken page or misconfigured third-party script. Investigate the stack traces in browser DevTools and fix the root cause.".to_string(),
                urls: affected,
            });
        }
    }
}
