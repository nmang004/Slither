use crate::analysis::{AnalysisContext, Analyzer};
use crate::models::issue::{Issue, IssueCategory, IssueUrl, Severity};
use std::collections::{HashMap, HashSet};

/// Normalize a URL for sitemap-vs-crawl comparison, falling back to the raw
/// value on parse error.
fn norm_sitemap(url: &str) -> String {
    crate::crawler::url_utils::normalize_url(url).unwrap_or_else(|_| url.to_string())
}

pub struct SitemapsAnalyzer;

impl Analyzer for SitemapsAnalyzer {
    fn name(&self) -> &str {
        "Sitemaps"
    }
    fn category(&self) -> IssueCategory {
        IssueCategory::Sitemaps
    }
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        let mut issues = Vec::new();
        match &ctx.sitemap_data {
            // `None` means sitemap discovery never ran for this crawl (the
            // playwright backend, `inspect`, and the PageSpeed re-run all skip
            // it), which is not evidence that the site lacks a sitemap.
            // Only an actual discovery pass that found nothing is reportable.
            None => {}
            Some(data) => {
                // A declared sitemap that could not be fetched is reported even
                // when a working one was found elsewhere: "your robots.txt
                // points at a dead sitemap" is a real defect that was
                // previously recorded in `errors` and never surfaced.
                self.check_unreachable_declared(data, &mut issues);
                self.check_malformed_sitemap(data, &mut issues);
                self.check_invalid_locs(data, &mut issues);

                if data.sitemap_sources.is_empty() {
                    self.check_no_sitemap_found(&mut issues);
                } else {
                    self.check_empty_sitemap(data, &mut issues);
                    self.check_non_indexable_in_sitemap(ctx, data, &mut issues);
                    self.check_over_50k_urls(data, &mut issues);
                    self.check_over_50mb(data, &mut issues);
                    self.check_urls_not_in_sitemap(ctx, data, &mut issues);
                    self.check_url_in_multiple_sitemaps(data, &mut issues);
                }
            }
        }
        issues
    }
}

impl SitemapsAnalyzer {
    fn check_no_sitemap_found(&self, issues: &mut Vec<Issue>) {
        issues.push(Issue {
            category: IssueCategory::Sitemaps,
            check: "no_sitemap_found".to_string(),
            display_name: "No Sitemap Found".to_string(),
            severity: Severity::Warning,
            description: "No XML sitemap found".to_string(),
            guidance: "An XML sitemap helps search engines discover and crawl your pages efficiently. Create a sitemap.xml and submit it via Google Search Console and Bing Webmaster Tools.".to_string(),
            urls: Vec::new(),
        });
    }

    /// A sitemap that was declared but could not be read. Distinct from having
    /// no sitemap: telling the user to "create a sitemap.xml" when they have one
    /// and their robots.txt merely points at the wrong path is actively wrong
    /// advice.
    fn check_unreachable_declared(
        &self,
        data: &crate::crawler::sitemaps::SitemapData,
        issues: &mut Vec<Issue>,
    ) {
        if data.unreachable.is_empty() {
            return;
        }
        let urls: Vec<IssueUrl> = data
            .unreachable
            .iter()
            .map(|u| IssueUrl {
                url: u.clone(),
                detail: Some("Declared sitemap could not be read".to_string()),
            })
            .collect();
        issues.push(Issue {
            category: IssueCategory::Sitemaps,
            check: "sitemap_unreachable".to_string(),
            display_name: "Declared Sitemap Unreachable".to_string(),
            severity: Severity::Warning,
            description: "A sitemap referenced in robots.txt could not be fetched".to_string(),
            guidance: "Search engines follow the Sitemap: line in robots.txt. Point it at a URL that returns 200, or remove the stale reference.".to_string(),
            urls,
        });
    }

    /// Search Console reports a sitemap it cannot parse as "Sitemap could not be
    /// read" and ignores it entirely, so a partial read must not be presented as
    /// a healthy source.
    fn check_malformed_sitemap(
        &self,
        data: &crate::crawler::sitemaps::SitemapData,
        issues: &mut Vec<Issue>,
    ) {
        if data.malformed.is_empty() {
            return;
        }
        let urls: Vec<IssueUrl> = data
            .malformed
            .iter()
            .map(|u| IssueUrl {
                url: u.clone(),
                detail: Some("XML parse error — the file is truncated or invalid".to_string()),
            })
            .collect();
        issues.push(Issue {
            category: IssueCategory::Sitemaps,
            check: "sitemap_parse_error".to_string(),
            display_name: "Sitemap XML Parse Error".to_string(),
            severity: Severity::Warning,
            description: "A sitemap could not be parsed as valid XML".to_string(),
            guidance: "Search engines reject a sitemap they cannot parse, so none of its URLs are read. Validate the file and re-submit it.".to_string(),
            urls,
        });
    }

