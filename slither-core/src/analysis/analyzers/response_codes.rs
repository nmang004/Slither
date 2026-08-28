use crate::analysis::{AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};
use crate::models::page::PageData;

/// The URL that was originally requested for this page.
///
/// A page is recorded at the address that finally served it, so for anything
/// reached through a redirect `page.url` is the *destination* and the requested
/// address is the head of the chain. Findings about the request — "this URL
/// redirects" — must be filed against the head; findings about the response —
/// "this URL is a 404" — stay on `page.url`.
fn requested_url(page: &PageData) -> &str {
    page.redirect_chain
        .as_ref()
        .and_then(|chain| chain.first())
        .map_or(page.url.as_str(), |hop| hop.url.as_str())
}

/// Byte budget for the URLs interpolated into an issue detail.
///
/// Details are UI text rendered verbatim into the HTML report and the MCP
/// payload. A 2,000-character URL blows the budget on its own — which is how a
/// 501-page crawl once produced a 17 MB report — so every detail in the
/// codebase caps what it embeds.
const MAX_PATH_BYTES: usize = 300;

/// One URL, capped for use inside an issue detail.
fn bounded_url(url: &str) -> String {
    if url.len() > MAX_PATH_BYTES {
        format!(
            "{}\u{2026}",
            url.chars().take(MAX_PATH_BYTES).collect::<String>()
        )
    } else {
        url.to_string()
    }
}

/// Render a redirect path as `a -> b -> c`, bounded in bytes.
fn hop_path(hops: &[&str]) -> String {
    let mut out = String::new();
    for (i, hop) in hops.iter().enumerate() {
        if !out.is_empty() && out.len() + hop.len() + 4 > MAX_PATH_BYTES {
            out.push_str(&format!(" -> (+{} more)", hops.len() - i));
            break;
        }
        if !out.is_empty() {
            out.push_str(" -> ");
        }
        // A single oversized URL is truncated rather than dropped, so the path
        // is never empty — the same rule `detail_sample` follows.
        out.push_str(&bounded_url(hop));
    }
    out
}

/// Suffix naming the URL a response was reached through, when it redirected.
///
/// A 404 found only behind a 301 is fixed by repointing the link at the
/// *redirecting* address, which appears nowhere in the record otherwise.
fn via_redirect(page: &PageData) -> String {
    match page.redirect_chain.as_ref().and_then(|chain| chain.first()) {
        Some(hop) => format!(" — reached via redirect from {}", bounded_url(&hop.url)),
        None => String::new(),
    }
}

pub struct ResponseCodesAnalyzer;

impl Analyzer for ResponseCodesAnalyzer {
    fn name(&self) -> &str {
        "Response Codes"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::ResponseCodes
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        self.check_internal_server_error(ctx, &mut issues);
        self.check_internal_client_error(ctx, &mut issues);
        self.check_internal_redirect_loop(ctx, &mut issues);
        self.check_internal_no_response(ctx, &mut issues);
        self.check_internal_redirect_chain(ctx, &mut issues);
        self.check_internal_redirect(ctx, &mut issues);
        issues
    }
}

