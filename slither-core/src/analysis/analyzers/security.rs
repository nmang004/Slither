use crate::analysis::{AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

pub struct SecurityAnalyzer;

impl Analyzer for SecurityAnalyzer {
    fn name(&self) -> &str {
        "Security"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Security
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_http_urls(ctx, &mut issues);
        self.check_mixed_content(ctx, &mut issues);
        self.check_insecure_form_action(ctx, &mut issues);
        self.check_form_on_http(ctx, &mut issues);
        self.check_unsafe_cross_origin(ctx, &mut issues);
        self.check_missing_hsts(ctx, &mut issues);
        self.check_missing_csp(ctx, &mut issues);
        self.check_missing_x_content_type_options(ctx, &mut issues);
        self.check_missing_x_frame_options(ctx, &mut issues);
        self.check_missing_referrer_policy(ctx, &mut issues);
        issues
    }
}

impl SecurityAnalyzer {
    fn check_http_urls(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.is_https)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Served over HTTP, not HTTPS".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Security,
                check: "http_urls".to_string(),
                display_name: "HTTP Pages (Not Secure)".to_string(),
                severity: Severity::Warning,
                description: "Pages served over insecure HTTP".to_string(),
                guidance: "All pages should be served over HTTPS. Migrate to HTTPS and set up 301 redirects from HTTP to HTTPS to protect user data and improve search rankings.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_mixed_content(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.mixed_content.is_empty())
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{} mixed content resource(s): {}",
                    p.mixed_content.len(),
                    p.mixed_content
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Security,
                check: "mixed_content".to_string(),
                display_name: "Mixed Content".to_string(),
                severity: Severity::Warning,
                description: "HTTPS pages loading resources over HTTP".to_string(),
                guidance: "Mixed content weakens HTTPS security. Update all resource URLs (images, scripts, stylesheets) to use HTTPS.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_insecure_form_action(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.insecure_forms.is_empty())
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{} form(s) with insecure action: {}",
                    p.insecure_forms.len(),
                    p.insecure_forms
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Security,
                check: "insecure_form_action".to_string(),
                display_name: "Insecure Form Actions".to_string(),
                severity: Severity::Warning,
                description: "Forms submitting data to insecure HTTP endpoints".to_string(),
                guidance: "Form action URLs should use HTTPS to protect submitted data. Update form action attributes to use HTTPS URLs.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_form_on_http(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.is_https && !p.insecure_forms.is_empty())
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("HTTP page contains forms".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Security,
                check: "form_on_http".to_string(),
                display_name: "Forms on HTTP Pages".to_string(),
                severity: Severity::Warning,
                description: "Forms present on pages served over HTTP".to_string(),
                guidance: "Forms on HTTP pages expose user input to interception. Migrate these pages to HTTPS to protect form submissions.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_unsafe_cross_origin(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.unsafe_cross_origin_links.is_empty())
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{} link(s) with target=\"_blank\" missing rel=\"noopener\": {}",
                    p.unsafe_cross_origin_links.len(),
                    p.unsafe_cross_origin_links
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Security,
                check: "unsafe_cross_origin".to_string(),
                display_name: "Unsafe Cross-Origin Links".to_string(),
                severity: Severity::Info,
                description: "Links with target=\"_blank\" missing rel=\"noopener\"".to_string(),
                guidance: "Links that open in a new tab without rel=\"noopener\" can allow the opened page to access the originating page via window.opener. Add rel=\"noopener\" to external links with target=\"_blank\".".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing_hsts(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.is_https && !p.security_headers.has_hsts)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Missing Strict-Transport-Security header".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Security,
                check: "missing_hsts".to_string(),
                display_name: "Missing HSTS Header".to_string(),
                severity: Severity::Info,
                description: "HTTPS pages missing the Strict-Transport-Security header".to_string(),
                guidance: "HSTS tells browsers to always use HTTPS for this domain. Add a Strict-Transport-Security header with an appropriate max-age value (e.g., max-age=31536000).".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing_csp(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.security_headers.has_csp)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Missing Content-Security-Policy header".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Security,
                check: "missing_csp".to_string(),
                display_name: "Missing CSP Header".to_string(),
                severity: Severity::Info,
                description: "Pages missing the Content-Security-Policy header".to_string(),
                guidance: "A Content-Security-Policy header helps prevent XSS attacks and data injection by specifying which content sources are allowed. Configure a CSP policy appropriate for your site.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing_x_content_type_options(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.security_headers.has_x_content_type_options)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Missing X-Content-Type-Options header".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Security,
                check: "missing_x_content_type_options".to_string(),
                display_name: "Missing X-Content-Type-Options".to_string(),
                severity: Severity::Info,
                description: "Pages missing the X-Content-Type-Options header".to_string(),
                guidance: "Add the header X-Content-Type-Options: nosniff to prevent browsers from MIME-sniffing the content type, which can prevent certain attacks.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing_x_frame_options(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| !p.security_headers.has_x_frame_options)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("Missing X-Frame-Options header".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Security,
                check: "missing_x_frame_options".to_string(),
                display_name: "Missing X-Frame-Options".to_string(),
                severity: Severity::Info,
                description: "Pages missing the X-Frame-Options header".to_string(),
                guidance: "Add X-Frame-Options: DENY or SAMEORIGIN to prevent your pages from being embedded in iframes on other sites, protecting against clickjacking attacks.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_missing_referrer_policy(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                if !p.security_headers.has_referrer_policy {
                    return true;
                }
                if let Some(ref val) = p.security_headers.referrer_policy_value {
                    let lower = val.to_lowercase();
                    lower == "unsafe-url" || lower == "no-referrer-when-downgrade"
                } else {
                    false
                }
            })
            .map(|p| {
                let detail = if !p.security_headers.has_referrer_policy {
                    "Missing Referrer-Policy header".to_string()
                } else {
                    format!(
                        "Unsafe Referrer-Policy value: {}",
                        p.security_headers
                            .referrer_policy_value
                            .as_deref()
                            .unwrap_or("unknown")
                    )
                };
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(detail),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Security,
                check: "missing_referrer_policy".to_string(),
                display_name: "Missing or Unsafe Referrer-Policy".to_string(),
                severity: Severity::Info,
                description: "Pages with a missing or unsafe Referrer-Policy".to_string(),
                guidance: "Set a Referrer-Policy header such as strict-origin-when-cross-origin or no-referrer to control how much referrer information is shared with external sites.".to_string(),
                urls: affected,
            });
        }
    }
}
