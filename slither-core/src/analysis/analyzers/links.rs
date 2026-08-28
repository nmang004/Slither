use crate::analysis::{AnalysisContext, Analyzer, UrlAliases};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};
use std::collections::{HashMap, HashSet};

/// Normalize a URL for comparison, falling back to the raw string on error.
fn norm(url: &str) -> String {
    crate::crawler::url_utils::normalize_url(url).unwrap_or_else(|_| url.to_string())
}

/// True if a link points at a local development host. Matching on the parsed
/// host rather than a substring keeps legitimate hosts like `mylocalhost.com`
/// and paths like `/localhost-setup-guide` from being flagged.
fn is_local_dev_host(url: &str) -> bool {
    match url::Url::parse(url) {
        Ok(parsed) => match parsed.host_str() {
            Some(host) => {
                let host = host.trim_start_matches('[').trim_end_matches(']');
                host.eq_ignore_ascii_case("localhost")
                    || host.to_ascii_lowercase().ends_with(".localhost")
                    || host == "::1"
                    || host
                        .parse::<std::net::Ipv4Addr>()
                        .is_ok_and(|ip| ip.is_loopback())
            }
            None => false,
        },
        Err(_) => false,
    }
}

/// A link is broken if its target errors — but a deliberately gated target
/// (401/403/429) is not broken, and 429 is frequently caused by our own crawl
/// rate. The response-code analyzer already draws this distinction; both now
/// share one definition so they cannot contradict each other in the same report.
fn is_broken(status: u16) -> bool {
    status >= 400 && !crate::analysis::ACCESS_CONTROLLED.contains(&status)
}

pub struct LinksAnalyzer;

impl Analyzer for LinksAnalyzer {
    fn name(&self) -> &str {
        "Links"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Links
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        // Built once and shared by every check that joins an `<a href>` against
        // a crawled page: a page is recorded at the URL that finally served it,
        // so an href naming a URL that redirects matches nothing without this.
        let aliases = UrlAliases::build(&ctx.pages);
        self.check_broken_internal(ctx, &aliases, &mut issues);
        self.check_localhost_outlinks(ctx, &mut issues);
        self.check_orphan_pages(ctx, &aliases, &mut issues);
        self.check_no_internal_outlinks(ctx, &mut issues);
        self.check_no_anchor_text(ctx, &mut issues);
        self.check_non_descriptive_anchor(ctx, &mut issues);
        self.check_internal_nofollow(ctx, &mut issues);
        self.check_high_crawl_depth(ctx, &mut issues);
        self.check_high_external_outlinks(ctx, &mut issues);
        issues
    }
}

const NON_DESCRIPTIVE_ANCHORS: &[&str] = &[
    "click here",
    "read more",
    "learn more",
    "here",
    "link",
    "more",
    "continue",
    "click",
    "this",
];