impl ResponseCodesAnalyzer {
    fn check_internal_server_error(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| (500..600).contains(&p.status))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("Status {}{}", p.status, via_redirect(p))),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::ResponseCodes,
                check: "internal_server_error".to_string(),
                display_name: "5xx Server Errors".to_string(),
                severity: Severity::Critical,
                description: "Pages returning 5xx server errors".to_string(),
                guidance: "Server errors indicate the web server is failing to process requests. Investigate the server logs and fix the underlying issues causing these errors.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_internal_client_error(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        // 401/403 are usually deliberate access control and 429 is often the
        // crawler's own rate limiting, so "restore the missing content" is the
        // wrong advice and Critical is the wrong severity. Split them out.
        const ACCESS_CONTROLLED: [u16; 3] = crate::analysis::ACCESS_CONTROLLED;

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| (400..500).contains(&p.status) && !ACCESS_CONTROLLED.contains(&p.status))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("Status {}{}", p.status, via_redirect(p))),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::ResponseCodes,
                check: "internal_client_error".to_string(),
                display_name: "4xx Client Errors".to_string(),
                severity: Severity::Critical,
                description: "Pages returning 4xx client errors".to_string(),
                guidance: "Client errors such as 404 Not Found indicate broken or missing pages. Remove or update any internal links pointing to these URLs, or restore the missing content.".to_string(),
                urls: affected,
            });
        }

        let restricted: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| ACCESS_CONTROLLED.contains(&p.status))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!(
                    "{}{}",
                    match p.status {
                        401 => "Status 401 — authentication required",
                        403 => "Status 403 — access forbidden",
                        _ => "Status 429 — rate limited",
                    },
                    via_redirect(p)
                )),
            })
            .collect();
        if !restricted.is_empty() {
            issues.push(Issue {
                category: IssueCategory::ResponseCodes,
                check: "internal_access_restricted".to_string(),
                display_name: "Access-Restricted or Rate-Limited".to_string(),
                severity: Severity::Warning,
                description: "URLs returning 401, 403, or 429".to_string(),
                guidance: "These responses are often intentional: 401/403 protect private areas, and 429 usually means this crawl was rate limited (try a lower --concurrency or a higher --delay). Confirm that no page you want indexed is behind one of these, and that they are not linked from public pages.".to_string(),
                urls: restricted,
            });
        }
    }

    fn check_internal_redirect_loop(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                // A genuine loop is a URL that appears more than once in the
                // chain. The chain's first hop is always the page's own URL, so
                // the previous `any(hop.url == p.url)` flagged every ordinary
                // redirect (e.g. /x -> /x/) as a loop.
                if let Some(chain) = &p.redirect_chain {
                    let mut seen = std::collections::HashSet::new();
                    chain.iter().any(|hop| !seen.insert(hop.url.as_str()))
                } else {
                    false
                }
            })
            .map(|p| IssueUrl {
                // The loop is a property of the URL that was requested, not of
                // whichever hop the crawler gave up on.
                url: requested_url(p).to_string(),
                detail: Some(format!(
                    "Redirect chain loops back to the same URL: {}",
                    hop_path(
                        &p.redirect_chain
                            .as_ref()
                            .map(|chain| chain.iter().map(|h| h.url.as_str()).collect::<Vec<_>>())
                            .unwrap_or_default()
                    )
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::ResponseCodes,
                check: "internal_redirect_loop".to_string(),
                display_name: "Redirect Loops".to_string(),
                severity: Severity::Critical,
                description: "Pages caught in redirect loops".to_string(),
                guidance: "Redirect loops prevent search engines and users from accessing the page. Review the redirect rules and ensure each URL resolves to a final destination without looping.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_internal_no_response(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.status == 0)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some("No response (timeout or DNS failure)".to_string()),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::ResponseCodes,
                check: "internal_no_response".to_string(),
                display_name: "No Response".to_string(),
                severity: Severity::Critical,
                description: "Pages that returned no response".to_string(),
                guidance: "A status code of 0 typically means the server did not respond at all, due to a DNS resolution failure, connection timeout, or network error. Verify the server is reachable and DNS records are correct.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_internal_redirect_chain(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                p.redirect_chain
                    .as_ref()
                    .is_some_and(|chain| chain.len() >= 2)
            })
            .map(|p| {
                let chain = p.redirect_chain.as_ref().unwrap();
                // Each hop records the URL that was *requested* at that step, so
                // the destination is `p.url` and must be appended — the path
                // previously stopped one short and never named where the chain
                // ended up.
                let mut hops: Vec<&str> = chain.iter().map(|h| h.url.as_str()).collect();
                // Unless the chain never resolved — a loop that exhausted the
                // hop budget leaves `p.url` equal to the last hop, and printing
                // it twice reads as a hop that is not there.
                if hops.last() != Some(&p.url.as_str()) {
                    hops.push(&p.url);
                }
                IssueUrl {
                    // The chain belongs to the URL that starts it. Filing it
                    // against `p.url` blamed the destination, which is the one
                    // address in the path that is already correct.
                    url: requested_url(p).to_string(),
                    detail: Some(format!("{} hops: {}", chain.len(), hop_path(&hops))),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::ResponseCodes,
                check: "internal_redirect_chain".to_string(),
                display_name: "Long Redirect Chains".to_string(),
                severity: Severity::Warning,
                description: "Pages with long redirect chains (2+ hops)".to_string(),
                guidance: "Long redirect chains slow down page loading and waste crawl budget. Update internal links to point directly to the final destination URL.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_internal_redirect(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        // A page is recorded at the address that finally served it, with the
        // hops in `redirect_chain`, so testing `status` for 3xx almost never
        // matched and single-hop redirects were invisible — "which URLs
        // redirect?" returned nothing. The redirect chain is the reliable
        // signal. Chains of 2+ hops are reported by the long-chain check, so
        // only single-hop redirects are reported here to avoid double-counting.
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                p.redirect_chain
                    .as_ref()
                    .is_some_and(|chain| chain.len() == 1)
            })
            .map(|p| {
                let hop = &p.redirect_chain.as_ref().unwrap()[0];
                IssueUrl {
                    // The URL that redirects is the head of the chain. Reporting
                    // `p.url` named the *destination* — a 200 — as "Status 301
                    // redirect", so the guidance to repoint internal links sent
                    // the user at the one URL that was already correct, while
                    // the address that actually 301s appeared in no issue.
                    url: hop.url.clone(),
                    detail: Some(format!(
                        "Status {} redirect to {} (final response {})",
                        hop.status,
                        bounded_url(&p.url),
                        p.status
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::ResponseCodes,
                check: "internal_redirect".to_string(),
                display_name: "3xx Redirects".to_string(),
                severity: Severity::Info,
                description: "URLs that redirect to another address".to_string(),
                guidance: "While redirects are sometimes necessary, they add latency. Where possible, update internal links to point to the final destination URL directly.".to_string(),
                urls: affected,
            });
        }
    }
}
