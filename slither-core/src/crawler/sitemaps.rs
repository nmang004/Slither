use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::{debug, warn};

use super::fetcher::Fetcher;

/// Maximum recursion depth for sitemap indexes.
const MAX_INDEX_DEPTH: u32 = 2;
/// Maximum total URLs to collect from sitemaps.
const MAX_TOTAL_URLS: usize = 50_000;

/// Aggregated sitemap data from discovery and parsing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SitemapData {
    pub sitemap_urls: Vec<SitemapEntry>,
    pub sitemap_sources: Vec<SitemapSource>,
    pub errors: Vec<String>,
    /// Collection stopped at [`MAX_TOTAL_URLS`], so `sitemap_urls` is a partial
    /// view. Coverage checks must not treat it as the complete set — a site
    /// whose first child sitemap fills the cap would otherwise have every
    /// crawled page reported as "not in sitemap".
    #[serde(default)]
    pub truncated: bool,
    /// Sitemaps that were reached but could not be parsed. Search Console
    /// reports these as "Sitemap could not be read", so a partial read must not
    /// be presented as a healthy source.
    #[serde(default)]
    pub malformed: Vec<String>,
    /// Sitemaps that were declared (usually in robots.txt) but did not return a
    /// usable response. "Your robots.txt points at a dead sitemap" is a
    /// different defect from "you have no sitemap".
    #[serde(default)]
    pub unreachable: Vec<String>,
    /// `<loc>` values that are not absolute URLs, which the protocol requires.
    #[serde(default)]
    pub invalid_locs: Vec<String>,
}

/// A single URL entry discovered from a sitemap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SitemapEntry {
    pub url: String,
    pub source_sitemap: String,
}

/// Metadata about a sitemap source that was fetched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SitemapSource {
    pub url: String,
    pub url_count: u32,
    pub size_bytes: u64,
}

/// Well-known sitemap paths probed when robots.txt declares none, in the order
/// Googlebot is documented to try them.
const WELL_KNOWN_PATHS: [&str; 4] = [
    "/sitemap.xml",
    "/sitemap_index.xml",
    "/sitemaps.xml",
    "/sitemap-index.xml",
];

/// Build the origins to probe from whatever the caller passed as `site`.
///
/// Accepts either a full seed URL or a bare host. Given a URL the origin is used
/// verbatim, preserving scheme and port — building `https://{host}` from a bare
/// host meant an http-only site, or any site on a non-default port, probed an
/// address that does not exist and was reported as having no sitemap at all.
fn discovery_origins(site: &str) -> Vec<String> {
    if let Ok(parsed) = url::Url::parse(site) {
        if parsed.has_host() && matches!(parsed.scheme(), "http" | "https") {
            let mut origin = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
            if let Some(port) = parsed.port() {
                origin.push_str(&format!(":{port}"));
            }
            return vec![origin];
        }
    }
    // A bare host carries no scheme, so try the secure one first and fall back.
    vec![format!("https://{site}"), format!("http://{site}")]
}

/// Fetches and parses XML sitemaps for a domain.
pub struct SitemapFetcher<'a> {
    fetcher: &'a Fetcher,
}

impl<'a> SitemapFetcher<'a> {
    pub fn new(fetcher: &'a Fetcher) -> Self {
        Self { fetcher }
    }