    /// The protocol requires fully-qualified URLs; a relative `<loc>` is ignored
    /// by search engines.
    fn check_invalid_locs(
        &self,
        data: &crate::crawler::sitemaps::SitemapData,
        issues: &mut Vec<Issue>,
    ) {
        if data.invalid_locs.is_empty() {
            return;
        }
        let urls: Vec<IssueUrl> = data
            .invalid_locs
            .iter()
            .map(|u| IssueUrl {
                url: u.clone(),
                detail: Some("Not an absolute http(s) URL".to_string()),
            })
            .collect();
        issues.push(Issue {
            category: IssueCategory::Sitemaps,
            check: "sitemap_invalid_loc".to_string(),
            display_name: "Invalid Sitemap URL".to_string(),
            severity: Severity::Warning,
            description: "Sitemap entries that are not absolute URLs".to_string(),
            guidance: "The sitemap protocol requires each <loc> to be a complete URL including scheme and host. Relative paths are ignored.".to_string(),
            urls,
        });
    }

    /// An empty `<urlset>` is invalid against the sitemaps.org schema, which
    /// requires at least one `<url>`, and Search Console reports it as "Sitemap
    /// is empty".
    fn check_empty_sitemap(
        &self,
        data: &crate::crawler::sitemaps::SitemapData,
        issues: &mut Vec<Issue>,
    ) {
        let affected: Vec<IssueUrl> = data
            .sitemap_sources
            .iter()
            .filter(|s| s.url_count == 0)
            .map(|s| IssueUrl {
                url: s.url.clone(),
                detail: Some("Contains no URLs".to_string()),
            })
            .collect();
        if affected.is_empty() {
            return;
        }
        issues.push(Issue {
            category: IssueCategory::Sitemaps,
            check: "sitemap_empty".to_string(),
            display_name: "Empty Sitemap".to_string(),
            severity: Severity::Warning,
            description: "A sitemap contains no URLs".to_string(),
            guidance: "An empty sitemap is rejected by search engines and is invalid against the sitemaps.org schema, which requires at least one <url>. Populate it or remove the reference.".to_string(),
            urls: affected,
        });
    }

