pub mod analyzers;
pub mod scoring;

use crate::crawler::sitemaps::SitemapData;
use crate::models::issue::Issue;
use crate::models::page::PageData;
use std::collections::HashMap;

/// Context available to all analyzers.
pub struct AnalysisContext {
    pub seed_url: String,
    pub domain: String,
    pub sitemap_data: Option<SitemapData>,
    pub pages: Vec<PageData>,
    /// Raw robots.txt content, for AI-crawler policy analysis. `None` when
    /// robots.txt was not fetched (e.g. `--ignore-robots`).
    pub robots_txt: Option<String>,
}

/// Normalize a URL for cross-page comparison, falling back to the raw string
/// when it cannot be parsed. Both sides of any URL join must go through this,
/// or trailing-slash and query-order differences produce phantom mismatches.
pub fn norm_url(url: &str) -> String {
    crate::crawler::url_utils::normalize_url(url).unwrap_or_else(|_| url.to_string())
}

/// Maps every URL the crawl *requested* to the URL that finally served it.
///
/// A `PageData` is filed under the address that actually answered, with
/// `redirect_chain` holding the hops that led there — so `redirect_chain[0].url`
/// is the URL that was originally requested, and that is the form which appears
/// in an `<a href>`, a sitemap `<loc>` or a `rel=canonical`. Joining any such
/// authored URL directly against `page.url` therefore misses every page reached
/// through a redirect, and the miss is silent: a link to `/old` matches no page,
/// so its target's status is unknown (a link that 301s into a 404 is never
/// reported broken), and the destination looks unlinked (false orphan pages,
/// dropped link-graph edges, and a PageRank computed over the wrong graph).
///
/// One side of every such join must be resolved through this map.
#[derive(Debug, Default, Clone)]
pub struct UrlAliases {
    /// normalized requested URL -> normalized URL that served it.
    to_final: HashMap<String, String>,
}

impl UrlAliases {
    /// Build the alias map for one crawl.
    pub fn build(pages: &[PageData]) -> Self {
        let mut to_final: HashMap<String, String> = HashMap::new();

        // Pass 1: every crawled page maps to itself. Done first, and with a
        // plain insert, so a URL that has its own record always wins over a
        // redirect hop of the same name — a crawl that stored an unresolved 3xx
        // (robots blocked the hop, or the hop budget ran out) has real data at
        // that address and must not be aliased away.
        for p in pages {
            let final_key = norm_url(&p.url);
            to_final.insert(final_key.clone(), final_key);
        }

        // Pass 2: each hop of each chain resolves to the URL that served it.
        for p in pages {
            let Some(chain) = &p.redirect_chain else {
                continue;
            };
            let final_key = norm_url(&p.url);
            for hop in chain {
                to_final
                    .entry(norm_url(&hop.url))
                    .or_insert_with(|| final_key.clone());
            }
        }

        Self { to_final }
    }

    /// The normalized URL that served `url`.
    ///
    /// Falls back to `url`'s own normalized form when the crawl never requested
    /// it, so an uncrawled link still compares equal to an uncrawled page.
    pub fn resolve(&self, url: &str) -> String {
        let key = norm_url(url);
        match self.to_final.get(&key) {
            Some(final_url) => final_url.clone(),
            None => key,
        }
    }

    /// The destination `url` redirects to, or `None` when it does not redirect
    /// (either it served its own response, or it was never crawled).
    pub fn redirect_target(&self, url: &str) -> Option<&str> {
        let key = norm_url(url);
        self.to_final
            .get(&key)
            .filter(|final_url| *final_url != &key)
            .map(String::as_str)
    }
}

/// Cap on an issue `detail` string.
///
/// Details are human-readable UI text rendered verbatim into the HTML report
/// and the MCP payload. The analyzers used to join *every* matching item on a
/// page into one line, which produced a single 1.2 MB `<span>` from a page with
/// 20,000 nofollow links and a 5.6 MB report from one page with 50,000 images.
/// The full list already lives in `pages[].internal_links` / `pages[].images`,
/// so the joined copy is duplication — it only needs to be a legible sample.
const MAX_DETAIL_ITEMS: usize = 10;
const MAX_DETAIL_BYTES: usize = 300;

