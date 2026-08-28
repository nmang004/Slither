use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageData {
    pub url: String,
    pub status: u16,
    pub redirect_chain: Option<Vec<RedirectHop>>,
    pub response_time_ms: u64,
    pub content_type: Option<String>,
    pub depth: u32,
    pub title: Option<String>,
    pub meta_description: Option<String>,
    pub meta_robots: Option<String>,
    pub canonical: Option<String>,
    pub h1: Vec<String>,
    pub headings: Vec<Heading>,
    pub word_count: u32,
    pub body_text: String,
    pub internal_links: Vec<LinkData>,
    pub external_links: Vec<LinkData>,
    pub images: Vec<ImageData>,
    pub schema_types: Vec<String>,
    pub og_tags: HashMap<String, String>,
    pub content_hash: String,

    // Security
    #[serde(default)]
    pub is_https: bool,
    #[serde(default)]
    pub security_headers: SecurityHeaders,
    #[serde(default)]
    pub mixed_content: Vec<String>,
    #[serde(default)]
    pub insecure_forms: Vec<String>,

    // URL metadata
    #[serde(default)]
    pub url_length: u32,
    #[serde(default)]
    pub has_parameters: bool,
    #[serde(default)]
    pub has_underscores: bool,
    #[serde(default)]
    pub has_uppercase: bool,
    #[serde(default)]
    pub has_non_ascii: bool,
    #[serde(default)]
    pub has_multiple_slashes: bool,
    #[serde(default)]
    pub has_repetitive_path: bool,

    // Expanded title/meta
    #[serde(default)]
    pub title_length: Option<u32>,
    #[serde(default)]
    pub title_pixel_width: Option<u32>,
    #[serde(default)]
    pub meta_description_length: Option<u32>,
    #[serde(default)]
    pub meta_description_pixel_width: Option<u32>,
    #[serde(default)]
    pub title_count: u32,
    #[serde(default)]
    pub meta_description_count: u32,
    #[serde(default)]
    pub title_in_head: bool,
    #[serde(default)]
    pub meta_desc_in_head: bool,

    // Canonicals expanded
    #[serde(default)]
    pub canonical_is_relative: bool,
    #[serde(default)]
    pub canonical_count: u32,
    #[serde(default)]
    pub canonical_source: Option<CanonicalSource>,
    #[serde(default)]
    pub has_self_canonical: bool,

    // Directives
    #[serde(default)]
    pub x_robots_tag: Option<String>,
    #[serde(default)]
    pub meta_robots_directives: Vec<String>,

    // Hreflang
    #[serde(default)]
    pub hreflang_tags: Vec<HreflangTag>,

    // Pagination
    #[serde(default)]
    pub pagination: Option<PaginationData>,

    // Content analysis
    #[serde(default)]
    pub readability_score: Option<f32>,
    #[serde(default)]
    pub is_soft_404: bool,

    // Structured data expanded
    #[serde(default)]
    pub structured_data: Vec<StructuredDataBlock>,

    // Link metadata
    #[serde(default)]
    pub unsafe_cross_origin_links: Vec<String>,

    // Core Web Vitals (populated by --pagespeed)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcp_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inp_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cls: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fcp_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttfb_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performance_score: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwv_status: Option<String>,

    // JavaScript audit (populated by rendering backends)
    #[serde(default)]
    pub js_injected_title: bool,
    #[serde(default)]
    pub js_injected_description: bool,
    #[serde(default)]
    pub js_injected_canonical: bool,
    #[serde(default)]
    pub js_injected_h1: bool,
    #[serde(default)]
    pub js_injected_structured_data: bool,
    #[serde(default)]
    pub console_errors: Vec<String>,
    #[serde(default)]
    pub scripts: Vec<ScriptData>,
}

