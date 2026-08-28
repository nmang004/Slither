//! Generate an XML sitemap from crawled pages.

use crate::models::page::PageData;

/// The sitemaps.org protocol caps a single file at 50,000 URLs; Google rejects
/// an over-limit file outright. Larger crawls are split and referenced from a
/// `<sitemapindex>`.
pub const MAX_URLS_PER_SITEMAP: usize = 50_000;

/// The schema types `<loc>` as maxLength 2048, so a longer URL makes the whole
/// file fail validation.
const MAX_LOC_LENGTH: usize = 2048;

/// One generated file, ready to write.
pub struct SitemapFile {
    /// Suggested filename, relative to wherever the caller writes it.
    pub filename: String,
    pub xml: String,
}

/// The indexable URLs a sitemap would list: 2xx HTML, not noindex, and either
/// self-canonical or declaring no canonical.
///
/// Exposed so a caller can report the count — writing an empty sitemap and
/// printing a success message left the user with a file search engines reject
/// ("Sitemap is empty") and no way to notice.
pub fn indexable_urls(pages: &[PageData]) -> Vec<String> {
    let mut urls: Vec<String> = pages
        .iter()
        .filter(|p| p.is_html_page() && !p.is_noindex())
        .filter(|p| {
            // Include only pages that are their own canonical (or declare none).
            match &p.canonical {
                Some(c) => p.has_self_canonical || c == &p.url,
                None => true,
            }
        })
        // An over-long <loc> invalidates the entire document against the
        // official schema, taking every other URL down with it.
        .filter(|p| p.url.len() <= MAX_LOC_LENGTH)
        .map(|p| p.url.clone())
        .collect();
    urls.sort_unstable();
    urls.dedup();
    urls
}

/// Build a sitemaps.org-compliant `<urlset>` XML document listing the crawl's
/// indexable pages. URLs are XML-escaped.
///
/// Note this emits a single file. Use [`generate_sitemap_set`] for a crawl that
/// may exceed [`MAX_URLS_PER_SITEMAP`].
pub fn generate_sitemap(pages: &[PageData]) -> String {
    urlset_xml(&indexable_urls(pages))
}

/// Build the complete set of sitemap files for a crawl, splitting at the
/// protocol's 50,000-URL limit and adding a `<sitemapindex>` when more than one
/// file is needed.
///
/// `base_url` is the public directory the files will be served from (e.g.
/// `https://example.com/`), used to build absolute `<loc>` values in the index,
/// which the protocol requires.
///
/// Returns an empty vector when no page qualifies, so the caller can say so
/// rather than writing an empty `<urlset>` — which is invalid against the
/// official schema and rejected by Google.
pub fn generate_sitemap_set(pages: &[PageData], base_url: &str) -> Vec<SitemapFile> {
    let urls = indexable_urls(pages);
    if urls.is_empty() {
        return Vec::new();
    }

    let chunks: Vec<&[String]> = urls.chunks(MAX_URLS_PER_SITEMAP).collect();
    if chunks.len() == 1 {
        return vec![SitemapFile {
            filename: "sitemap.xml".to_string(),
            xml: urlset_xml(chunks[0]),
        }];
    }

    let mut files: Vec<SitemapFile> = chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| SitemapFile {
            filename: format!("sitemap-{}.xml", i + 1),
            xml: urlset_xml(chunk),
        })
        .collect();

    let base = base_url.trim_end_matches('/');
    let mut index = String::with_capacity(files.len() * 100 + 128);
    index.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    index.push_str("<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for f in &files {
        index.push_str("  <sitemap>\n    <loc>");
        index.push_str(&xml_escape(&format!("{}/{}", base, f.filename)));
        index.push_str("</loc>\n  </sitemap>\n");
    }
    index.push_str("</sitemapindex>\n");

    files.insert(
        0,
        SitemapFile {
            filename: "sitemap.xml".to_string(),
            xml: index,
        },
    );
    files
}