impl LinksAnalyzer {
    fn check_broken_internal(
        &self,
        ctx: &AnalysisContext,
        aliases: &UrlAliases,
        issues: &mut Vec<Issue>,
    ) {
        // Key the lookup by normalized URL and normalize each link before
        // comparing, so trailing-slash / query-order differences between a link
        // href and the crawled page's URL don't cause false negatives.
        let status_map: HashMap<String, u16> =
            ctx.pages.iter().map(|p| (norm(&p.url), p.status)).collect();

        // A link is as broken as whatever finally answers it. Looking the raw
        // href up in `status_map` only ever matched a URL that served its own
        // response, so `/moved -> 301 -> /gone -> 404` was reported as no
        // problem at all — the href matched no page, and a page whose only bad
        // link redirected produced no Critical finding whatsoever. Resolving
        // the href through the alias map first restores the join.
        let resolve_status = |href: &str| -> Option<(String, u16)> {
            let target = aliases.resolve(href);
            status_map.get(&target).map(|&status| (target, status))
        };

        // Name the redirect explicitly: the href the author has to change is
        // `/moved`, but the 404 they need to see is at `/gone`, and a detail
        // naming only one of the two sends them to the wrong file.
        let describe = |href: &str, target: &str, status: u16| -> String {
            if aliases.redirect_target(href).is_some() {
                format!("{} -> {} (status {})", href, target, status)
            } else {
                format!("{} (status {})", href, status)
            }
        };

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| (200..300).contains(&p.status))
            .filter(|p| {
                p.internal_links
                    .iter()
                    .any(|link| resolve_status(&link.url).is_some_and(|(_, s)| is_broken(s)))
            })
            .map(|p| {
                let broken: Vec<String> = p
                    .internal_links
                    .iter()
                    .filter_map(|link| {
                        let (target, status) = resolve_status(&link.url)?;
                        is_broken(status).then(|| describe(&link.url, &target, status))
                    })
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Broken links: {}",
                        crate::analysis::detail_sample(&broken, broken.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Links,
                check: "broken_internal".to_string(),
                display_name: "Broken Internal Links".to_string(),
                severity: Severity::Critical,
                description: "Pages with broken internal links".to_string(),
                guidance: "Broken internal links hurt user experience and waste crawl budget. Update or remove links pointing to 4xx/5xx pages, or restore the missing target pages.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_localhost_outlinks(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                let mut all_links = p.internal_links.iter().chain(p.external_links.iter());
                all_links.any(|link| is_local_dev_host(&link.url))
            })
            .map(|p| {
                let localhost_links: Vec<String> = p
                    .internal_links
                    .iter()
                    .chain(p.external_links.iter())
                    .filter(|link| is_local_dev_host(&link.url))
                    .map(|link| link.url.clone())
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Localhost links: {}",
                        crate::analysis::detail_sample(&localhost_links, localhost_links.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Links,
                check: "localhost_outlinks".to_string(),
                display_name: "Localhost Links".to_string(),
                severity: Severity::Warning,
                description: "Pages linking to localhost or 127.0.0.1".to_string(),
                guidance: "Links to localhost or 127.0.0.1 are development artifacts that should not appear on production sites. Replace these with the correct production URLs.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_orphan_pages(
        &self,
        ctx: &AnalysisContext,
        aliases: &UrlAliases,
        issues: &mut Vec<Issue>,
    ) {
        // Build set of all URLs that receive at least one internal link
        // Normalized set of every internally-linked URL. Comparing normalized
        // forms avoids false orphans from trailing-slash / query-order
        // differences between a link href and the crawled page URL.
        //
        // Each href is resolved to the page that finally served it first: a
        // link to a URL that 301s is an inbound link to the destination, and
        // without the resolution every page reachable only through a redirect
        // was reported as an orphan with "no incoming internal links" while the
        // homepage linked straight to it.
        //
        // Self-links (a logo or breadcrumb pointing at the current page) are not
        // inbound links from anywhere else, so they must not rescue a page from
        // the orphan check — and the comparison happens after resolution, so a
        // link to a URL that redirects back to the linking page is still a self
        // link. `link_graph` skips them for the same reason.
        let mut linked_urls: HashSet<String> = HashSet::new();
        for page in &ctx.pages {
            let source = norm(&page.url);
            for link in &page.internal_links {
                let target = aliases.resolve(&link.url);
                if target != source {
                    linked_urls.insert(target);
                }
            }
        }

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                (200..300).contains(&p.status)
                    && p.depth > 0 // not the seed URL
                    && !linked_urls.contains(&norm(&p.url))
            })
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("Depth {}, no incoming internal links", p.depth)),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Links,
                check: "orphan_pages".to_string(),
                display_name: "Orphan Pages".to_string(),
                severity: Severity::Warning,
                description: "Orphan pages with no internal links pointing to them".to_string(),
                guidance: "Orphan pages are difficult for search engines and users to discover. Add internal links from relevant pages to ensure these pages are accessible through site navigation.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_no_internal_outlinks(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.is_html_page() && p.internal_links.is_empty())
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: None,
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Links,
                check: "no_internal_outlinks".to_string(),
                display_name: "No Internal Outlinks".to_string(),
                severity: Severity::Warning,
                description: "Pages with no internal outgoing links".to_string(),
                guidance: "Pages without internal links are dead ends that prevent users and crawlers from navigating to other parts of the site. Add relevant internal links to improve site structure.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_no_anchor_text(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                (200..300).contains(&p.status)
                    && p.internal_links
                        .iter()
                        .any(|link| link.anchor.trim().is_empty())
            })
            .map(|p| {
                let empty_count = p
                    .internal_links
                    .iter()
                    .filter(|link| link.anchor.trim().is_empty())
                    .count();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!("{} link(s) with empty anchor text", empty_count)),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Links,
                check: "no_anchor_text".to_string(),
                display_name: "Empty Anchor Text".to_string(),
                severity: Severity::Info,
                description: "Internal links with empty anchor text".to_string(),
                guidance: "Anchor text helps search engines understand the context of the linked page. Add descriptive anchor text to internal links instead of leaving them empty.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_non_descriptive_anchor(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                (200..300).contains(&p.status)
                    && p.internal_links.iter().any(|link| {
                        let lower = link.anchor.trim().to_lowercase();
                        NON_DESCRIPTIVE_ANCHORS.contains(&lower.as_str())
                    })
            })
            .map(|p| {
                let bad: Vec<String> = p
                    .internal_links
                    .iter()
                    .filter(|link| {
                        let lower = link.anchor.trim().to_lowercase();
                        NON_DESCRIPTIVE_ANCHORS.contains(&lower.as_str())
                    })
                    .map(|link| format!("\"{}\" -> {}", link.anchor, link.url))
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "Non-descriptive anchors: {}",
                        crate::analysis::detail_sample(&bad, bad.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Links,
                check: "non_descriptive_anchor".to_string(),
                display_name: "Non-Descriptive Anchors".to_string(),
                severity: Severity::Info,
                description: "Internal links with non-descriptive anchor text".to_string(),
                guidance: "Generic anchor text like 'click here' or 'read more' provides no context to search engines. Use descriptive anchor text that indicates the topic of the destination page.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_internal_nofollow(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| {
                (200..300).contains(&p.status) && p.internal_links.iter().any(|link| link.nofollow)
            })
            .map(|p| {
                let nofollow_links: Vec<String> = p
                    .internal_links
                    .iter()
                    .filter(|link| link.nofollow)
                    .map(|link| link.url.clone())
                    .collect();
                IssueUrl {
                    url: p.url.clone(),
                    detail: Some(format!(
                        "{} nofollow link(s): {}",
                        nofollow_links.len(),
                        crate::analysis::detail_sample(&nofollow_links, nofollow_links.len())
                    )),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Links,
                check: "internal_nofollow".to_string(),
                display_name: "Internal Nofollow Links".to_string(),
                severity: Severity::Info,
                description: "Internal links with nofollow attribute".to_string(),
                guidance: "Using nofollow on internal links prevents link equity from flowing to those pages. Remove nofollow from internal links unless there is a specific reason to restrict crawling.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_high_crawl_depth(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.depth > 4)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("Depth {}", p.depth)),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Links,
                check: "high_crawl_depth".to_string(),
                display_name: "High Crawl Depth (5+)".to_string(),
                severity: Severity::Info,
                description: "Pages with a crawl depth greater than 4".to_string(),
                guidance: "Pages buried deep in the site structure are harder for search engines to discover and may receive less crawl priority. Improve internal linking to bring important pages closer to the homepage.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_high_external_outlinks(&self, ctx: &AnalysisContext, issues: &mut Vec<Issue>) {
        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            .filter(|p| p.external_links.len() > 100)
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: Some(format!("{} external links", p.external_links.len())),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Links,
                check: "high_external_outlinks".to_string(),
                display_name: "Excessive External Links".to_string(),
                severity: Severity::Info,
                description: "Pages with more than 100 external outgoing links".to_string(),
                guidance: "Excessive external links may dilute page authority and can appear spammy. Review and reduce the number of external links, keeping only those relevant to the content.".to_string(),
                urls: affected,
            });
        }
    }
}