    /// Discover and fetch all sitemaps for a site.
    ///
    /// `site` may be a full seed URL (preferred — its scheme and port are kept)
    /// or a bare host.
    ///
    /// Discovery chain:
    /// 1. Sitemaps declared in robots.txt.
    /// 2. If those yielded nothing usable, the well-known paths — Googlebot
    ///    probes `/sitemap.xml` regardless of robots.txt, so a broken robots
    ///    reference must not suppress discovery of a sitemap that is really
    ///    there.
    pub async fn fetch_all(&self, site: &str, robots_sitemaps: &[String]) -> SitemapData {
        let mut data = SitemapData::default();
        let mut visited: HashSet<String> = HashSet::new();

        if !robots_sitemaps.is_empty() {
            debug!(
                "Found {} sitemap(s) in robots.txt for {}",
                robots_sitemaps.len(),
                site
            );
            for sitemap_url in robots_sitemaps {
                self.fetch_sitemap(sitemap_url, &mut data, 0, &mut visited, true)
                    .await;
                if data.sitemap_urls.len() >= MAX_TOTAL_URLS {
                    break;
                }
            }
            if !data.sitemap_sources.is_empty() {
                return data;
            }
            debug!("robots.txt sitemaps yielded nothing usable; probing well-known paths");
        }

        for origin in discovery_origins(site) {
            for path in WELL_KNOWN_PATHS {
                let candidate = format!("{origin}{path}");
                if visited.contains(&candidate) {
                    continue;
                }
                debug!("Trying sitemap candidate: {}", candidate);
                let before = data.sitemap_sources.len();
                self.fetch_sitemap(&candidate, &mut data, 0, &mut visited, false)
                    .await;
                if data.sitemap_sources.len() > before {
                    return data;
                }
            }
        }

        debug!("No sitemaps found for {}", site);
        data
    }

    /// Fetch and process a single sitemap URL.
    ///
    /// `visited` guards against a sitemap index that references itself: without
    /// it the walk re-fetched and re-appended the same file at every level up to
    /// `MAX_INDEX_DEPTH`, triple-counting sources and duplicating URLs.
    ///
    /// `declared` marks a sitemap the site told us about (robots.txt, or a child
    /// of an index) as opposed to a speculative probe of a well-known path. Only
    /// a declared sitemap failing is a defect — a 404 on `/sitemap_index.xml`
    /// for a site that only publishes `/sitemap.xml` is the normal case.
    fn fetch_sitemap<'b>(
        &'b self,
        url: &'b str,
        data: &'b mut SitemapData,
        depth: u32,
        visited: &'b mut HashSet<String>,
        declared: bool,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'b>> {
        Box::pin(async move {
            if depth > MAX_INDEX_DEPTH {
                warn!(
                    "Sitemap index depth {} exceeds max {}, skipping {}",
                    depth, MAX_INDEX_DEPTH, url
                );
                data.errors
                    .push(format!("Sitemap index depth exceeded for {}", url));
                return;
            }

            if !visited.insert(url.to_string()) {
                debug!("Sitemap {} already fetched, skipping self-reference", url);
                return;
            }

            if data.sitemap_urls.len() >= MAX_TOTAL_URLS {
                debug!("Reached max URL cap ({}), skipping {}", MAX_TOTAL_URLS, url);
                data.truncated = true;
                return;
            }

            // Follow redirects: a sitemap that 301s to its real location is
            // ordinary (an http:// declaration on an https site, or a moved
            // path), and treating the 3xx as a failure reported the site as
            // having no sitemap at all.
            match self.fetcher.fetch_with_redirects(url, 5).await {
                Ok((result, _chain)) => {
                    if result.status != 200 {
                        debug!("Sitemap {} returned status {}", url, result.status);
                        data.errors
                            .push(format!("Sitemap {} returned status {}", url, result.status));
                        if declared {
                            data.unreachable.push(url.to_string());
                        }
                        return;
                    }

                    self.process_sitemap_response(
                        url,
                        &result.body,
                        data,
                        depth,
                        visited,
                        declared,
                    )
                    .await;
                }
                Err(e) => {
                    warn!("Failed to fetch sitemap {}: {}", url, e);
                    data.errors
                        .push(format!("Failed to fetch sitemap {}: {}", url, e));
                    if declared {
                        data.unreachable.push(url.to_string());
                    }
                }
            }
        })
    }

    /// Process a sitemap response body: detect if it's an index or urlset, parse accordingly.
    async fn process_sitemap_response(
        &self,
        url: &str,
        body: &str,
        data: &mut SitemapData,
        depth: u32,
        visited: &mut HashSet<String>,
        declared: bool,
    ) {
        let size_bytes = body.len() as u64;

        if body.contains("<sitemapindex") {
            // It's a sitemap index — parse child sitemap URLs and fetch them
            let (child_urls, parse_error) = parse_sitemap_index_checked(body);
            debug!(
                "Sitemap index {} contains {} child sitemaps",
                url,
                child_urls.len()
            );
            if let Some(err) = parse_error {
                data.errors
                    .push(format!("Sitemap {} is malformed XML: {}", url, err));
                data.malformed.push(url.to_string());
            }

            data.sitemap_sources.push(SitemapSource {
                url: url.to_string(),
                url_count: child_urls.len() as u32,
                size_bytes,
            });

            for child_url in child_urls {
                if data.sitemap_urls.len() >= MAX_TOTAL_URLS {
                    data.truncated = true;
                    break;
                }
                self.fetch_sitemap(&child_url, data, depth + 1, visited, true)
                    .await;
            }
        } else if body.contains("<urlset") {
            // It's a URL set — extract URLs
            let (urls, parse_error) = parse_urlset_checked(body);
            debug!("Sitemap {} contains {} URLs", url, urls.len());
            if let Some(err) = parse_error {
                data.errors
                    .push(format!("Sitemap {} is malformed XML: {}", url, err));
                data.malformed.push(url.to_string());
            }

            let url_count = urls.len() as u32;

            for entry_url in urls {
                if data.sitemap_urls.len() >= MAX_TOTAL_URLS {
                    data.truncated = true;
                    break;
                }
                // The protocol requires a fully-qualified URL. A relative
                // `<loc>` was stored raw and then reported as a crawled page
                // "missing from the sitemap" — telling the user to add a page
                // that is already listed, in a broken form.
                if !is_absolute_http_url(&entry_url) {
                    data.invalid_locs.push(entry_url);
                    continue;
                }
                data.sitemap_urls.push(SitemapEntry {
                    url: entry_url,
                    source_sitemap: url.to_string(),
                });
            }

            data.sitemap_sources.push(SitemapSource {
                url: url.to_string(),
                url_count,
                size_bytes,
            });
        } else {
            debug!("Sitemap {} has unrecognized format", url);
            data.errors.push(format!(
                "Sitemap {} has unrecognized format (not sitemapindex or urlset)",
                url
            ));
            if declared {
                data.unreachable.push(url.to_string());
            }
        }
    }
}