fn urlset_xml(urls: &[String]) -> String {
    let mut out = String::with_capacity(urls.len() * 80 + 128);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n");
    for url in urls {
        out.push_str("  <url>\n    <loc>");
        out.push_str(&xml_escape(url));
        out.push_str("</loc>\n  </url>\n");
    }
    out.push_str("</urlset>\n");
    out
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(url: &str, status: u16, noindex: bool) -> crate::models::page::PageData {
        let json = format!(
            r#"{{"url":"{url}","status":{status},"response_time_ms":0,"depth":0,
            "content_type":"text/html","h1":[],"headings":[],"word_count":0,"body_text":"",
            "internal_links":[],"external_links":[],"images":[],"schema_types":[],"og_tags":{{}},
            "content_hash":"","is_https":true,"security_headers":{{"has_hsts":false,"has_csp":false,
            "has_x_content_type_options":false,"has_x_frame_options":false,"has_referrer_policy":false}},
            "mixed_content":[],"insecure_forms":[],"url_length":0,"has_parameters":false,
            "has_underscores":false,"has_uppercase":false,"has_non_ascii":false,
            "has_multiple_slashes":false,"has_repetitive_path":false,"title_count":0,
            "meta_description_count":0,"title_in_head":false,"meta_desc_in_head":false,
            "canonical_is_relative":false,"canonical_count":0,"has_self_canonical":false,
            "meta_robots_directives":{},"hreflang_tags":[],"is_soft_404":false,
            "structured_data":[],"unsafe_cross_origin_links":[],"js_injected_title":false,
            "js_injected_description":false,"js_injected_canonical":false,"js_injected_h1":false,
            "js_injected_structured_data":false,"console_errors":[],"scripts":[]}}"#,
            if noindex { r#"["noindex"]"# } else { "[]" }
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn includes_only_indexable_pages() {
        let pages = vec![
            page("https://e.com/", 200, false),
            page("https://e.com/noindex", 200, true),
            page("https://e.com/404", 404, false),
        ];
        let xml = generate_sitemap(&pages);
        assert!(xml.contains("<loc>https://e.com/</loc>"));
        assert!(!xml.contains("/noindex"));
        assert!(!xml.contains("/404"));
    }

    #[test]
    fn escapes_urls() {
        let pages = vec![page("https://e.com/a?x=1&y=2", 200, false)];
        let xml = generate_sitemap(&pages);
        assert!(xml.contains("https://e.com/a?x=1&amp;y=2"));
    }

    /// A single `<urlset>` may hold at most 50,000 URLs; a larger crawl must be
    /// split behind a `<sitemapindex>`. Writing 60,001 into one file produced an
    /// artifact search engines reject — and one the tool's own analyzer flags as
    /// a warning when it finds it on someone else's site.
    #[test]
    fn splits_beyond_the_protocol_url_limit() {
        let pages: Vec<_> = (0..MAX_URLS_PER_SITEMAP + 10)
            .map(|i| page(&format!("https://e.com/p/{i}"), 200, false))
            .collect();

        let files = generate_sitemap_set(&pages, "https://e.com/");
        assert_eq!(files.len(), 3, "index + two urlset files");
        assert_eq!(files[0].filename, "sitemap.xml");
        assert!(files[0].xml.contains("<sitemapindex"));
        assert!(files[0]
            .xml
            .contains("<loc>https://e.com/sitemap-1.xml</loc>"));

        for f in &files[1..] {
            let count = f.xml.matches("<loc>").count();
            assert!(
                count <= MAX_URLS_PER_SITEMAP,
                "{} holds {count} URLs",
                f.filename
            );
        }
        let total: usize = files[1..]
            .iter()
            .map(|f| f.xml.matches("<loc>").count())
            .sum();
        assert_eq!(total, MAX_URLS_PER_SITEMAP + 10, "no URL is dropped");
    }

    #[test]
    fn a_small_crawl_produces_one_file_and_no_index() {
        let pages = vec![page("https://e.com/", 200, false)];
        let files = generate_sitemap_set(&pages, "https://e.com/");
        assert_eq!(files.len(), 1);
        assert!(!files[0].xml.contains("<sitemapindex"));
    }

    /// An empty `<urlset>` is invalid against the official schema, which requires
    /// at least one `<url>`. Returning no files lets the caller say "no indexable
    /// pages" instead of writing a file no search engine accepts.
    #[test]
    fn no_indexable_pages_produces_no_file() {
        let pages = vec![page("https://e.com/gone", 404, false)];
        assert!(indexable_urls(&pages).is_empty());
        assert!(generate_sitemap_set(&pages, "https://e.com/").is_empty());
    }

    /// The schema caps `<loc>` at 2048 characters, and one over-long URL makes
    /// the whole document fail validation.
    #[test]
    fn over_long_urls_are_excluded() {
        let long = format!("https://e.com/{}", "a".repeat(MAX_LOC_LENGTH));
        let pages = vec![
            page(&long, 200, false),
            page("https://e.com/ok", 200, false),
        ];
        let urls = indexable_urls(&pages);
        assert_eq!(urls, vec!["https://e.com/ok".to_string()]);
    }
}
