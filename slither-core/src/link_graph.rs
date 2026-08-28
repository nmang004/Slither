//! Internal link-graph analysis over a completed crawl.
//!
//! This replaces the former Python `slither-link` package. PageRank / internal
//! link-equity over thousands of pages is exactly the compact, high-signal
//! artifact an LLM wants handed to it — and it is genuinely expensive to compute
//! in-context — so it lives here in Rust, reachable from the CLI and MCP.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::analysis::UrlAliases;
use crate::crawler::url_utils::normalize_url;
use crate::models::page::PageData;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedPage {
    pub url: String,
    pub pagerank: f64,
    pub in_degree: usize,
    pub out_degree: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiloCount {
    pub silo: String,
    pub pages: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkGraphReport {
    pub total_nodes: usize,
    pub total_edges: usize,
    /// Weakly connected components (edges treated as undirected).
    pub components: usize,
    pub avg_out_degree: f64,
    /// Pages with no internal inbound links (excluding the seed), bounded by
    /// `top_n`. `orphan_page_count` carries the true total.
    pub orphan_pages: Vec<String>,
    /// Total orphan pages found, before `orphan_pages` was truncated.
    pub orphan_page_count: usize,
    /// Highest internal-authority pages by PageRank.
    pub top_by_pagerank: Vec<RankedPage>,
    /// Pages that link out the most (navigation hubs).
    pub top_hubs: Vec<RankedPage>,
    /// Pages grouped by first path segment ("silo"), bounded by `top_n`.
    pub silo_distribution: Vec<SiloCount>,
    /// Total distinct silos found, before `silo_distribution` was truncated.
    pub silo_count: usize,
}

/// Compute the internal link graph from crawled pages. `top_n` bounds the
/// ranked lists. `seed_url` (if known) is excluded from orphan reporting.
pub fn compute_link_graph(
    pages: &[PageData],
    seed_url: Option<&str>,
    top_n: usize,
) -> LinkGraphReport {
    // A page is recorded at the URL that finally served it, so an `<a href>`
    // naming a URL that redirects matches no node. Every such edge was silently
    // dropped: the destination showed in-degree 0 (a false orphan), the source
    // showed out-degree 0, and `total_edges`, `avg_out_degree`, the component
    // count and every PageRank value were computed over the wrong graph.
    let aliases = UrlAliases::build(pages);

    // Nodes: normalized URLs of crawled pages. Index them for union-find.
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut urls: Vec<String> = Vec::new();
    for p in pages {
        let key = normalize_url(&p.url).unwrap_or_else(|_| p.url.clone());
        index.entry(key.clone()).or_insert_with(|| {
            urls.push(key.clone());
            urls.len() - 1
        });
    }
    let n = urls.len();

    let mut out_degree = vec![0usize; n];
    let mut in_degree = vec![0usize; n];
    // Adjacency for PageRank: out_links[i] = destination node indices.
    let mut out_links: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut uf = UnionFind::new(n);
    let mut total_edges = 0usize;

    for p in pages {
        let src_key = normalize_url(&p.url).unwrap_or_else(|_| p.url.clone());
        let Some(&src) = index.get(&src_key) else {
            continue;
        };
        // Dedup destinations per source page so repeated nav links count once.
        let mut seen_dst = std::collections::HashSet::new();
        for link in &p.internal_links {
            let dst_key = aliases.resolve(&link.url);
            let Some(&dst) = index.get(&dst_key) else {
                continue; // link to an uncrawled URL — not a graph node
            };
            if dst == src || !seen_dst.insert(dst) {
                continue;
            }
            out_degree[src] += 1;
            in_degree[dst] += 1;
            out_links[src].push(dst);
            uf.union(src, dst);
            total_edges += 1;
        }
    }

    // The seed is excluded from orphan reporting by identity, so it too must be
    // resolved: a site whose seed `/` 301s to `/en/` files the page under
    // `/en/`, and comparing the unresolved seed reported the homepage itself as
    // an orphan.
    let seed_key = seed_url.map(|s| aliases.resolve(s));
    // `top_n` bounds every list in the report: a site with thousands of
    // orphans would otherwise emit them all through the MCP tool, which is the
    // same context blow-up an uncapped affected-URL list causes.
    let all_orphans: Vec<String> = (0..n)
        .filter(|&i| in_degree[i] == 0)
        .filter(|&i| seed_key.as_deref() != Some(urls[i].as_str()))
        .map(|i| urls[i].clone())
        .collect();
    let orphan_page_count = all_orphans.len();
    let orphan_pages: Vec<String> = all_orphans.into_iter().take(top_n).collect();

    let pagerank = compute_pagerank(&out_links, n);

    let mut ranked: Vec<RankedPage> = (0..n)
        .map(|i| RankedPage {
            url: urls[i].clone(),
            pagerank: pagerank[i],
            in_degree: in_degree[i],
            out_degree: out_degree[i],
        })
        .collect();

    let mut top_by_pagerank = ranked.clone();
    top_by_pagerank.sort_by(|a, b| {
        b.pagerank
            .partial_cmp(&a.pagerank)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    top_by_pagerank.truncate(top_n);

    ranked.sort_by_key(|p| std::cmp::Reverse(p.out_degree));
    let top_hubs: Vec<RankedPage> = ranked.into_iter().take(top_n).collect();

    // Silo distribution by first path segment.
    let mut silos: HashMap<String, usize> = HashMap::new();
    for url in &urls {
        let silo = first_path_segment(url);
        *silos.entry(silo).or_insert(0) += 1;
    }
    let mut silo_distribution: Vec<SiloCount> = silos
        .into_iter()
        .map(|(silo, pages)| SiloCount { silo, pages })
        .collect();
    silo_distribution.sort_by(|a, b| b.pages.cmp(&a.pages).then_with(|| a.silo.cmp(&b.silo)));
    let silo_count = silo_distribution.len();
    silo_distribution.truncate(top_n);

    let avg_out_degree = if n > 0 {
        total_edges as f64 / n as f64
    } else {
        0.0
    };

    LinkGraphReport {
        total_nodes: n,
        total_edges,
        components: uf.component_count(),
        avg_out_degree,
        orphan_pages,
        orphan_page_count,
        top_by_pagerank,
        top_hubs,
        silo_distribution,
        silo_count,
    }
}

/// PageRank via power iteration (damping 0.85). Dangling nodes (no out-links)
/// distribute their rank uniformly so the total stays conserved.
fn compute_pagerank(out_links: &[Vec<usize>], n: usize) -> Vec<f64> {
    if n == 0 {
        return Vec::new();
    }
    const DAMPING: f64 = 0.85;
    const ITERATIONS: usize = 40;
    let base = (1.0 - DAMPING) / n as f64;
    let mut rank = vec![1.0 / n as f64; n];

    for _ in 0..ITERATIONS {
        let mut next = vec![base; n];
        let mut dangling_sum = 0.0;
        for i in 0..n {
            if out_links[i].is_empty() {
                dangling_sum += rank[i];
            } else {
                let share = DAMPING * rank[i] / out_links[i].len() as f64;
                for &dst in &out_links[i] {
                    next[dst] += share;
                }
            }
        }
        // Redistribute dangling rank uniformly.
        let dangling = DAMPING * dangling_sum / n as f64;
        for r in next.iter_mut() {
            *r += dangling;
        }
        rank = next;
    }
    rank
}

fn first_path_segment(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next().map(|seg| seg.to_string()))
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "(root)".to_string())
}

/// Simple union-find for counting weakly connected components.
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }
    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        // Path compression.
        let mut cur = x;
        while self.parent[cur] != root {
            let next = self.parent[cur];
            self.parent[cur] = root;
            cur = next;
        }
        root
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        if self.rank[ra] < self.rank[rb] {
            self.parent[ra] = rb;
        } else if self.rank[ra] > self.rank[rb] {
            self.parent[rb] = ra;
        } else {
            self.parent[rb] = ra;
            self.rank[ra] += 1;
        }
    }
    fn component_count(&mut self) -> usize {
        let n = self.parent.len();
        let mut roots = std::collections::HashSet::new();
        for i in 0..n {
            let r = self.find(i);
            roots.insert(r);
        }
        roots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::page::{LinkData, PageData};

    fn page(url: &str, links: &[&str]) -> PageData {
        let mut p = PageData {
            url: url.to_string(),
            ..default_page()
        };
        p.internal_links = links
            .iter()
            .map(|l| LinkData {
                url: l.to_string(),
                anchor: String::new(),
                nofollow: false,
            })
            .collect();
        p
    }

    /// A page reached through a 301: recorded at `url` (where the response came
    /// from) with `requested` — the address that appears in `<a href>`s — as the
    /// head of its chain.
    fn redirected_page(url: &str, requested: &str, links: &[&str]) -> PageData {
        let mut p = page(url, links);
        p.redirect_chain = Some(vec![crate::models::page::RedirectHop {
            status: 301,
            url: requested.to_string(),
        }]);
        p
    }

    fn default_page() -> PageData {
        // Minimal PageData built from a JSON string (avoids the json! macro's
        // recursion limit on a large object); only url + internal_links matter.
        let json = r#"{
            "url": "", "status": 200, "response_time_ms": 0, "depth": 0,
            "h1": [], "headings": [], "word_count": 0, "body_text": "",
            "internal_links": [], "external_links": [], "images": [],
            "schema_types": [], "og_tags": {}, "content_hash": "", "is_https": true,
            "security_headers": {"has_hsts": false, "has_csp": false,
                "has_x_content_type_options": false, "has_x_frame_options": false,
                "has_referrer_policy": false},
            "mixed_content": [], "insecure_forms": [], "url_length": 0,
            "has_parameters": false, "has_underscores": false, "has_uppercase": false,
            "has_non_ascii": false, "has_multiple_slashes": false, "has_repetitive_path": false,
            "title_count": 0, "meta_description_count": 0, "title_in_head": false,
            "meta_desc_in_head": false, "canonical_is_relative": false, "canonical_count": 0,
            "has_self_canonical": false, "meta_robots_directives": [], "hreflang_tags": [],
            "is_soft_404": false, "structured_data": [], "unsafe_cross_origin_links": [],
            "js_injected_title": false, "js_injected_description": false,
            "js_injected_canonical": false, "js_injected_h1": false,
            "js_injected_structured_data": false, "console_errors": [], "scripts": []
        }"#;
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn orphans_hubs_and_components() {
        let pages = vec![
            page("https://x.com/", &["https://x.com/a", "https://x.com/b"]),
            page("https://x.com/a", &["https://x.com/b"]),
            page("https://x.com/b", &[]),
            page("https://x.com/orphan", &[]),
        ];
        let r = compute_link_graph(&pages, Some("https://x.com/"), 10);
        assert_eq!(r.total_nodes, 4);
        assert_eq!(r.total_edges, 3);
        // /orphan has no inbound links and isn't the seed.
        assert!(r.orphan_pages.contains(&"https://x.com/orphan".to_string()));
        // The seed is excluded even though it has in-degree 0.
        assert!(!r.orphan_pages.contains(&"https://x.com/".to_string()));
        // Two components: {/, a, b} and {orphan}.
        assert_eq!(r.components, 2);
        // The most-linked-to page (/b, in-degree 2) should have high PageRank.
        assert_eq!(r.top_by_pagerank[0].url, "https://x.com/b");
    }

    /// B3: an `<a href>` naming a URL that 301s must still form an edge to the
    /// page that answered it. Without resolving the href, the edge was dropped
    /// entirely — the destination showed in-degree 0 (a false orphan), the
    /// source out-degree 0, and `total_edges`, `avg_out_degree` and every
    /// PageRank value were computed over a graph the site does not have.
    #[test]
    fn links_through_a_redirect_are_edges() {
        let pages = vec![
            page(
                "https://x.com/",
                &["https://x.com/old-a", "https://x.com/old-b"],
            ),
            redirected_page(
                "https://x.com/new-a",
                "https://x.com/old-a",
                &["https://x.com/"],
            ),
            redirected_page(
                "https://x.com/new-b",
                "https://x.com/old-b",
                &["https://x.com/"],
            ),
        ];
        let r = compute_link_graph(&pages, Some("https://x.com/"), 10);

        assert_eq!(r.total_nodes, 3, "only served URLs are nodes");
        assert_eq!(
            r.total_edges, 4,
            "2 edges out of the homepage through 301s, 2 back to it"
        );
        assert!(
            r.orphan_pages.is_empty(),
            "both destinations are linked from the homepage: {:?}",
            r.orphan_pages
        );
        assert_eq!(r.components, 1, "the redirect edges connect the graph");

        let home = r
            .top_by_pagerank
            .iter()
            .find(|p| p.url == "https://x.com/")
            .expect("homepage is ranked");
        assert_eq!(
            (home.in_degree, home.out_degree),
            (2, 2),
            "the homepage links out through 301s and is linked back"
        );
    }

    /// The seed itself may redirect (`/` 301s to `/en/`), in which case the page
    /// is filed at the destination. Comparing the unresolved seed reported the
    /// site's own homepage as an orphan.
    #[test]
    fn a_redirecting_seed_is_not_an_orphan() {
        let pages = vec![redirected_page("https://x.com/en/", "https://x.com/", &[])];
        let r = compute_link_graph(&pages, Some("https://x.com/"), 10);
        assert!(
            r.orphan_pages.is_empty(),
            "the seed is excluded from orphan reporting even when it redirects: {:?}",
            r.orphan_pages
        );
    }

    #[test]
    fn empty_graph() {
        let r = compute_link_graph(&[], None, 5);
        assert_eq!(r.total_nodes, 0);
        assert_eq!(r.components, 0);
        assert!(r.orphan_pages.is_empty());
    }
}