/// True if a `Content-Type` denotes an HTML document.
///
/// Media types are case-insensitive (RFC 9110 §8.3.1). Three copies of this
/// predicate had drifted apart: the crawler's parse gate matched `text/html`
/// case-sensitively, so `TEXT/HTML` was never parsed but was still classified as
/// HTML — producing Missing Title, Missing H1, Missing Canonical and three more
/// findings that were all false, and dead-ending the crawl because links were
/// never extracted.
pub fn is_html_content_type(content_type: Option<&str>) -> bool {
    match content_type {
        Some(ct) => {
            let ct = ct.to_ascii_lowercase();
            ct.contains("text/html") || ct.contains("application/xhtml+xml")
        }
        None => false,
    }
}

/// True if the body looks like HTML, used when a server sends no `Content-Type`
/// at all. Browsers sniff in that case rather than refusing to render.
pub fn body_looks_like_html(body: &str) -> bool {
    let head = body.trim_start().get(..512).unwrap_or(body.trim_start());
    let head = head.to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html") || head.contains("<head")
}

impl PageData {
    /// True if this page is a real, indexable HTML document — a 2xx response
    /// whose content type is HTML (or unknown, which the crawler only records
    /// for pages it parsed as HTML). Presence checks (missing title/description/
    /// H1/canonical) should gate on this so non-HTML assets (PDFs, images, JSON)
    /// and error pages (404/5xx/timeouts) are not flagged for missing SEO tags.
    pub fn is_html_page(&self) -> bool {
        if !(200..300).contains(&self.status) {
            return false;
        }
        match &self.content_type {
            Some(ct) => is_html_content_type(Some(ct)),
            None => false,
        }
    }

    /// The union of robots directives from the `<meta name="robots">` tag and
    /// the `X-Robots-Tag` HTTP header (search engines apply both), lowercased.
    /// A header value may carry a `<ua>:` prefix (e.g. `googlebot: noindex`),
    /// which is stripped so the directive token is captured. Directives that
    /// legitimately contain a colon (`max-snippet:50`) are kept whole.
    pub fn effective_robots_directives(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .meta_robots_directives
            .iter()
            .map(|d| d.trim().to_ascii_lowercase())
            .collect();
        if let Some(header) = &self.x_robots_tag {
            for part in header.split(',') {
                let token = strip_user_agent_prefix(part.trim());
                if !token.is_empty() {
                    out.push(token.to_ascii_lowercase());
                }
            }
        }
        out
    }