/// True if `loc` is the fully-qualified http(s) URL the sitemap protocol
/// requires.
fn is_absolute_http_url(loc: &str) -> bool {
    url::Url::parse(loc)
        .map(|u| u.has_host() && matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false)
}

/// Parse a sitemap index XML and extract child `<sitemap><loc>` URLs.
pub fn parse_sitemap_index(xml: &str) -> Vec<String> {
    extract_locs(xml, b"sitemap").0
}

/// Parse a sitemap urlset XML and extract `<url><loc>` URLs.
pub fn parse_urlset(xml: &str) -> Vec<String> {
    extract_locs(xml, b"url").0
}

/// As [`parse_sitemap_index`], also returning the XML error that stopped the
/// parse, if any.
pub fn parse_sitemap_index_checked(xml: &str) -> (Vec<String>, Option<String>) {
    extract_locs(xml, b"sitemap")
}

/// As [`parse_urlset`], also returning the XML error that stopped the parse.
///
/// A truncated sitemap previously returned whatever had been read so far and
/// was recorded as a healthy source, so the site's real defect ("Sitemap could
/// not be read" in Search Console) was invisible and the pages listed after the
/// truncation were reported as missing from the sitemap.
pub fn parse_urlset_checked(xml: &str) -> (Vec<String>, Option<String>) {
    extract_locs(xml, b"url")
}