    fn check_non_indexable_in_sitemap(
        &self,
        ctx: &AnalysisContext,
        data: &crate::crawler::sitemaps::SitemapData,
        issues: &mut Vec<Issue>,
    ) {
        let page_map: HashMap<String, &crate::models::page::PageData> = ctx
            .pages
            .iter()
            .map(|p| (norm_sitemap(&p.url), p))
            .collect();

        let affected: Vec<IssueUrl> = data
            .sitemap_urls
            .iter()
            .filter_map(|entry| {
                if let Some(page) = page_map.get(&norm_sitemap(&entry.url)) {
                    let has_noindex = page.is_noindex();
                    let is_error = page.status >= 400;
                    if has_noindex || is_error {
                        let reason = if is_error {
                            format!("status {}", page.status)
                        } else {
                            "noindex".to_string()
                        };
                        Some(IssueUrl {
                            url: entry.url.clone(),
                            detail: Some(format!(
                                "Non-indexable ({}) in {}",
                                reason, entry.source_sitemap
                            )),
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Sitemaps,
                check: "non_indexable_in_sitemap".to_string(),
                display_name: "Non-Indexable in Sitemap".to_string(),
                severity: Severity::Warning,
                description: "Non-indexable URLs found in sitemap".to_string(),
                guidance: "Sitemaps should only contain indexable, 200-status URLs. Remove noindexed pages and error pages from the sitemap to avoid wasting crawl budget.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_over_50k_urls(
        &self,
        data: &crate::crawler::sitemaps::SitemapData,
        issues: &mut Vec<Issue>,
    ) {
        let affected: Vec<IssueUrl> = data
            .sitemap_sources
            .iter()
            .filter(|s| s.url_count > 50_000)
            .map(|s| IssueUrl {
                url: s.url.clone(),
                detail: Some(format!("{} URLs (limit 50,000)", s.url_count)),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Sitemaps,
                check: "over_50k_urls".to_string(),
                display_name: "Sitemap Over 50k URLs".to_string(),
                severity: Severity::Warning,
                description: "Sitemaps exceeding the 50,000 URL limit".to_string(),
                guidance: "The sitemap protocol limits each sitemap file to 50,000 URLs. Split large sitemaps into multiple files and reference them from a sitemap index.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_over_50mb(
        &self,
        data: &crate::crawler::sitemaps::SitemapData,
        issues: &mut Vec<Issue>,
    ) {
        let affected: Vec<IssueUrl> = data
            .sitemap_sources
            .iter()
            .filter(|s| s.size_bytes > 50_000_000)
            .map(|s| IssueUrl {
                url: s.url.clone(),
                detail: Some(format!(
                    "{:.1} MB (limit 50 MB)",
                    s.size_bytes as f64 / 1_000_000.0
                )),
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Sitemaps,
                check: "over_50mb".to_string(),
                display_name: "Sitemap Over 50 MB".to_string(),
                severity: Severity::Warning,
                description: "Sitemaps exceeding the 50 MB size limit".to_string(),
                guidance: "The sitemap protocol limits each sitemap file to 50 MB uncompressed. Split large sitemaps into smaller files and reference them from a sitemap index.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_urls_not_in_sitemap(
        &self,
        ctx: &AnalysisContext,
        data: &crate::crawler::sitemaps::SitemapData,
        issues: &mut Vec<Issue>,
    ) {
        // Collection stopped at the cap, so absence from the partial set proves
        // nothing. A site whose first child sitemap fills the cap had every
        // crawled page reported as missing from a sitemap that in fact lists
        // them — verified on itch.io, rei.com, canada.ca and aljazeera.net.
        if data.truncated {
            return;
        }
        // A sitemap we could not parse may well contain these URLs.
        if !data.malformed.is_empty() {
            return;
        }

        // Compare normalized forms so a sitemap using trailing slashes doesn't
        // report every crawled page as "not in sitemap".
        let sitemap_urls: HashSet<String> = data
            .sitemap_urls
            .iter()
            .map(|e| norm_sitemap(&e.url))
            .collect();

        let affected: Vec<IssueUrl> = ctx
            .pages
            .iter()
            // "Canonicalised elsewhere" is the exclusion we want. Requiring a
            // self-canonical instead disabled this check entirely on sites that
            // declare no canonical tags at all.
            .filter(|p| {
                p.is_html_page()
                    && !p.is_noindex()
                    && (p.canonical.is_none() || p.has_self_canonical)
            })
            .filter(|p| !sitemap_urls.contains(&norm_sitemap(&p.url)))
            .map(|p| IssueUrl {
                url: p.url.clone(),
                detail: None,
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Sitemaps,
                check: "urls_not_in_sitemap".to_string(),
                display_name: "Pages Not in Sitemap".to_string(),
                severity: Severity::Info,
                description: "Crawled pages not found in any sitemap".to_string(),
                guidance: "All indexable pages should be included in the XML sitemap. Add these URLs to your sitemap to help search engines discover and crawl them.".to_string(),
                urls: affected,
            });
        }
    }

    fn check_url_in_multiple_sitemaps(
        &self,
        data: &crate::crawler::sitemaps::SitemapData,
        issues: &mut Vec<Issue>,
    ) {
        // Group sitemap entries by URL and find those appearing in multiple sources
        let mut url_sources: HashMap<&str, HashSet<&str>> = HashMap::new();
        for entry in &data.sitemap_urls {
            url_sources
                .entry(entry.url.as_str())
                .or_default()
                .insert(entry.source_sitemap.as_str());
        }

        let affected: Vec<IssueUrl> = url_sources
            .iter()
            .filter(|(_, sources)| sources.len() > 1)
            .map(|(url, sources)| {
                let source_list: Vec<&str> = sources.iter().copied().collect();
                IssueUrl {
                    url: url.to_string(),
                    detail: Some(format!("Found in: {}", source_list.join(", "))),
                }
            })
            .collect();
        if !affected.is_empty() {
            issues.push(Issue {
                category: IssueCategory::Sitemaps,
                check: "url_in_multiple_sitemaps".to_string(),
                display_name: "URLs in Multiple Sitemaps".to_string(),
                severity: Severity::Info,
                description: "URLs appearing in multiple sitemaps".to_string(),
                guidance: "While not strictly an error, having the same URL in multiple sitemaps can be redundant. Consider organizing sitemaps so each URL appears in only one file.".to_string(),
                urls: affected,
            });
        }
    }
}