    /// True if the page is marked noindex via meta or header.
    pub fn is_noindex(&self) -> bool {
        self.effective_robots_directives()
            .iter()
            .any(|d| d == "noindex" || d == "none")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub src: Option<String>,
    #[serde(default)]
    pub is_async: bool,
    #[serde(default)]
    pub is_defer: bool,
    #[serde(default)]
    pub is_module: bool,
    #[serde(default)]
    pub in_head: bool,
    #[serde(default)]
    pub size_bytes: u32,
    #[serde(default)]
    pub is_third_party: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedirectHop {
    pub status: u16,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heading {
    pub level: u8,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkData {
    pub url: String,
    pub anchor: String,
    pub nofollow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageData {
    pub src: String,
    pub alt: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl ImageData {
    /// True if this image carries meaning a reader would miss without it, and
    /// therefore needs alt text.
    ///
    /// Spacer GIFs, lazy-load placeholders and analytics pixels are inlined as
    /// `data:` URIs or sized 1×1. They are decorative by definition — the
    /// correct markup is `alt=""`, not a description — so counting them as
    /// "missing alt text" buries the real content images. Measured on a
    /// 1,815-page site: 13,067 of 17,064 flagged images (76%) were these.
    pub fn needs_alt_text(&self) -> bool {
        if self
            .src
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("data:")
        {
            return false;
        }
        // A 1×1 (or zero-sized) image is a tracking pixel or spacer.
        matches!((self.width, self.height), (Some(w), Some(h)) if w > 1 && h > 1)
            || self.width.is_none()
            || self.height.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SecurityHeaders {
    pub has_hsts: bool,
    pub has_csp: bool,
    pub has_x_content_type_options: bool,
    pub has_x_frame_options: bool,
    pub has_referrer_policy: bool,
    pub referrer_policy_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HreflangTag {
    pub lang: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationData {
    pub prev: Option<String>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredDataBlock {
    pub format: StructuredDataFormat,
    pub schema_type: Option<String>,
    pub raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    #[serde(default)]
    pub missing_required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredDataFormat {
    JsonLd,
    Microdata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalSource {
    Html,
    HttpHeader,
    Both,
}

#[cfg(test)]
mod robots_directive_tests {
    use super::PageData;

    fn page_with(meta: Vec<&str>, header: Option<&str>) -> PageData {
        let json = r#"{"url":"https://e.com/","status":200,"response_time_ms":0,"depth":0,
            "h1":[],"headings":[],"word_count":0,"body_text":"","internal_links":[],
            "external_links":[],"images":[],"schema_types":[],"og_tags":{},"content_hash":"",
            "is_https":true,"security_headers":{"has_hsts":false,"has_csp":false,
            "has_x_content_type_options":false,"has_x_frame_options":false,"has_referrer_policy":false},
            "mixed_content":[],"insecure_forms":[],"url_length":0,"has_parameters":false,
            "has_underscores":false,"has_uppercase":false,"has_non_ascii":false,
            "has_multiple_slashes":false,"has_repetitive_path":false,"title_count":0,
            "meta_description_count":0,"title_in_head":false,"meta_desc_in_head":false,
            "canonical_is_relative":false,"canonical_count":0,"has_self_canonical":false,
            "meta_robots_directives":[],"hreflang_tags":[],"is_soft_404":false,
            "structured_data":[],"unsafe_cross_origin_links":[],"js_injected_title":false,
            "js_injected_description":false,"js_injected_canonical":false,"js_injected_h1":false,
            "js_injected_structured_data":false,"console_errors":[],"scripts":[]}"#;
        let mut p: PageData = serde_json::from_str(json).unwrap();
        p.meta_robots_directives = meta.into_iter().map(String::from).collect();
        p.x_robots_tag = header.map(String::from);
        p
    }

    #[test]
    fn header_noindex_is_detected() {
        let p = page_with(vec![], Some("noindex, nofollow"));
        assert!(p.is_noindex());
    }

    #[test]
    fn header_with_ua_prefix() {
        let p = page_with(vec![], Some("googlebot: noindex"));
        assert!(p.is_noindex());
    }

    #[test]
    fn merges_meta_and_header() {
        let p = page_with(vec!["nofollow"], Some("noindex"));
        let d = p.effective_robots_directives();
        assert!(d.contains(&"nofollow".to_string()));
        assert!(d.contains(&"noindex".to_string()));
    }

    #[test]
    fn indexable_when_no_directives() {
        let p = page_with(vec![], None);
        assert!(!p.is_noindex());
    }
}

/// Strip a leading `<user-agent>:` from one `X-Robots-Tag` directive.
///
/// The header is either `<directives>` or `<user-agent>: <directives>`, and
/// some directives carry their own colon-separated value. Splitting on the last
/// colon turned `max-snippet:50` into `50`, so only strip the prefix when the
/// text before the first colon is not itself a valued directive.
fn strip_user_agent_prefix(part: &str) -> &str {
    const VALUED_DIRECTIVES: [&str; 4] = [
        "max-snippet",
        "max-image-preview",
        "max-video-preview",
        "unavailable_after",
    ];

    match part.split_once(':') {
        Some((head, tail)) => {
            let head_norm = head.trim().to_ascii_lowercase();
            if VALUED_DIRECTIVES.contains(&head_norm.as_str()) {
                part
            } else {
                tail.trim()
            }
        }
        None => part,
    }
}

#[cfg(test)]
mod x_robots_tests {
    use super::*;

    fn page_with_header(header: &str) -> PageData {
        let json = format!(
            r#"{{"url":"https://e.com/","status":200,"redirect_chain":null,
                "response_time_ms":1,"content_type":"text/html","depth":0,
                "title":null,"meta_description":null,"meta_robots":null,
                "canonical":null,"h1":[],"headings":[],"word_count":0,
                "body_text":"","internal_links":[],"external_links":[],
                "images":[],"schema_types":[],"og_tags":{{}},"content_hash":"",
                "x_robots_tag":"{header}"}}"#
        );
        serde_json::from_str(&json).expect("fixture should parse")
    }

    #[test]
    fn strips_a_user_agent_prefix() {
        let p = page_with_header("googlebot: noindex");
        assert!(p.effective_robots_directives().contains(&"noindex".into()));
        assert!(p.is_noindex());
    }

    /// Regression: splitting on the last colon reduced `max-snippet:50` to
    /// `50`, polluting the directive list shown in issue details.
    #[test]
    fn keeps_valued_directives_whole() {
        let p = page_with_header("max-snippet:50, max-image-preview:large");
        let d = p.effective_robots_directives();
        assert!(d.contains(&"max-snippet:50".to_string()), "got {d:?}");
        assert!(
            d.contains(&"max-image-preview:large".to_string()),
            "got {d:?}"
        );
    }

    #[test]
    fn handles_a_bare_directive_list() {
        let p = page_with_header("noindex, nofollow");
        let d = p.effective_robots_directives();
        assert!(d.contains(&"noindex".to_string()));
        assert!(d.contains(&"nofollow".to_string()));
    }

    #[test]
    fn valued_directive_with_a_ua_prefix_keeps_its_value() {
        assert_eq!(strip_user_agent_prefix("max-snippet:50"), "max-snippet:50");
        assert_eq!(strip_user_agent_prefix("googlebot: noindex"), "noindex");
        assert_eq!(strip_user_agent_prefix("noindex"), "noindex");
    }
}

#[cfg(test)]
mod content_type_tests {
    use super::{body_looks_like_html, is_html_content_type};

    /// Regression: media types are case-insensitive (RFC 9110 §8.3.1). The
    /// crawler's parse gate matched `text/html` case-sensitively, so `TEXT/HTML`
    /// was classified as HTML but never parsed — producing six false "missing
    /// element" findings and dead-ending the crawl at one page.
    #[test]
    fn html_content_types_are_matched_case_insensitively() {
        assert!(is_html_content_type(Some("text/html")));
        assert!(is_html_content_type(Some("TEXT/HTML; charset=UTF-8")));
        assert!(is_html_content_type(Some("Text/Html")));
        assert!(is_html_content_type(Some("application/xhtml+xml")));
        assert!(!is_html_content_type(Some("application/pdf")));
        assert!(!is_html_content_type(Some("image/png")));
        assert!(!is_html_content_type(None));
    }

    /// A server sending no Content-Type at all should still have its HTML read,
    /// as a browser would.
    #[test]
    fn a_missing_content_type_falls_back_to_sniffing() {
        assert!(body_looks_like_html("<!DOCTYPE html><html><head>"));
        assert!(body_looks_like_html("  <html lang=\"en\">"));
        assert!(!body_looks_like_html("%PDF-1.7 binary junk"));
        assert!(!body_looks_like_html(""));
    }
}

#[cfg(test)]
mod image_alt_tests {
    use super::ImageData;

    fn img(src: &str, w: Option<u32>, h: Option<u32>) -> ImageData {
        ImageData {
            src: src.to_string(),
            alt: None,
            width: w,
            height: h,
        }
    }

    /// Regression: spacer GIFs and analytics pixels were counted as "missing alt
    /// text". On a 1,815-page site 13,067 of 17,064 flagged images (76%) were
    /// these, burying the ~4,000 real content images.
    #[test]
    fn decorative_images_do_not_need_alt_text() {
        assert!(!img("data:image/gif;base64,R0lGODlhAQABAIAAAA", None, None).needs_alt_text());
        assert!(!img("DATA:image/png;base64,iVBOR", None, None).needs_alt_text());
        assert!(!img("https://x.test/pixel.gif", Some(1), Some(1)).needs_alt_text());
    }

    #[test]
    fn content_images_still_need_alt_text() {
        assert!(img("https://x.test/photo.jpg", Some(800), Some(600)).needs_alt_text());
        // Dimensions are usually absent in markup; those must still be checked.
        assert!(img("https://x.test/photo.jpg", None, None).needs_alt_text());
    }
}