/// Extract `<loc>` values nested under `parent` (`url` or `sitemap`).
///
/// Content is accumulated across events until `</loc>`: quick-xml emits text,
/// CDATA, and entity references (`&amp;` etc.) as separate events, so pushing
/// per-event would split a URL like `...?x=1&y=2` into two.
fn extract_locs(xml: &str, parent: &[u8]) -> (Vec<String>, Option<String>) {
    let mut urls = Vec::new();
    let mut parse_error: Option<String> = None;
    let mut reader = Reader::from_str(xml);
    // A sitemap truncated mid-element does not raise a parse error — quick-xml
    // simply reports EOF — so a partial file was presented as a healthy source.
    // Track nesting instead: elements still open at EOF mean the document was
    // cut short.
    reader.config_mut().check_end_names = true;
    let mut depth: i32 = 0;
    let mut in_parent = false;
    let mut in_loc = false;
    let mut current = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                match e.local_name().as_ref() {
                    name if name == parent => in_parent = true,
                    b"loc" if in_parent => {
                        in_loc = true;
                        current.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                depth -= 1;
                match e.local_name().as_ref() {
                    name if name == parent => {
                        in_parent = false;
                        in_loc = false;
                    }
                    b"loc" => {
                        let trimmed = current.trim();
                        if !trimmed.is_empty() {
                            urls.push(trimmed.to_string());
                        }
                        in_loc = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) if in_loc => {
                if let Ok(text) = e.xml_content(quick_xml::XmlVersion::Implicit1_0) {
                    current.push_str(&text);
                }
            }
            Ok(Event::CData(ref e)) if in_loc => {
                if let Ok(text) = e.decode() {
                    current.push_str(&text);
                }
            }
            // Entity references (`&amp;`, `&lt;`, …) inside a <loc> arrive as
            // their own events in quick-xml 0.41, carrying the entity name.
            Ok(Event::GeneralRef(ref e)) if in_loc => {
                if let Ok(name) = e.decode() {
                    let name = name.as_ref();
                    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
                        if let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                        {
                            current.push(ch);
                        }
                    } else if let Some(dec) = name.strip_prefix('#') {
                        if let Some(ch) = dec.parse::<u32>().ok().and_then(char::from_u32) {
                            current.push(ch);
                        }
                    } else {
                        match name {
                            "amp" => current.push('&'),
                            "lt" => current.push('<'),
                            "gt" => current.push('>'),
                            "quot" => current.push('"'),
                            "apos" => current.push('\''),
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::Eof) => {
                if depth > 0 && parse_error.is_none() {
                    parse_error = Some(format!(
                        "unexpected end of document ({depth} element(s) unclosed)"
                    ));
                }
                break;
            }
            Err(e) => {
                warn!("Error parsing sitemap XML: {}", e);
                parse_error = Some(e.to_string());
                break;
            }
            _ => {}
        }
        buf.clear();
    }

    (urls, parse_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_urlset() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>https://example.com/page1</loc>
    <lastmod>2024-01-01</lastmod>
  </url>
  <url>
    <loc>https://example.com/page2</loc>
  </url>
  <url>
    <loc>https://example.com/page3</loc>
    <priority>0.8</priority>
  </url>
</urlset>"#;

        let urls = parse_urlset(xml);
        assert_eq!(urls.len(), 3);
        assert_eq!(urls[0], "https://example.com/page1");
        assert_eq!(urls[1], "https://example.com/page2");
        assert_eq!(urls[2], "https://example.com/page3");
    }

    #[test]
    fn test_parse_sitemap_index() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap>
    <loc>https://example.com/sitemap-posts.xml</loc>
    <lastmod>2024-01-01</lastmod>
  </sitemap>
  <sitemap>
    <loc>https://example.com/sitemap-pages.xml</loc>
  </sitemap>
</sitemapindex>"#;

        let urls = parse_sitemap_index(xml);
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "https://example.com/sitemap-posts.xml");
        assert_eq!(urls[1], "https://example.com/sitemap-pages.xml");
    }

    #[test]
    fn test_parse_empty_urlset() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
</urlset>"#;

        let urls = parse_urlset(xml);
        assert!(urls.is_empty());
    }

    #[test]
    fn test_parse_empty_sitemap_index() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
</sitemapindex>"#;

        let urls = parse_sitemap_index(xml);
        assert!(urls.is_empty());
    }

    #[test]
    fn test_parse_urlset_with_whitespace() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>
      https://example.com/page1
    </loc>
  </url>
