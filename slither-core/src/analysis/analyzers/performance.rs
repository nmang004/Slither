use crate::analysis::{AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};
use crate::pagespeed::models::{CwvStatus, CwvThresholds};

pub struct PerformanceAnalyzer;

impl Analyzer for PerformanceAnalyzer {
    fn name(&self) -> &str {
        "Performance"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Performance
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_lcp_poor(ctx, &mut issues);
        self.check_lcp_needs_work(ctx, &mut issues);
        self.check_inp_poor(ctx, &mut issues);
        self.check_inp_needs_work(ctx, &mut issues);
        self.check_cls_poor(ctx, &mut issues);
        self.check_cls_needs_work(ctx, &mut issues);
        self.check_fcp_slow(ctx, &mut issues);
        self.check_ttfb_slow(ctx, &mut issues);
        self.check_score_poor(ctx, &mut issues);
        self.check_score_needs_work(ctx, &mut issues);
        // Always-available server-latency check (no --pagespeed needed).
        self.check_slow_server_response(ctx, &mut issues);
        issues
    }
}

impl PerformanceAnalyzer {
    /// Flag HTML pages whose server response (TTFB proxy from the crawl fetch)
    /// is slow. This is measured for every crawled page, so the Performance
    /// category produces signal even without `--pagespeed`.
    fn check_slow_server_response(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        const SLOW_MS: u64 = 1500;
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.is_html_page())
            .filter(|p| p.response_time_ms >= SLOW_MS)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("Response time: {}ms", p.response_time_ms)),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "slow_server_response".to_string(),
                display_name: "Slow Server Response".to_string(),
                severity: Severity::Warning,
                description: format!("Pages that took over {SLOW_MS}ms to respond"),
                guidance: "A slow server response (high TTFB) delays every downstream metric and hurts crawl efficiency. Investigate backend/database latency, enable caching, and use a CDN.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_lcp_poor(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let lcp = p.lcp_ms?;
                if CwvThresholds::lcp_status(lcp) == CwvStatus::Poor {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("LCP: {:.0}ms", lcp)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "cwv_lcp_poor".to_string(),
                display_name: "LCP exceeds 4.0s (Poor)".to_string(),
                severity: Severity::Critical,
                description: "Pages with Largest Contentful Paint exceeding 4.0 seconds, rated Poor by Core Web Vitals".to_string(),
                guidance: "Optimize the largest visible element on the page. Common fixes include optimizing images, preloading critical resources, reducing server response time, and removing render-blocking resources.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_lcp_needs_work(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let lcp = p.lcp_ms?;
                if CwvThresholds::lcp_status(lcp) == CwvStatus::NeedsImprovement {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("LCP: {:.0}ms", lcp)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "cwv_lcp_needs_work".to_string(),
                display_name: "LCP exceeds 2.5s".to_string(),
                severity: Severity::Warning,
                description: "Pages with Largest Contentful Paint between 2.5s and 4.0s, rated Needs Improvement".to_string(),
                guidance: "LCP is close to the Poor threshold. Optimize images, use preload for critical resources, and reduce time to first byte to bring LCP under 2.5 seconds.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_inp_poor(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let inp = p.inp_ms?;
                if CwvThresholds::inp_status(inp) == CwvStatus::Poor {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("INP: {:.0}ms", inp)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "cwv_inp_poor".to_string(),
                display_name: "INP exceeds 500ms (Poor)".to_string(),
                severity: Severity::Critical,
                description: "Pages with Interaction to Next Paint exceeding 500ms, rated Poor by Core Web Vitals".to_string(),
                guidance: "Reduce input delay by breaking up long tasks, minimizing JavaScript execution time, and avoiding large DOM sizes. Ensure event handlers complete quickly.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_inp_needs_work(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let inp = p.inp_ms?;
                if CwvThresholds::inp_status(inp) == CwvStatus::NeedsImprovement {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("INP: {:.0}ms", inp)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "cwv_inp_needs_work".to_string(),
                display_name: "INP exceeds 200ms".to_string(),
                severity: Severity::Warning,
                description: "Pages with Interaction to Next Paint between 200ms and 500ms, rated Needs Improvement".to_string(),
                guidance: "Optimize event handlers and reduce main thread blocking to bring INP under 200ms. Consider code splitting and deferring non-critical JavaScript.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_cls_poor(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let cls = p.cls?;
                if CwvThresholds::cls_status(cls) == CwvStatus::Poor {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("CLS: {:.3}", cls)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "cwv_cls_poor".to_string(),
                display_name: "CLS exceeds 0.25 (Poor)".to_string(),
                severity: Severity::Critical,
                description: "Pages with Cumulative Layout Shift exceeding 0.25, rated Poor by Core Web Vitals".to_string(),
                guidance: "Reduce layout shifts by setting explicit dimensions on images and videos, avoiding dynamically injected content above the fold, and using CSS contain where appropriate.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_cls_needs_work(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let cls = p.cls?;
                if CwvThresholds::cls_status(cls) == CwvStatus::NeedsImprovement {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("CLS: {:.3}", cls)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "cwv_cls_needs_work".to_string(),
                display_name: "CLS exceeds 0.1".to_string(),
                severity: Severity::Warning,
                description: "Pages with Cumulative Layout Shift between 0.1 and 0.25, rated Needs Improvement".to_string(),
                guidance: "Set explicit width and height on images and embeds, use font-display: swap for web fonts, and avoid inserting content above existing content to reduce layout shifts.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_fcp_slow(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let fcp = p.fcp_ms?;
                if fcp > 3000.0 {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("FCP: {:.0}ms", fcp)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "cwv_fcp_slow".to_string(),
                display_name: "FCP exceeds 3.0s".to_string(),
                severity: Severity::Warning,
                description: "Pages with First Contentful Paint exceeding 3.0 seconds".to_string(),
                guidance: "Reduce FCP by eliminating render-blocking resources, minifying CSS, deferring non-critical CSS, and improving server response times.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_ttfb_slow(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let ttfb = p.ttfb_ms?;
                if ttfb > 800.0 {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("TTFB: {:.0}ms", ttfb)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "cwv_ttfb_slow".to_string(),
                display_name: "TTFB exceeds 800ms".to_string(),
                severity: Severity::Warning,
                description: "Pages with Time to First Byte exceeding 800ms".to_string(),
                guidance: "Reduce TTFB by optimizing server-side processing, using a CDN, enabling caching, and upgrading hosting infrastructure.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_score_poor(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let score = p.performance_score?;
                if score < 50 {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("Performance score: {}", score)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "cwv_score_poor".to_string(),
                display_name: "Performance score below 50".to_string(),
                severity: Severity::Critical,
                description: "Pages with a PageSpeed Insights performance score below 50".to_string(),
                guidance: "A score below 50 indicates serious performance problems. Review the PageSpeed Insights report for specific opportunities and address the highest-impact suggestions first.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_score_needs_work(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter_map(|p| {
                let score = p.performance_score?;
                if (50..90).contains(&score) {
                    Some(IssueUrl {
                        url: p.url.clone(),
                        detail: Some(format!("Performance score: {}", score)),
                    })
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Performance,
                check: "cwv_score_needs_work".to_string(),
                display_name: "Performance score below 90".to_string(),
                severity: Severity::Warning,
                description: "Pages with a PageSpeed Insights performance score between 50 and 89".to_string(),
                guidance: "Aim for a score of 90 or above. Review the PageSpeed Insights diagnostics and opportunities sections for actionable improvements.".to_string(),
                urls: affected,
            });
        }
    }
}