/// Join a sample of `items` for an issue detail, bounded by both count and
/// bytes, and say how many were left out.
///
/// Bounding by count alone is not enough: one 2,000-character URL blows the
/// budget on its own, which is how a 501-page crawl produced a 17 MB report.
pub fn detail_sample(items: &[String], total: usize) -> String {
    let mut out = String::new();
    let mut shown = 0usize;
    for item in items.iter().take(MAX_DETAIL_ITEMS) {
        if !out.is_empty() && out.len() + item.len() + 2 > MAX_DETAIL_BYTES {
            break;
        }
        if !out.is_empty() {
            out.push_str(", ");
        }
        // A single oversized item is truncated rather than dropped, so the
        // detail is never empty.
        if item.len() > MAX_DETAIL_BYTES {
            out.push_str(&item.chars().take(MAX_DETAIL_BYTES).collect::<String>());
            out.push('\u{2026}');
        } else {
            out.push_str(item);
        }
        shown += 1;
    }
    if total > shown {
        out.push_str(&format!(" (+{} more)", total - shown));
    }
    out
}

/// Statuses that mean "deliberately gated", not "broken".
///
/// A login wall answering 403, or a rate limiter answering 429 — often to our
/// own crawl — is not a broken link. The response-code analyzer already excluded
/// these from its critical 4xx check while the link analyzer did not, so the
/// same report called the same URL both "access restricted" (warning) and
/// "broken link" (critical). On itch.io all 18 critical findings were links to
/// /login.
pub const ACCESS_CONTROLLED: [u16; 3] = [401, 403, 429];

/// True if a page is a real HTML document that search engines will index.
///
/// Quality checks that describe a *fixable* SEO defect — duplicate titles,
/// title length, duplicate H1s — should gate on this. A 404's boilerplate
/// title is not a duplicate-title problem the owner can fix, and a noindex
/// page cannot contribute to duplicate content because it never enters the
/// index. Presence checks use the weaker [`PageData::is_html_page`], since a
/// noindex page still ought to have a title.
pub fn is_indexable_html(page: &PageData) -> bool {
    page.is_html_page() && !page.is_noindex()
}

/// Trait implemented by each SEO check category.
pub trait Analyzer {
    fn name(&self) -> &str;
    fn category(&self) -> crate::models::issue::IssueCategory;
    fn analyze(&self, ctx: &AnalysisContext) -> Vec<Issue>;
}

/// Registry that collects and runs all analyzers.
pub struct AnalyzerRegistry {
    analyzers: Vec<Box<dyn Analyzer + Send + Sync>>,
}

impl Default for AnalyzerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AnalyzerRegistry {
    pub fn new() -> Self {
        Self {
            analyzers: Vec::new(),
        }
    }

    pub fn register(&mut self, analyzer: Box<dyn Analyzer + Send + Sync>) {
        self.analyzers.push(analyzer);
    }

    pub fn default_registry() -> Self {
        use analyzers::*;
        let mut reg = Self::new();
        reg.register(Box::new(response_codes::ResponseCodesAnalyzer));
        reg.register(Box::new(security::SecurityAnalyzer));
        reg.register(Box::new(url_checks::UrlAnalyzer));
        reg.register(Box::new(page_titles::PageTitlesAnalyzer));
        reg.register(Box::new(meta_description::MetaDescriptionAnalyzer));
        reg.register(Box::new(headings::HeadingsAnalyzer));
        reg.register(Box::new(content::ContentAnalyzer));
        reg.register(Box::new(images::ImagesAnalyzer));
        reg.register(Box::new(canonicals::CanonicalsAnalyzer));
        reg.register(Box::new(directives::DirectivesAnalyzer));
        reg.register(Box::new(hreflang::HreflangAnalyzer));
        reg.register(Box::new(links::LinksAnalyzer));
        reg.register(Box::new(structured_data::StructuredDataAnalyzer));
        reg.register(Box::new(sitemaps::SitemapsAnalyzer));
        reg.register(Box::new(performance::PerformanceAnalyzer));
        reg.register(Box::new(js::JsAnalyzer));
        reg.register(Box::new(robots::RobotsAnalyzer));
        reg
    }

    pub fn run_all(&self, ctx: &AnalysisContext) -> Vec<Issue> {
        self.analyzers.iter().flat_map(|a| a.analyze(ctx)).collect()
    }

    pub fn analyzer_count(&self) -> usize {
        self.analyzers.len()
    }
}