</urlset>"#;

        let urls = parse_urlset(xml);
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0], "https://example.com/page1");
    }

    #[test]
    fn test_parse_urlset_cdata_loc() {
        // <loc> values wrapped in CDATA must be extracted (were dropped before).
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc><![CDATA[https://example.com/a?x=1&y=2]]></loc></url>
</urlset>"#;
        let urls = parse_urlset(xml);
        assert_eq!(urls, vec!["https://example.com/a?x=1&y=2".to_string()]);
    }

    #[test]
    fn test_parse_urlset_entity_escaped_loc() {
        // &amp; in a <loc> must be unescaped to &.
        let xml = r#"<urlset><url><loc>https://example.com/a?x=1&amp;y=2</loc></url></urlset>"#;
        let urls = parse_urlset(xml);
        assert_eq!(urls, vec!["https://example.com/a?x=1&y=2".to_string()]);
    }

    #[test]
    fn test_parse_invalid_xml() {
        let xml = "this is not xml at all";
        let urls = parse_urlset(xml);
        assert!(urls.is_empty());

        let urls = parse_sitemap_index(xml);
        assert!(urls.is_empty());
    }

    #[test]
    fn test_sitemap_data_default() {
        let data = SitemapData::default();
        assert!(data.sitemap_urls.is_empty());
        assert!(data.sitemap_sources.is_empty());
        assert!(data.errors.is_empty());
        assert!(!data.truncated);
        assert!(data.malformed.is_empty());
        assert!(data.unreachable.is_empty());
    }

    /// A truncated sitemap returned whatever had been read and was recorded as
    /// healthy, so the real defect was invisible and the pages listed after the
    /// break were reported as missing from the sitemap.
    #[test]
    fn truncated_xml_reports_a_parse_error() {
        let xml = "<urlset><url><loc>https://e.com/a</loc></url><url><loc>https://e.com/b";
        let (urls, err) = parse_urlset_checked(xml);
        assert_eq!(urls, vec!["https://e.com/a".to_string()]);
        assert!(err.is_some(), "the truncation must be reported");
    }

    #[test]
    fn well_formed_xml_reports_no_parse_error() {
        let xml = r#"<urlset><url><loc>https://e.com/a</loc></url></urlset>"#;
        let (urls, err) = parse_urlset_checked(xml);
        assert_eq!(urls.len(), 1);
        assert!(err.is_none());
    }

    /// Discovery built `https://{host}` from a bare host, so an http-only site
    /// or one on a non-default port probed an address that does not exist and
    /// was reported as having no sitemap.
    #[test]
    fn discovery_keeps_the_seed_scheme_and_port() {
        assert_eq!(
            discovery_origins("http://127.0.0.1:9194/some/page"),
            vec!["http://127.0.0.1:9194".to_string()]
        );
        assert_eq!(
            discovery_origins("https://example.com/"),
            vec!["https://example.com".to_string()]
        );
    }

    #[test]
    fn a_bare_host_falls_back_from_https_to_http() {
        assert_eq!(
            discovery_origins("example.com"),
            vec![
                "https://example.com".to_string(),
                "http://example.com".to_string()
            ]
        );
    }

    #[test]
    fn only_absolute_http_locs_are_accepted() {
        assert!(is_absolute_http_url("https://example.com/a"));
        assert!(is_absolute_http_url("http://example.com/a"));
        assert!(!is_absolute_http_url("/relative.html"));
        assert!(!is_absolute_http_url("example.com/a"));
        assert!(!is_absolute_http_url("ftp://example.com/a"));
    }
}
