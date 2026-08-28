use crate::models::page::{
    CanonicalSource, Heading, HreflangTag, ImageData, LinkData, PageData, PaginationData,
    ScriptData, SecurityHeaders, StructuredDataBlock, StructuredDataFormat,
};
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use url::Url;

/// True if the element sits inside a `<template>`.
///
/// Template contents are an inert `DocumentFragment`: a browser does not render
/// them, follow their links, count their images, or apply their meta tags. They
/// are the standard client-side templating idiom (Vue, Alpine, HTMX, web
/// components), and `scraper` walks the whole node arena, so an unscoped
/// selector treats a `<a href="/product/{{slug}}">` placeholder as a real link —
/// which the crawler then fetches, 404s on, and reports as the site's own broken
/// link.
fn is_in_template(el: &scraper::ElementRef) -> bool {
    el.ancestors().any(|node| {
        node.value()
            .as_element()
            .is_some_and(|e| e.name() == "template")
    })
}

/// True when the element sits inside an inline `<svg>`.
///
/// An SVG logo carries its own `<title>` as an accessibility label. That is not
/// the document title, and excluding it is the real job that scoping title
/// selection to `<head>` used to do.
fn is_in_svg(el: &scraper::ElementRef) -> bool {
    el.ancestors()
        .any(|node| node.value().as_element().is_some_and(|e| e.name() == "svg"))
}

/// True when the element sits inside `<head>`.
fn is_in_head(el: &scraper::ElementRef) -> bool {
    el.ancestors().any(|node| {
        node.value()
            .as_element()
            .is_some_and(|e| e.name() == "head")
    })
}

/// The document's `<title>` elements in document order.
///
/// Deliberately not scoped to `<head>`. Anything the HTML spec does not allow
/// in the head — a tracking pixel `<img>`, a stray text node, a `<div>` from a
/// mis-templated partial — closes the head early, and every following element,
/// `<title>` included, is parsed into `<body>`. Scoping the selector to
/// `head title` therefore reported "Missing Title" for pages whose markup
/// plainly carries one, and left `page_titles::check_outside_head` unreachable,
/// since it requires a title to be present in order to report it as misplaced.
fn document_titles<'a>(
    document: &'a Html,
    selector: &'a Selector,
) -> impl Iterator<Item = scraper::ElementRef<'a>> + 'a {
    document
        .select(selector)
        .filter(|el| !is_in_template(el) && !is_in_svg(el))
}

/// `Html::select`, restricted to elements a browser would actually render.
fn select_live<'a>(
    document: &'a Html,
    selector: &'a Selector,
) -> impl Iterator<Item = scraper::ElementRef<'a>> + 'a {
    document.select(selector).filter(|el| !is_in_template(el))
}

/// Collapse ASCII whitespace runs into single spaces, as HTML rendering does.
///
/// Everything the SERP shows — `document.title`, the snippet, anchor text — is
/// whitespace-collapsed by the browser, so a title that the template wrapped
/// across three indented source lines renders 12 characters shorter than its
/// raw text node. Measuring the raw form produced false "Over 60 Characters" and
/// "Over 561px Wide" findings, and let duplicate titles escape detection because
/// the map was keyed on the un-collapsed string.
fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Text content of an element as a browser would render it.
fn rendered_text(el: &scraper::ElementRef) -> String {
    collapse_ws(&el.text().collect::<String>())
}

/// True for elements whose text a browser never displays as prose.
fn is_inert_text_container(name: &str) -> bool {
    matches!(name, "script" | "style" | "noscript" | "template")
}

/// Rendered text of an element, excluding inert subtrees.
///
/// `ElementRef::text()` concatenates every descendant text node, and a
/// `<noscript>` inside a heading is one raw text node holding its markup — so a
/// 61-character headline was measured at 819 characters and reported as an
/// over-length H2.
fn visible_text(el: &scraper::ElementRef) -> String {
    let mut parts: Vec<String> = Vec::new();
    for node in el.descendants() {
        let Some(text) = node.value().as_text() else {
            continue;
        };
        let mut inert = false;
        let mut cursor = node.parent();
        while let Some(parent) = cursor {
            if parent.id() == el.id() {
                break;
            }
            if parent
                .value()
                .as_element()
                .is_some_and(|e| is_inert_text_container(e.name()))
            {
                inert = true;
                break;
            }
            cursor = parent.parent();
        }
        if !inert {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }
    collapse_ws(&parts.join(" "))
}

/// Parse an HTML document and extract all SEO-relevant data.
pub fn parse_html(html: &str, page_url: &str) -> PageData {
    let document = Html::parse_document(html);
    let page_base = Url::parse(page_url).ok();
    // Internal/external classification is always relative to the page's own host,
    // even when a <base href> points elsewhere (e.g. a CDN).
    let base_host = page_base
        .as_ref()
        .and_then(|u| u.host_str().map(String::from));
    // Relative URLs (links, images, hreflang, pagination) resolve against
    // <base href> when present, per the HTML spec — not the page URL.
    let resolution_base = extract_base_href(&document, page_base.as_ref());
    let base_url = resolution_base.or(page_base);

    let title = extract_title(&document);
    let meta_description = extract_meta(&document, "description");
    let meta_robots = extract_robots_meta(&document);
    // Resolve a relative canonical (e.g. href="/page") to an absolute URL so
    // self-canonical and canonical-target checks can match it.
    let canonical_raw = extract_canonical(&document);
    let canonical = canonical_raw.as_deref().map(|c| match base_url.as_ref() {
        Some(base) => base
            .join(c)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| c.to_string()),
        None => c.to_string(),
    });
    let h1 = extract_h1s(&document);
    let headings = extract_headings(&document);
    let (internal_links, external_links) =
        extract_links(&document, base_url.as_ref(), base_host.as_deref());
    let images = extract_images(&document, base_url.as_ref());
    let schema_types = extract_schema_types(&document);
    let og_tags = extract_og_tags(&document);
    let body_text = extract_body_text(&document);
    let word_count = count_words(&body_text);
    let content_hash = compute_hash(&body_text);

    // New extraction functions
    let (title_count, title_in_head, title_length, title_pixel_width) =
        extract_title_metadata(&document, title.as_deref());
    let (
        meta_description_count,
        meta_desc_in_head,
        meta_description_length,
        meta_description_pixel_width,
    ) = extract_meta_description_metadata(&document, meta_description.as_deref());
    let (canonical_count, canonical_is_relative, canonical_in_head, has_self_canonical) =
        extract_canonical_metadata(
            &document,
            canonical_raw.as_deref(),
            canonical.as_deref(),
            page_url,
        );
    let canonical_source = if canonical.is_some() && canonical_in_head {
        Some(CanonicalSource::Html)
    } else {
        None
    };
    let hreflang_tags = extract_hreflang_tags(&document, base_url.as_ref());
    let pagination = extract_pagination(&document, base_url.as_ref());
    let meta_robots_directives = extract_meta_robots_directives(meta_robots.as_deref());
    let is_https = page_url.starts_with("https://");
    let mixed_content = if is_https {
        extract_mixed_content(&document)
    } else {
        Vec::new()
    };
    let insecure_forms = extract_insecure_forms(&document);
    let unsafe_cross_origin_links = extract_unsafe_cross_origin_links(&document);
    let structured_data = extract_structured_data(&document);
    let readability_score = crate::utils::readability::flesch_kincaid_reading_ease(&body_text);
    let is_soft_404 = detect_soft_404(&body_text, title.as_deref());
    let scripts = extract_scripts(&document, base_host.as_deref());

    PageData {
        url: page_url.to_string(),
        status: 0, // Set by caller
        redirect_chain: None,
        response_time_ms: 0, // Set by caller
        content_type: None,  // Set by caller
        depth: 0,            // Set by caller
        title,
        meta_description,
        meta_robots,
        canonical,
        h1,
        headings,
        word_count,
        body_text,
        internal_links,
        external_links,
        images,
        schema_types,
        og_tags,
        content_hash,
        is_https,
        security_headers: SecurityHeaders::default(),
        mixed_content,
        insecure_forms,
        url_length: 0,
        has_parameters: false,
        has_underscores: false,
        has_uppercase: false,
        has_non_ascii: false,
        has_multiple_slashes: false,
        has_repetitive_path: false,
        title_length,
        title_pixel_width,
        meta_description_length,
        meta_description_pixel_width,
        title_count,
        meta_description_count,
        title_in_head,
        meta_desc_in_head,
        canonical_is_relative,
        canonical_count,
        canonical_source,
        has_self_canonical,
        x_robots_tag: None,
        meta_robots_directives,
        hreflang_tags,
        pagination,
        readability_score,
        is_soft_404,
        structured_data,
        unsafe_cross_origin_links,
        lcp_ms: None,
        inp_ms: None,
        cls: None,
        fcp_ms: None,
        ttfb_ms: None,
        performance_score: None,
        cwv_status: None,
        js_injected_title: false,
        js_injected_description: false,
        js_injected_canonical: false,
        js_injected_h1: false,
        js_injected_structured_data: false,
        console_errors: Vec::new(),
        scripts,
    }
}

/// Resolve `<base href>` (the first one in the document) against the page URL.
/// Returns `None` when there is no usable base tag.
fn extract_base_href(document: &Html, page_base: Option<&Url>) -> Option<Url> {
    let selector = Selector::parse("base[href]").ok()?;
    let href = select_live(document, &selector)
        .next()
        .and_then(|el| el.value().attr("href"))?
        .trim();
    if href.is_empty() {
        return None;
    }
    match page_base {
        Some(base) => base.join(href).ok(),
        None => Url::parse(href).ok(),
    }
}

fn extract_title(document: &Html) -> Option<String> {
    // See `document_titles` for why this is not scoped to <head>.
    let selector = Selector::parse("title").ok()?;
    document
        .select(&selector)
        .find(|el| !is_in_template(el) && !is_in_svg(el))
        .map(|el| rendered_text(&el))
}

fn extract_meta(document: &Html, name: &str) -> Option<String> {
    let selector = Selector::parse(&format!("meta[name=\"{}\" i]", name)).ok()?;
    document
        .select(&selector)
        .find(|el| !is_in_template(el))
        .and_then(|el| el.value().attr("content").map(collapse_ws))
}

/// Robots meta names that bind general Google Search indexability.
///
/// `robots` addresses every crawler; `googlebot` is the user agent Google Search
/// itself crawls with, so a `noindex` under either name removes the page from
/// Search.
const GENERAL_ROBOTS_META_NAMES: [&str; 2] = ["robots", "googlebot"];

/// Robots meta names scoped to a single Google surface.
///
/// Google's documentation is explicit that `googlebot-news` applies to Google
/// News only: a `noindex` there drops the page from the News surface while
/// leaving it fully indexable in Search. Folding it into the general rules
/// marked such a page `is_indexable = false`, which gates roughly twenty checks
/// — duplicate-title detection among them simply stopped firing — dropped the
/// page from the generated sitemap, and advised removing a live, Search-
/// indexable article from the client's sitemap.
const SCOPED_ROBOTS_META_NAMES: [&str; 1] = ["googlebot-news"];

/// True if a directive token produced by [`extract_robots_meta`] is scoped to a
/// single Google surface (`googlebot-news: noindex`) rather than binding general
/// Search. Callers reading `meta_robots_directives` for a Search-wide decision
/// must skip these, as `PageData::is_noindex` already does.
pub(crate) fn is_surface_scoped_directive(directive: &str) -> bool {
    match directive.split_once(':') {
        Some((head, _)) => SCOPED_ROBOTS_META_NAMES.contains(&head.trim()),
        None => false,
    }
}

/// The `content` values of every live `<meta name="…">` with this name.
fn robots_meta_contents(document: &Html, name: &str) -> Vec<String> {
    let Ok(selector) = Selector::parse(&format!("meta[name=\"{}\" i]", name)) else {
        return Vec::new();
    };
    select_live(document, &selector)
        .filter_map(|el| el.value().attr("content"))
        .map(collapse_ws)
        .filter(|v| !v.is_empty())
        .collect()
}

/// Combine every robots directive the page declares.
///
/// Google: "For situations where multiple crawlers are specified along with
/// different rules, the search engine will use the sum of the negative rules."
/// Reading only the first `<meta name="robots">` meant a theme's `index,follow`
/// masked an SEO plugin's `noindex` on the very next line, and the page was
/// reported indexable and written into the generated sitemap. Crawler-specific
/// tags such as `<meta name="googlebot" content="noindex">` are equally binding
/// and were not read at all.
///
/// Surface-scoped names ([`SCOPED_ROBOTS_META_NAMES`]) are kept, but each of
/// their tokens is emitted carrying its `<name>:` prefix — the same `ua:
/// directive` shape `X-Robots-Tag` uses, which `PageData::is_noindex` already
/// declines to read as a bare directive. The token stays visible in the CSV
/// export, the HTML report and `meta_robots_directives`; it just no longer
/// decides general Search indexability.
fn extract_robots_meta(document: &Html) -> Option<String> {
    let mut values: Vec<String> = Vec::new();

    for name in GENERAL_ROBOTS_META_NAMES {
        values.extend(robots_meta_contents(document, name));
    }

    for name in SCOPED_ROBOTS_META_NAMES {
        for content in robots_meta_contents(document, name) {
            // Prefix every token, not the value as a whole: the directive list
            // is split on commas downstream, so `googlebot-news: noindex,
            // nosnippet` would leak `nosnippet` as a general directive.
            for token in content.split(',') {
                let token = token.trim();
                if !token.is_empty() {
                    values.push(format!("{name}: {token}"));
                }
            }
        }
    }

    if values.is_empty() {
        None
    } else {
        Some(values.join(", "))
    }
}

fn extract_canonical(document: &Html) -> Option<String> {
    let selector = Selector::parse("link[rel~=\"canonical\"]").ok()?;
    document
        .select(&selector)
        .find(|el| !is_in_template(el))
        .and_then(|el| el.value().attr("href").map(|s| s.to_string()))
}

fn extract_h1s(document: &Html) -> Vec<String> {
    let selector = match Selector::parse("h1") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    select_live(document, &selector)
        .map(|el| visible_text(&el))
        .collect()
}

fn extract_headings(document: &Html) -> Vec<Heading> {
    // Include h1 so the list is the full heading outline in document order,
    // which the heading-sequence analyzer needs. `p.h1` remains the separate
    // convenience list for H1-specific checks.
    let selector = match Selector::parse("h1, h2, h3, h4, h5, h6") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    select_live(document, &selector)
        .filter_map(|el| {
            let tag_name = el.value().name();
            let level: u8 = tag_name.strip_prefix('h')?.parse().ok()?;
            Some(Heading {
                level,
                text: visible_text(&el),
            })
        })
        .collect()
}

/// True for `href` schemes that do not navigate to a crawlable page.
///
/// Scheme comparison is case-insensitive per RFC 3986 §3.1.
fn is_non_navigational_scheme(href: &str) -> bool {
    const SCHEMES: [&str; 6] = ["mailto:", "tel:", "javascript:", "data:", "sms:", "callto:"];
    let lower = href.to_ascii_lowercase();
    SCHEMES.iter().any(|s| lower.starts_with(s))
}

fn extract_links(
    document: &Html,
    base_url: Option<&Url>,
    base_host: Option<&str>,
) -> (Vec<LinkData>, Vec<LinkData>) {
    let selector = match Selector::parse("a[href]") {
        Ok(s) => s,
        Err(_) => return (Vec::new(), Vec::new()),
    };

    let mut internal = Vec::new();
    let mut external = Vec::new();

    for el in select_live(document, &selector) {
        // Browsers strip leading and trailing whitespace from href before
        // resolving it, and URL schemes are case-insensitive, so `MAILTO:` and
        // `  javascript:…  ` are the same non-navigational links as their
        // lowercase, untrimmed forms. Matching them literally let them through
        // as "internal links", inflating link counts and masking the
        // dead-end "No Internal Outlinks" check on a page whose only anchor was
        // a mailto.
        let href = match el.value().attr("href") {
            Some(h) => h.trim(),
            None => continue,
        };

        if href.starts_with('#') || is_non_navigational_scheme(href) {
            continue;
        }

        let resolved = if let Some(base) = base_url {
            match base.join(href) {
                Ok(u) => u.to_string(),
                Err(_) => continue,
            }
        } else {
            href.to_string()
        };

        let anchor = visible_text(&el);
        let nofollow = el
            .value()
            .attr("rel")
            .map(|r| r.contains("nofollow"))
            .unwrap_or(false);

        let link = LinkData {
            url: resolved.clone(),
            anchor,
            nofollow,
        };

        // Determine internal vs external
        let link_host = Url::parse(&resolved)
            .ok()
            .and_then(|u| u.host_str().map(String::from));

        if let (Some(bh), Some(lh)) = (base_host, link_host.as_deref()) {
            if bh == lh {
                internal.push(link);
            } else {
                external.push(link);
            }
        } else {
            internal.push(link); // relative URLs are internal
        }
    }

    (internal, external)
}

fn extract_images(document: &Html, base_url: Option<&Url>) -> Vec<ImageData> {
    let selector = match Selector::parse("img") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    select_live(document, &selector)
        .filter_map(|el| {
            let src = el.value().attr("src")?;
            let resolved_src = if let Some(base) = base_url {
                base.join(src).ok()?.to_string()
            } else {
                src.to_string()
            };

            // `alt=""` is not a missing alt — it is the WCAG-correct way to mark
            // an image as decorative, and flagging it tells authors to undo the
            // right thing. Only a genuinely absent attribute counts as missing,
            // so the empty string is preserved here rather than folded to None.
            let alt = el.value().attr("alt").map(|s| s.to_string());

            let width = el.value().attr("width").and_then(|w| w.parse::<u32>().ok());
            let height = el
                .value()
                .attr("height")
                .and_then(|h| h.parse::<u32>().ok());

            Some(ImageData {
                src: resolved_src,
                alt,
                width,
                height,
            })
        })
        .collect()
}

fn extract_schema_types(document: &Html) -> Vec<String> {
    let selector = match Selector::parse("script[type=\"application/ld+json\"]") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let mut types = Vec::new();
    for el in select_live(document, &selector) {
        let json_text = el.text().collect::<String>();
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json_text) {
            collect_schema_types(&value, &mut types);
        }
    }
    types
}

/// Recursively collect `@type` values from a JSON-LD payload, handling
/// top-level arrays, `@graph` containers (Yoast/RankMath emit these), and
/// `@type` values that are themselves arrays.
fn collect_schema_types(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_schema_types(item, out);
            }
        }
        serde_json::Value::Object(obj) => {
            if let Some(graph) = obj.get("@graph") {
                collect_schema_types(graph, out);
            }
            match obj.get("@type") {
                Some(serde_json::Value::String(s)) => out.push(s.clone()),
                Some(serde_json::Value::Array(arr)) => {
                    for t in arr {
                        if let Some(s) = t.as_str() {
                            out.push(s.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn extract_og_tags(document: &Html) -> HashMap<String, String> {
    let selector = match Selector::parse("meta[property^=\"og:\"]") {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };

    let mut tags = HashMap::new();
    for el in select_live(document, &selector) {
        if let (Some(prop), Some(content)) =
            (el.value().attr("property"), el.value().attr("content"))
        {
            tags.insert(prop.to_string(), content.to_string());
        }
    }
    tags
}

fn extract_body_text(document: &Html) -> String {
    let body_selector = match Selector::parse("body") {
        Ok(s) => s,
        Err(_) => return String::new(),
    };

    let body = match select_live(document, &body_selector).next() {
        Some(b) => b,
        None => return String::new(),
    };

    // `noscript` belongs here alongside script and style. html5ever parses with
    // scripting enabled — as Googlebot renders — which makes a <noscript>
    // subtree a single raw *text* node holding its markup verbatim. Without this
    // the standard Google Tag Manager iframe snippet is harvested as prose: on
    // one news homepage 10,692 of 16,269 counted "words" were literal
    // `<img src=...>` strings, which also polluted heading text, the content
    // hash used for duplicate detection, and the readability score.
    // `template` joins them: its contents are an inert fragment the browser
    // never renders, so counting them inflates word count and poisons both the
    // duplicate-content hash and the readability score.
    let skip_selector = Selector::parse("script, style, noscript, template").ok();

    let mut skip_ids = std::collections::HashSet::new();
    if let Some(ref sel) = skip_selector {
        for el in select_live(document, sel) {
            skip_ids.insert(el.id());
        }
    }

    // Walk the body's descendants, skipping script/style subtrees
    let mut text_parts = Vec::new();
    for node in body.descendants() {
        // Check if any ancestor is a script/style
        if let Some(text) = node.value().as_text() {
            let mut is_in_skip = false;
            let mut parent = node.parent();
            while let Some(p) = parent {
                if skip_ids.contains(&p.id()) {
                    is_in_skip = true;
                    break;
                }
                parent = p.parent();
            }
            if !is_in_skip {
                let t = text.trim();
                if !t.is_empty() {
                    text_parts.push(t.to_string());
                }
            }
        }
    }

    text_parts.join(" ")
}

/// Count words in a way that is meaningful for scripts written without spaces.
///
/// Japanese, Chinese, Thai and Khmer do not separate words with spaces, so
/// `split_whitespace` collapsed an entire article to a handful of tokens: a
/// 2,300-character Japanese page counted as 4 words and was reported as thin
/// content. Characters in those scripts are counted individually — the measure
/// Google uses for CJK — while space-separated scripts keep token counting.
fn count_words(text: &str) -> u32 {
    let continuous = text.chars().filter(|c| is_continuous_script(*c)).count();
    let spaced = text
        .split_whitespace()
        .filter(|token| !token.chars().any(is_continuous_script))
        .count();
    (continuous + spaced) as u32
}

/// True for scripts that do not put spaces between words.
fn is_continuous_script(c: char) -> bool {
    crate::utils::pixel_width::is_full_width(c)
        || matches!(c as u32,
            0x0E00..=0x0E7F   // Thai
            | 0x0E80..=0x0EFF // Lao
            | 0x1780..=0x17FF // Khmer
            | 0x1000..=0x109F // Myanmar
        )
}

fn compute_hash(text: &str) -> String {
    // Hash the whitespace-collapsed text. HTML collapses whitespace runs when
    // rendering, so two documents differing only in source indentation are the
    // same page to a reader and to Google — but hashing the raw string gave them
    // different hashes and duplicate detection missed them. `word_count` was
    // already computed on the collapsed form, so the two disagreed.
    let mut hasher = Sha256::new();
    hasher.update(collapse_ws(text).as_bytes());
    format!("{:x}", hasher.finalize())
}

// ============================================================
// New extraction functions
// ============================================================

/// Extract title metadata: count, whether in <head>, length, pixel width.
fn extract_title_metadata(
    document: &Html,
    title: Option<&str>,
) -> (u32, bool, Option<u32>, Option<u32>) {
    // Inline SVG <title> elements are accessibility labels, not document
    // titles — counting them reported "Multiple Title Tags" on any page with an
    // accessible inline logo, which is most modern sites and is correct markup.
    // Titles in <body> are still counted: the parser puts them there when the
    // head is closed early by invalid markup, and that is a real finding rather
    // than a reason to pretend the page has no title.
    let selector = match Selector::parse("title") {
        Ok(s) => s,
        Err(_) => return (0, false, None, None),
    };

    let title_count = document_titles(document, &selector).count() as u32;
    let title_in_head = document_titles(document, &selector)
        .next()
        .is_some_and(|el| is_in_head(&el));

    // Character count, not byte count — a CJK/accented title must not be
    // measured at 2–3× its real length.
    let title_length = title.map(rendered_len);
    let title_pixel_width = title.map(crate::utils::pixel_width::estimate_title_pixel_width);

    (title_count, title_in_head, title_length, title_pixel_width)
}

/// Extract meta description metadata: count, whether in <head>, length, pixel width.
fn extract_meta_description_metadata(
    document: &Html,
    desc: Option<&str>,
) -> (u32, bool, Option<u32>, Option<u32>) {
    let selector = match Selector::parse("meta[name=\"description\" i]") {
        Ok(s) => s,
        Err(_) => return (0, false, None, None),
    };

    let count = select_live(document, &selector).count() as u32;

    let head_selector = Selector::parse("head meta[name=\"description\" i]").ok();
    let in_head = head_selector
        .map(|sel| select_live(document, &sel).next().is_some())
        .unwrap_or(false);

    let length = desc.map(rendered_len);
    let pixel_width = desc.map(crate::utils::pixel_width::estimate_description_pixel_width);

    (count, in_head, length, pixel_width)
}

/// Extract canonical metadata: count, is_relative, is_in_head, is_self_referencing.
fn extract_canonical_metadata(
    document: &Html,
    canonical_raw: Option<&str>,
    canonical_abs: Option<&str>,
    page_url: &str,
) -> (u32, bool, bool, bool) {
    let selector = match Selector::parse("link[rel~=\"canonical\"]") {
        Ok(s) => s,
        Err(_) => return (0, false, false, false),
    };

    let count = select_live(document, &selector).count() as u32;

    let head_selector = Selector::parse("head link[rel~=\"canonical\"]").ok();
    let in_head = head_selector
        .map(|sel| select_live(document, &sel).next().is_some())
        .unwrap_or(false);

    // Relativity is a property of the tag as authored.
    let is_relative = canonical_raw
        .map(|c| {
            let lower = c.to_ascii_lowercase();
            !lower.starts_with("http://") && !lower.starts_with("https://")
        })
        .unwrap_or(false);

    // Self-reference is checked against the resolved absolute canonical, so a
    // relative self-canonical (href="/page") is correctly recognized.
    let has_self_canonical = match canonical_abs {
        Some(c) => {
            let norm_c = crate::crawler::url_utils::normalize_url(c);
            let norm_p = crate::crawler::url_utils::normalize_url(page_url);
            matches!((norm_c, norm_p), (Ok(a), Ok(b)) if a == b)
        }
        None => false,
    };

    (count, is_relative, in_head, has_self_canonical)
}

/// Extract hreflang tags from <link rel="alternate" hreflang="..."> elements.
fn extract_hreflang_tags(document: &Html, base_url: Option<&Url>) -> Vec<HreflangTag> {
    // Google: "The <link> tags must be inside a well-formed <head> section."
    // Counting body-placed annotations as valid gave a broken, non-reciprocal
    // cluster a clean bill of health.
    let selector = match Selector::parse("head link[rel~=\"alternate\"][hreflang]") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    select_live(document, &selector)
        .filter_map(|el| {
            let lang = el.value().attr("hreflang")?.to_string();
            let href = el.value().attr("href")?;
            let url = if let Some(base) = base_url {
                base.join(href).ok()?.to_string()
            } else {
                href.to_string()
            };
            Some(HreflangTag { lang, url })
        })
        .collect()
}

/// Extract pagination links (rel="prev" and rel="next").
fn extract_pagination(document: &Html, base_url: Option<&Url>) -> Option<PaginationData> {
    let prev_selector = Selector::parse("link[rel~=\"prev\"]").ok();
    let next_selector = Selector::parse("link[rel~=\"next\"]").ok();

    let prev = prev_selector.and_then(|sel| {
        select_live(document, &sel).next().and_then(|el| {
            let href = el.value().attr("href")?;
            if let Some(base) = base_url {
                Some(base.join(href).ok()?.to_string())
            } else {
                Some(href.to_string())
            }
        })
    });

    let next = next_selector.and_then(|sel| {
        select_live(document, &sel).next().and_then(|el| {
            let href = el.value().attr("href")?;
            if let Some(base) = base_url {
                Some(base.join(href).ok()?.to_string())
            } else {
                Some(href.to_string())
            }
        })
    });

    if prev.is_some() || next.is_some() {
        Some(PaginationData { prev, next })
    } else {
        None
    }
}

/// Split the meta robots content string into individual directives.
fn extract_meta_robots_directives(meta_robots: Option<&str>) -> Vec<String> {
    match meta_robots {
        Some(mr) => mr
            .split(',')
            .map(|d| d.trim().to_lowercase())
            .filter(|d| !d.is_empty())
            .collect(),
        None => Vec::new(),
    }
}

/// On HTTPS pages, find resources loaded over HTTP (mixed content).
fn extract_mixed_content(document: &Html) -> Vec<String> {
    let mut mixed = Vec::new();

    let selectors = [
        "img[src]",
        "script[src]",
        "iframe[src]",
        "video[src]",
        "audio[src]",
        "source[src]",
        "link[rel~=\"stylesheet\"][href]",
    ];

    for sel_str in &selectors {
        let selector = match Selector::parse(sel_str) {
            Ok(s) => s,
            Err(_) => continue,
        };

        for el in select_live(document, &selector) {
            // For link[stylesheet], check href; for others, check src
            let url = if sel_str.contains("[href]") {
                el.value().attr("href")
            } else {
                el.value().attr("src")
            };

            if let Some(url) = url {
                if url.starts_with("http://") {
                    mixed.push(url.to_string());
                }
            }
        }
    }

    mixed
}

/// Find <form> tags with action starting with "http://".
fn extract_insecure_forms(document: &Html) -> Vec<String> {
    let selector = match Selector::parse("form[action]") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    select_live(document, &selector)
        .filter_map(|el| {
            let action = el.value().attr("action")?;
            if action.starts_with("http://") {
                Some(action.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Find <a target="_blank"> tags missing rel="noopener" or rel="noreferrer".
fn extract_unsafe_cross_origin_links(document: &Html) -> Vec<String> {
    let selector = match Selector::parse("a[target=\"_blank\"]") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    select_live(document, &selector)
        .filter_map(|el| {
            let rel = el.value().attr("rel").unwrap_or("").to_lowercase();
            let has_noopener = rel.contains("noopener");
            let has_noreferrer = rel.contains("noreferrer");
            if !has_noopener && !has_noreferrer {
                let href = el.value().attr("href").unwrap_or("").to_string();
                Some(href)
            } else {
                None
            }
        })
        .collect()
}

/// True if a `<script>`'s `type` marks it as executable JavaScript.
///
/// An absent, empty, or JS MIME type is JavaScript; anything else is a data
/// block. `application/ld+json` (schema markup) and `application/json`
/// (`__NEXT_DATA__` and friends) are `<script>` elements that never execute,
/// and counting them made every Next.js and schema-rich page look like it was
/// shipping excessive JavaScript.
fn is_javascript_script_type(script_type: Option<&str>) -> bool {
    match script_type {
        None => true,
        Some(t) => {
            let t = t.trim().to_ascii_lowercase();
            // A parameterized type such as `text/javascript; charset=utf-8`
            // still identifies as JavaScript.
            let base = t.split(';').next().unwrap_or("").trim().to_string();
            base.is_empty()
                || base == "module"
                || base == "text/javascript"
                || base == "application/javascript"
                || base == "text/ecmascript"
                || base == "application/ecmascript"
                || base == "text/jscript"
        }
    }
}

/// Extract executable `<script>` elements, capturing src/attributes/size and
/// classifying third-party vs same-origin by host. Non-executable data blocks
/// (JSON-LD, `application/json`) are skipped — see
/// [`is_javascript_script_type`].
fn extract_scripts(document: &Html, base_host: Option<&str>) -> Vec<ScriptData> {
    let selector = match Selector::parse("script") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let head_ids: std::collections::HashSet<_> = Selector::parse("head script")
        .ok()
        .map(|sel| select_live(document, &sel).map(|el| el.id()).collect())
        .unwrap_or_default();

    select_live(document, &selector)
        .filter(|el| is_javascript_script_type(el.value().attr("type")))
        .map(|el| {
            let src = el.value().attr("src").map(String::from);
            let is_async = el.value().attr("async").is_some();
            let is_defer = el.value().attr("defer").is_some();
            let is_module = el
                .value()
                .attr("type")
                .map(|t| t.trim().eq_ignore_ascii_case("module"))
                .unwrap_or(false);
            let in_head = head_ids.contains(&el.id());

            let (size_bytes, is_third_party) = if let Some(src_url) = src.as_deref() {
                let third_party = base_host
                    .and_then(|bh| {
                        Url::parse(src_url)
                            .ok()
                            .and_then(|u| u.host_str().map(String::from))
                            .map(|h| h != bh)
                    })
                    .unwrap_or(false);
                (0u32, third_party)
            } else {
                let inline = el.text().collect::<String>();
                (inline.len() as u32, false)
            };

            ScriptData {
                src,
                is_async,
                is_defer,
                is_module,
                in_head,
                size_bytes,
                is_third_party,
            }
        })
        .collect()
}

/// Extract full structured data: JSON-LD blocks and Microdata.
fn extract_structured_data(document: &Html) -> Vec<StructuredDataBlock> {
    let mut blocks = Vec::new();

    // JSON-LD
    if let Ok(selector) = Selector::parse("script[type=\"application/ld+json\"]") {
        for el in select_live(document, &selector) {
            let raw = el.text().collect::<String>();
            match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(value) => {
                    // A block may declare several entities: a top-level array,
                    // or the `@graph` wrapper that Yoast and Rank Math emit on
                    // every WordPress page. Reading `@type` off the outer object
                    // returned None for both shapes, so validation was skipped
                    // silently — on most of the WordPress web. Each entity
                    // becomes its own block so it carries its own type and its
                    // own verdict.
                    let entities = top_level_entities(&value);
                    if entities.is_empty() {
                        blocks.push(StructuredDataBlock {
                            format: StructuredDataFormat::JsonLd,
                            schema_type: None,
                            raw: raw.clone(),
                            parse_error: None,
                            missing_required: Vec::new(),
                        });
                        continue;
                    }
                    let single = entities.len() == 1;
                    for entity in entities {
                        let schema_type = primary_type(entity);
                        let missing_required = schema_type
                            .as_deref()
                            .map(|st| check_required_fields(st, entity))
                            .unwrap_or_default();
                        blocks.push(StructuredDataBlock {
                            format: StructuredDataFormat::JsonLd,
                            schema_type,
                            // Keep the author's verbatim markup when the block
                            // holds a single entity; serialise the individual
                            // entity when one script carried several.
                            raw: if single {
                                raw.clone()
                            } else {
                                entity.to_string()
                            },
                            parse_error: None,
                            missing_required,
                        });
                    }
                }
                Err(e) => blocks.push(StructuredDataBlock {
                    format: StructuredDataFormat::JsonLd,
                    schema_type: None,
                    raw,
                    parse_error: Some(e.to_string()),
                    missing_required: Vec::new(),
                }),
            }
        }
    }

    // Microdata
    if let Ok(selector) = Selector::parse("[itemscope][itemtype]") {
        for el in select_live(document, &selector) {
            if let Some(itemtype) = el.value().attr("itemtype") {
                // Extract the type name from the URL (e.g., "https://schema.org/Product" -> "Product")
                let schema_type = itemtype.rsplit('/').next().map(String::from);

                // Collect the outer HTML as raw
                let raw = itemtype.to_string();

                blocks.push(StructuredDataBlock {
                    format: StructuredDataFormat::Microdata,
                    schema_type,
                    raw,
                    parse_error: None,
                    missing_required: Vec::new(),
                });
            }
        }
    }

    blocks
}

/// Number of characters a reader actually sees.
///
/// Zero-width joiners, the BOM, directional marks and combining marks are code
/// points with no glyph of their own, so counting them made a 17-character
/// title measure 21 and pushed titles over the length thresholds.
fn rendered_len(text: &str) -> u32 {
    text.chars()
        .filter(|c| !crate::utils::pixel_width::is_zero_width(*c))
        .count() as u32
}

/// The entities a JSON-LD block declares at the top level.
///
/// Handles the three shapes that appear in the wild: a single object, a
/// top-level array, and the `@graph` wrapper. Deliberately does *not* recurse
/// into arbitrary nested objects — a nested `publisher` Organization or an
/// `offers` Offer is a property of its parent, not a separately-declared entity,
/// and validating those would demand required fields on things Google never
/// evaluates standalone.
fn top_level_entities(value: &serde_json::Value) -> Vec<&serde_json::Value> {
    let mut out = Vec::new();
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                out.extend(top_level_entities(item));
            }
        }
        serde_json::Value::Object(obj) => {
            if let Some(graph) = obj.get("@graph") {
                out.extend(top_level_entities(graph));
            } else if obj.contains_key("@type") {
                out.push(value);
            }
        }
        _ => {}
    }
    out
}

/// The entity's primary `@type`. `@type` may be an array (e.g.
/// `["Product","Thing"]`), in which case the first entry is the primary one.
fn primary_type(entity: &serde_json::Value) -> Option<String> {
    match entity.get("@type") {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Array(arr)) => {
            arr.iter().find_map(|t| t.as_str()).map(|s| s.to_string())
        }
        _ => None,
    }
}

/// Fields Google documents as *required* for a rich result, per type.
///
/// Only properties Google's own reference marks Required belong here. Several
/// entries previously asserted requirements Google explicitly denies:
///   - Article/NewsArticle/BlogPosting: "There are no required properties;
///     instead, add the properties that apply to your content."
///   - Organization: "There are no required properties; instead, we recommend
///     adding as many properties that are relevant to your organization."
///     (`address` is Required for LocalBusiness, which keeps it.)
///   - Product: the product-snippet reference lists exactly one required
///     property, `name`; `image` is recommended. The old list demanded `image`
///     and so flagged eligible products while passing ineligible ones.
fn check_required_fields(schema_type: &str, value: &serde_json::Value) -> Vec<String> {
    let required: &[&str] = match schema_type {
        "Product" => &["name"],
        "LocalBusiness" => &["name", "address"],
        "BreadcrumbList" => &["itemListElement"],
        "FAQPage" => &["mainEntity"],
        "Recipe" => &["name", "image"],
        "Event" => &["name", "startDate", "location"],
        "VideoObject" => &["name", "uploadDate", "thumbnailUrl"],
        _ => return Vec::new(),
    };

    required
        .iter()
        .filter(|field| value.get(**field).is_none())
        .map(|field| field.to_string())
        .collect()
}

/// Detect soft 404s: pages that return 200 while being an error placeholder.
///
/// A soft 404 is *thin* as well as error-worded. The title branch used to return
/// early with no length guard, so any article whose headline contained a phrase
/// like "does not exist" was flagged however long it was — including live
/// 630-word Wikipedia articles. Both signals now require the page to be thin,
/// which is what the doc comment always claimed.
const SOFT_404_MAX_WORDS: u32 = 100;

fn detect_soft_404(body_text: &str, title: Option<&str>) -> bool {
    let patterns = [
        "page not found",
        "page doesn't exist",
        "no longer available",
        "we can't find",
        "does not exist",
    ];

    // Uses the script-aware count so a short CJK error page is still seen as
    // thin, and a long one is not.
    if count_words(body_text) >= SOFT_404_MAX_WORDS {
        return false;
    }

    let body_lower = body_text.to_lowercase();
    let title_lower = title.map(|t| t.to_lowercase());

    if let Some(ref t) = title_lower {
        if patterns.iter().any(|pattern| t.contains(pattern)) {
            return true;
        }
    }

    patterns.iter().any(|pattern| body_lower.contains(pattern))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic_html() {
        let html = r#"
        <html>
        <head>
            <title>Test Page</title>
            <meta name="description" content="A test description">
            <meta name="robots" content="index, follow">
            <link rel="canonical" href="https://example.com/test">
        </head>
        <body>
            <h1>Hello World</h1>
            <p>Some body text here.</p>
            <a href="/about">About</a>
        </body>
        </html>"#;

        let page = parse_html(html, "https://example.com/test");
        assert_eq!(page.title.as_deref(), Some("Test Page"));
        assert_eq!(page.meta_description.as_deref(), Some("A test description"));
        assert_eq!(page.canonical.as_deref(), Some("https://example.com/test"));
        assert_eq!(page.h1, vec!["Hello World"]);
        assert!(page.word_count > 0);
    }

    #[test]
    fn test_title_metadata() {
        let html = r#"
        <html>
        <head><title>Hello World</title></head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.title_count, 1);
        assert!(page.title_in_head);
        assert_eq!(page.title_length, Some(11));
        assert!(page.title_pixel_width.is_some());
    }

    /// Anything the HTML spec disallows in `<head>` closes it early, and the
    /// `<title>` after it is parsed into `<body>`. Scoping title selection to
    /// `head title` reported those pages as having no title at all — a false
    /// critical finding on a page whose markup plainly carries one, reported
    /// from the field on a real site. A tracking pixel in the head is the
    /// commonest cause; a stray text node or a `<div>` from a mis-templated
    /// partial do the same thing.
    #[test]
    fn a_title_pushed_out_of_head_by_invalid_markup_is_still_found() {
        for (name, head) in [
            (
                "tracking pixel",
                r#"<img src="/px.gif"><title>Real Title</title>"#,
            ),
            ("stray text", r#"&nbsp;<title>Real Title</title>"#),
            ("stray div", r#"<div></div><title>Real Title</title>"#),
        ] {
            let html = format!("<html><head>{head}</head><body></body></html>");
            let page = parse_html(&html, "https://example.com/");
            assert_eq!(
                page.title.as_deref(),
                Some("Real Title"),
                "{name}: the title is in the document and must be reported"
            );
            assert_eq!(page.title_count, 1, "{name}");
            assert!(
                !page.title_in_head,
                "{name}: the parser moved it to <body>, which is the real finding"
            );
        }
    }

    /// An inline SVG logo labels itself with `<title>`. That is an
    /// accessibility label, not the document title — the reason title selection
    /// was scoped to `<head>` in the first place — so it must not be adopted as
    /// one, nor counted toward "Multiple Title Tags".
    #[test]
    fn an_inline_svg_label_is_not_the_document_title() {
        let html = r#"<html><head></head>
            <body><svg><title>Logo</title></svg></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.title, None, "an SVG label is not a document title");
        assert_eq!(page.title_count, 0);

        let with_real = r#"<html><head><title>Real Title</title></head>
            <body><svg><title>Logo</title></svg></body></html>"#;
        let page = parse_html(with_real, "https://example.com/");
        assert_eq!(page.title.as_deref(), Some("Real Title"));
        assert_eq!(
            page.title_count, 1,
            "the SVG label must not count as a second title"
        );
        assert!(page.title_in_head);
    }

    /// Two real <title> elements in <head> is a genuine duplicate.
    #[test]
    fn test_multiple_titles() {
        let html = r#"
        <html>
        <head><title>First</title><title>Second</title></head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.title_count, 2);
    }

    #[test]
    fn test_meta_description_metadata() {
        let html = r#"
        <html>
        <head><meta name="description" content="Hello test"></head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.meta_description_count, 1);
        assert!(page.meta_desc_in_head);
        assert_eq!(page.meta_description_length, Some(10));
        assert!(page.meta_description_pixel_width.is_some());
    }

    /// Regression: an inline SVG logo carries its own <title> as an
    /// accessibility label. Counting it as a document title reported
    /// "Multiple Title Tags" on gov.uk, MDN, stripe.com and Smashing Magazine
    /// — i.e. on correct, accessible markup.
    #[test]
    fn svg_titles_are_not_document_titles() {
        let html = r#"
        <html>
        <head><title>Real Page Title</title></head>
        <body>
          <svg aria-label="Logo"><title>Logo</title><circle r="5"/></svg>
        </body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.title_count, 1, "only the <head> title counts");
        assert_eq!(page.title.as_deref(), Some("Real Page Title"));
    }

    #[test]
    fn test_canonical_metadata_self_referencing() {
        let html = r#"
        <html>
        <head><link rel="canonical" href="https://example.com/page"></head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/page");
        assert_eq!(page.canonical_count, 1);
        assert!(page.has_self_canonical);
        assert!(!page.canonical_is_relative);
    }

    #[test]
    fn test_canonical_relative() {
        let html = r#"
        <html>
        <head><link rel="canonical" href="/page"></head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/page");
        assert!(page.canonical_is_relative);
        // Relative canonical is resolved to absolute and recognized as self.
        assert_eq!(page.canonical.as_deref(), Some("https://example.com/page"));
        assert!(page.has_self_canonical);
    }

    #[test]
    fn test_base_href_resolves_relative_links() {
        // With <base href>, a relative link resolves against the base, not the page URL.
        let html = r#"<html><head>
            <base href="https://cdn.example.com/app/">
        </head><body>
            <a href="page.html">P</a>
            <img src="logo.png">
        </body></html>"#;
        let page = parse_html(html, "https://example.com/some/deep/path");
        // The link resolves against the CDN base and is therefore external.
        assert!(page
            .external_links
            .iter()
            .any(|l| l.url == "https://cdn.example.com/app/page.html"));
        assert!(page
            .images
            .iter()
            .any(|i| i.src == "https://cdn.example.com/app/logo.png"));
    }

    #[test]
    fn test_canonical_multi_token_rel() {
        // rel="canonical shortlink" must still be recognized as a canonical.
        let html = r#"<html><head>
            <link rel="canonical shortlink" href="https://example.com/p">
        </head><body></body></html>"#;
        let page = parse_html(html, "https://example.com/p");
        assert_eq!(page.canonical.as_deref(), Some("https://example.com/p"));
    }

    #[test]
    fn test_meta_description_capitalized_name() {
        // name="Description" (capitalized) must still be found.
        let html = r#"<html><head>
            <meta name="Description" content="Cap desc">
        </head><body></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.meta_description.as_deref(), Some("Cap desc"));
    }

    #[test]
    fn test_hreflang_tags() {
        let html = r#"
        <html>
        <head>
            <link rel="alternate" hreflang="en" href="https://example.com/en/">
            <link rel="alternate" hreflang="es" href="https://example.com/es/">
        </head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/en/");
        assert_eq!(page.hreflang_tags.len(), 2);
        assert_eq!(page.hreflang_tags[0].lang, "en");
        assert_eq!(page.hreflang_tags[1].lang, "es");
    }

    #[test]
    fn test_pagination() {
        let html = r#"
        <html>
        <head>
            <link rel="prev" href="https://example.com/page/1">
            <link rel="next" href="https://example.com/page/3">
        </head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/page/2");
        assert!(page.pagination.is_some());
        let pag = page.pagination.unwrap();
        assert_eq!(pag.prev.as_deref(), Some("https://example.com/page/1"));
        assert_eq!(pag.next.as_deref(), Some("https://example.com/page/3"));
    }

    #[test]
    fn test_no_pagination() {
        let html = r#"<html><head></head><body></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert!(page.pagination.is_none());
    }

    #[test]
    fn test_meta_robots_directives() {
        let html = r#"
        <html>
        <head><meta name="robots" content="noindex, nofollow, noarchive"></head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.meta_robots_directives.len(), 3);
        assert!(page.meta_robots_directives.contains(&"noindex".to_string()));
        assert!(page
            .meta_robots_directives
            .contains(&"nofollow".to_string()));
        assert!(page
            .meta_robots_directives
            .contains(&"noarchive".to_string()));
    }

    #[test]
    fn test_mixed_content_detection() {
        let html = r#"
        <html>
        <head></head>
        <body>
            <img src="http://example.com/image.jpg">
            <img src="https://example.com/safe.jpg">
            <script src="http://cdn.example.com/script.js"></script>
        </body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.mixed_content.len(), 2);
    }

    #[test]
    fn test_no_mixed_content_on_http() {
        let html = r#"
        <html>
        <head></head>
        <body>
            <img src="http://example.com/image.jpg">
        </body>
        </html>"#;
        let page = parse_html(html, "http://example.com/");
        assert!(page.mixed_content.is_empty());
    }

    #[test]
    fn test_insecure_forms() {
        let html = r#"
        <html>
        <head></head>
        <body>
            <form action="http://example.com/submit">
                <input type="text">
            </form>
            <form action="https://example.com/safe">
                <input type="text">
            </form>
        </body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.insecure_forms.len(), 1);
        assert_eq!(page.insecure_forms[0], "http://example.com/submit");
    }

    #[test]
    fn test_unsafe_cross_origin_links() {
        let html = r#"
        <html>
        <head></head>
        <body>
            <a href="https://other.com" target="_blank">Unsafe</a>
            <a href="https://safe.com" target="_blank" rel="noopener">Safe</a>
            <a href="https://also-safe.com" target="_blank" rel="noreferrer">Also Safe</a>
        </body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.unsafe_cross_origin_links.len(), 1);
        assert_eq!(page.unsafe_cross_origin_links[0], "https://other.com");
    }

    #[test]
    fn test_structured_data_json_ld() {
        let html = r#"
        <html>
        <head>
            <script type="application/ld+json">
            {
                "@type": "Article",
                "headline": "Test Article",
                "author": "John",
                "datePublished": "2024-01-01"
            }
            </script>
        </head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.structured_data.len(), 1);
        assert!(matches!(
            page.structured_data[0].format,
            StructuredDataFormat::JsonLd
        ));
        assert_eq!(
            page.structured_data[0].schema_type.as_deref(),
            Some("Article")
        );
        assert!(page.structured_data[0].missing_required.is_empty());
    }

    #[test]
    fn test_schema_types_graph_and_array() {
        // @graph payloads (Yoast/RankMath) and array @type must yield types.
        let html = r#"<html><head>
            <script type="application/ld+json">
            {"@context":"https://schema.org","@graph":[
                {"@type":"Organization","name":"Acme"},
                {"@type":["Product","Thing"],"name":"Widget"}
            ]}
            </script>
        </head><body></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert!(page.schema_types.contains(&"Organization".to_string()));
        assert!(page.schema_types.contains(&"Product".to_string()));
        assert!(page.schema_types.contains(&"Thing".to_string()));
    }

    #[test]
    fn test_schema_types_toplevel_array() {
        let html = r#"<html><head>
            <script type="application/ld+json">
            [{"@type":"WebSite","name":"S"},{"@type":"BreadcrumbList"}]
            </script>
        </head><body></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert!(page.schema_types.contains(&"WebSite".to_string()));
        assert!(page.schema_types.contains(&"BreadcrumbList".to_string()));
    }

    #[test]
    fn test_structured_data_missing_required() {
        let html = r#"
        <html>
        <head>
            <script type="application/ld+json">
            {
                "@type": "Product"
            }
            </script>
        </head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.structured_data.len(), 1);
        assert!(page.structured_data[0]
            .missing_required
            .contains(&"name".to_string()));
        // `image` is Recommended, not Required: Google's product-snippet
        // reference lists exactly one required property, `name`. This
        // assertion previously demanded `image` too, which flagged eligible
        // products.
        assert!(!page.structured_data[0]
            .missing_required
            .contains(&"image".to_string()));
    }

    #[test]
    fn test_structured_data_parse_error() {
        let html = r#"
        <html>
        <head>
            <script type="application/ld+json">
            {invalid json}
            </script>
        </head>
        <body></body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.structured_data.len(), 1);
        assert!(page.structured_data[0].parse_error.is_some());
    }

    #[test]
    fn test_microdata_extraction() {
        let html = r#"
        <html>
        <head></head>
        <body>
            <div itemscope itemtype="https://schema.org/Product">
                <span itemprop="name">Widget</span>
            </div>
        </body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        let microdata_blocks: Vec<_> = page
            .structured_data
            .iter()
            .filter(|b| matches!(b.format, StructuredDataFormat::Microdata))
            .collect();
        assert!(!microdata_blocks.is_empty());
        assert_eq!(microdata_blocks[0].schema_type.as_deref(), Some("Product"));
    }

    #[test]
    fn test_readability_score() {
        let html = r#"
        <html>
        <head></head>
        <body>
            <p>The cat sat on the mat. The dog ran in the park. The sun was warm today.</p>
        </body>
        </html>"#;
        let page = parse_html(html, "https://example.com/");
        assert!(page.readability_score.is_some());
    }

    #[test]
    fn test_soft_404_detection() {
        let html = r#"
        <html>
        <head><title>Page Not Found</title></head>
        <body>
            <p>Sorry, we couldn't find this page.</p>
        </body>
        </html>"#;
        let page = parse_html(html, "https://example.com/missing");
        assert!(page.is_soft_404);
    }

    #[test]
    fn test_not_soft_404() {
        let html = r#"
        <html>
        <head><title>About Us</title></head>
        <body>
            <p>Welcome to our company page.</p>
        </body>
        </html>"#;
        let page = parse_html(html, "https://example.com/about");
        assert!(!page.is_soft_404);
    }

    #[test]
    fn test_is_https() {
        let html = r#"<html><head></head><body></body></html>"#;
        let page_https = parse_html(html, "https://example.com/");
        assert!(page_https.is_https);

        let page_http = parse_html(html, "http://example.com/");
        assert!(!page_http.is_https);
    }

    /// Regression: `<template>` contents are an inert DocumentFragment. A
    /// browser does not render them, follow their links, or apply their meta
    /// tags — but scraper walks the whole node arena, so the crawler was
    /// fetching `/product/{{slug}}` placeholders, 404ing, and reporting the
    /// result as the site's own broken links. One inert element moved a
    /// fixture's health score from 59 to 31.
    #[test]
    fn template_contents_are_not_live_page_content() {
        let html = r#"<!doctype html><html><head><title>Real</title></head><body>
            <h1>Real</h1><a href="/real">real</a><img src="/real.png" alt="a">
            <template>
              <meta name="robots" content="noindex,nofollow">
              <link rel="canonical" href="/wrong">
              <a href="/product/{{slug}}">placeholder</a>
              <img src="/y.png">
              <h1>Template H1</h1>
            </template>
        </body></html>"#;
        let page = parse_html(html, "https://example.com/");

        assert_eq!(page.meta_robots, None, "template meta must not apply");
        assert_eq!(page.canonical, None, "template canonical must not apply");
        assert_eq!(page.h1, vec!["Real"], "template h1 must not count");
        assert_eq!(page.internal_links.len(), 1, "only the real link");
        assert_eq!(page.internal_links[0].url, "https://example.com/real");
        assert_eq!(page.images.len(), 1, "only the real image");
    }

    /// Regression: relative links must resolve against the URL that actually
    /// served the document, not the one originally requested. A site that 301s
    /// `/` to `/en/` had every relative link resolved one directory too high,
    /// so the crawler invented URLs, fetched them, and reported the 404s as the
    /// site's broken links. RFC 3986 §5.1.3.
    #[test]
    fn relative_links_resolve_against_the_serving_url() {
        let html =
            r#"<html><head><title>T</title></head><body><a href="s.html">x</a></body></html>"#;
        let page = parse_html(html, "https://example.com/d/t.html");
        assert_eq!(page.internal_links[0].url, "https://example.com/d/s.html");
    }

    // ---------------------------------------------------------------
    // Regressions from the edge-case sweep
    // ---------------------------------------------------------------

    /// `<noscript>` is a raw-text node when scripting is enabled (how Googlebot
    /// renders), so its markup was harvested as prose. The Google Tag Manager
    /// snippet alone added five "words" and raw `<iframe …>` text to body_text
    /// on a large share of the real web.
    #[test]
    fn noscript_markup_is_not_page_text() {
        let html = r#"<!doctype html><html><head><title>T</title></head><body>
            <noscript><iframe src="https://www.googletagmanager.com/ns.html?id=GTM-ABC"
              height="0" width="0" style="display:none"></iframe></noscript>
            <h1>Heading</h1><p>Real page content lives here.</p>
        </body></html>"#;
        let page = parse_html(html, "https://example.com/");

        assert!(
            !page.body_text.contains("<iframe"),
            "raw noscript markup leaked into body text: {}",
            page.body_text
        );
        assert!(!page.body_text.contains("googletagmanager"));
        // "Heading" + the five words of the paragraph.
        assert_eq!(page.word_count, 6, "body was {:?}", page.body_text);
    }

    /// A `<noscript>` fallback image inside a heading made a 61-character
    /// headline measure 819 characters and trip the over-70 check.
    #[test]
    fn noscript_inside_a_heading_does_not_inflate_it() {
        let html = r#"<!doctype html><html><head><title>T</title></head><body>
            <h2>Short headline<noscript><img class="a b c" src="https://cdn.example.com/very/long/path.jpg" srcset="x 1x, y 2x"></noscript></h2>
        </body></html>"#;
        let page = parse_html(html, "https://example.com/");
        let h2 = page.headings.iter().find(|h| h.level == 2).unwrap();
        assert_eq!(h2.text, "Short headline");
    }

    /// Browsers collapse whitespace for `document.title` and the SERP snippet,
    /// so a template-indented title rendered 12 characters shorter than its raw
    /// text node and was falsely reported over-length.
    #[test]
    fn title_and_description_whitespace_is_collapsed() {
        let html = "<!doctype html><html><head>\n<title>\n    Winter Jackets\n    | Northwind Supply\n  </title>\n<meta name=\"description\" content=\"A description\n   wrapped across\n   source lines.\">\n</head><body><h1>x</h1></body></html>";
        let page = parse_html(html, "https://example.com/");
        assert_eq!(
            page.title.as_deref(),
            Some("Winter Jackets | Northwind Supply")
        );
        assert_eq!(
            page.meta_description.as_deref(),
            Some("A description wrapped across source lines.")
        );
    }

    /// Two documents differing only in source indentation render identically,
    /// so duplicate detection must see them as the same page. word_count
    /// already agreed; only the hash disagreed.
    #[test]
    fn whitespace_only_differences_hash_identically() {
        let a = parse_html(
            "<html><head><title>A</title></head><body><p>Hello world shared copy.</p></body></html>",
            "https://example.com/a",
        );
        let b = parse_html(
            "<html><head><title>B</title></head><body><p>Hello    world\tshared\n\ncopy.</p></body></html>",
            "https://example.com/b",
        );
        assert_eq!(a.content_hash, b.content_hash);
    }

    /// Google applies "the sum of the negative rules" across every robots meta,
    /// and honours crawler-specific tags. Reading only the first tag let a
    /// theme's `index,follow` mask an SEO plugin's `noindex`, and the page was
    /// reported indexable and written into the generated sitemap.
    #[test]
    fn every_robots_meta_is_combined() {
        let html = r#"<!doctype html><html><head><title>T</title>
            <meta name="robots" content="index, follow">
            <meta name="robots" content="noindex, nofollow">
        </head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert!(page.is_noindex(), "second robots meta was dropped");
        assert!(page.meta_robots_directives.iter().any(|d| d == "nofollow"));
    }

    #[test]
    fn crawler_specific_robots_meta_is_read() {
        let html = r#"<!doctype html><html><head><title>T</title>
            <meta name="googlebot" content="noindex, nofollow">
        </head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert!(page.is_noindex(), "meta name=googlebot was ignored");
    }

    /// Regression: `googlebot-news` scopes to Google News only — Google's
    /// documentation is explicit that it does not affect general Search. Merging
    /// it into the general rules marked the page `is_indexable = false`, which
    /// gates roughly twenty checks (duplicate-title detection silently stopped
    /// firing), dropped the page from the generated sitemap, and advised
    /// removing a live, Search-indexable article from the client's sitemap.
    #[test]
    fn googlebot_news_does_not_affect_search_indexability() {
        let html = r#"<!doctype html><html><head><title>T</title>
            <meta name="googlebot-news" content="noindex">
        </head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");

        assert!(
            !page.is_noindex(),
            "a News-only noindex was applied to Search: {:?}",
            page.meta_robots_directives
        );
        // The directive is still recorded, carrying the scope it was declared
        // under, so the CSV export and HTML report can show it.
        assert_eq!(page.meta_robots.as_deref(), Some("googlebot-news: noindex"));
        assert_eq!(
            page.meta_robots_directives,
            vec!["googlebot-news: noindex".to_string()]
        );
    }

    /// A general `robots` noindex on the same page still applies — scoping the
    /// News tag must not make the page unconditionally indexable.
    #[test]
    fn a_general_noindex_still_applies_alongside_googlebot_news() {
        let html = r#"<!doctype html><html><head><title>T</title>
            <meta name="robots" content="noindex">
            <meta name="googlebot-news" content="index">
        </head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert!(page.is_noindex(), "the general robots noindex was dropped");
    }

    /// Every token of a scoped tag carries the prefix. The directive list is
    /// split on commas downstream, so prefixing only the value as a whole would
    /// leak the second token (`nosnippet` here) as a general directive.
    #[test]
    fn every_scoped_token_keeps_its_scope() {
        let html = r#"<!doctype html><html><head><title>T</title>
            <meta name="googlebot-news" content="noindex, nosnippet">
        </head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(
            page.meta_robots_directives,
            vec![
                "googlebot-news: noindex".to_string(),
                "googlebot-news: nosnippet".to_string()
            ]
        );
        assert!(!page.is_noindex());
        assert!(
            !page
                .effective_robots_directives()
                .iter()
                .any(|d| d == "nosnippet"),
            "a News-scoped nosnippet leaked into the general directives"
        );
    }

    #[test]
    fn surface_scoped_directives_are_recognized() {
        assert!(is_surface_scoped_directive("googlebot-news: noindex"));
        assert!(!is_surface_scoped_directive("noindex"));
        assert!(!is_surface_scoped_directive("max-snippet:50"));
        assert!(!is_surface_scoped_directive("googlebot: noindex"));
    }

    /// A soft 404 is thin *and* error-worded. The title branch returned early
    /// with no length guard, flagging live 600-word articles whose headline
    /// contained a phrase like "does not exist".
    #[test]
    fn a_long_article_is_not_a_soft_404() {
        let body = "This is a full length editorial article with real prose. ".repeat(40);
        let html = format!(
            "<html><head><title>Why the perfect employee does not exist</title></head><body><h1>h</h1><p>{body}</p></body></html>"
        );
        let page = parse_html(&html, "https://example.com/");
        assert!(page.word_count > 100);
        assert!(!page.is_soft_404, "content-rich page flagged as a soft 404");
    }

    #[test]
    fn a_thin_error_page_is_still_a_soft_404() {
        let html =
            "<html><head><title>Page not found</title></head><body><h1>Sorry</h1></body></html>";
        let page = parse_html(html, "https://example.com/");
        assert!(page.is_soft_404);
    }

    /// Google states verbatim that Article and Organization have no required
    /// properties; the tool asserted three and two respectively, contradicting
    /// its own guidance text on eligible pages.
    #[test]
    fn types_google_lists_no_required_properties_for_are_not_flagged() {
        let html = r#"<html><head><title>T</title><script type="application/ld+json">
            {"@context":"https://schema.org","@type":"Article","headline":"A Headline"}
        </script></head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        let block = page.structured_data.first().unwrap();
        assert_eq!(block.schema_type.as_deref(), Some("Article"));
        assert!(
            block.missing_required.is_empty(),
            "Article has no required properties per Google: {:?}",
            block.missing_required
        );

        let html = r#"<html><head><title>T</title><script type="application/ld+json">
            {"@context":"https://schema.org","@type":"Organization","name":"Acme","url":"https://acme.test/"}
        </script></head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert!(page.structured_data[0].missing_required.is_empty());
    }

    /// Product's one documented required property is `name`; the old list also
    /// demanded `image`, so it flagged eligible products and passed ineligible
    /// ones.
    #[test]
    fn product_requires_only_a_name() {
        let html = r#"<html><head><title>T</title><script type="application/ld+json">
            {"@context":"https://schema.org","@type":"Product","name":"Widget","offers":{"@type":"Offer","price":"9.99"}}
        </script></head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert!(page.structured_data[0].missing_required.is_empty());

        let html = r#"<html><head><title>T</title><script type="application/ld+json">
            {"@context":"https://schema.org","@type":"Product","sku":"ABC"}
        </script></head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.structured_data[0].missing_required, vec!["name"]);
    }

    /// `@graph` is what Yoast and Rank Math emit on every WordPress page.
    /// Reading `@type` off the outer object returned None, so validation was
    /// silently skipped for most of the WordPress web.
    #[test]
    fn graph_and_array_entities_are_validated() {
        let html = r#"<html><head><title>T</title><script type="application/ld+json">
        {"@context":"https://schema.org","@graph":[
          {"@type":"WebSite","@id":"https://t.test/#website"},
          {"@type":"Product","sku":"ABC","description":"no name"}]}
        </script></head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        let product = page
            .structured_data
            .iter()
            .find(|b| b.schema_type.as_deref() == Some("Product"))
            .expect("@graph Product was not extracted");
        assert_eq!(product.missing_required, vec!["name"]);

        // Top-level array, and an @type given as an array.
        let html = r#"<html><head><title>T</title><script type="application/ld+json">
        [{"@type":"Organization","name":"Acme"},{"@type":["Product","Thing"],"sku":"X"}]
        </script></head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        let product = page
            .structured_data
            .iter()
            .find(|b| b.schema_type.as_deref() == Some("Product"))
            .expect("array entity was not extracted");
        assert_eq!(product.missing_required, vec!["name"]);
    }

    /// Nested entities are properties of their parent, not separate
    /// declarations — validating them would demand `address` on every embedded
    /// publisher Organization.
    #[test]
    fn nested_entities_are_not_validated_standalone() {
        let html = r#"<html><head><title>T</title><script type="application/ld+json">
        {"@context":"https://schema.org","@type":"Article","headline":"H",
         "publisher":{"@type":"Organization","name":"Acme"}}
        </script></head><body><h1>x</h1></body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.structured_data.len(), 1);
        assert_eq!(
            page.structured_data[0].schema_type.as_deref(),
            Some("Article")
        );
    }

    /// Japanese, Chinese and Thai do not space their words, so an entire
    /// article collapsed to a handful of tokens and was reported thin.
    #[test]
    fn continuous_scripts_are_counted_by_character() {
        let ja = "ウィキペディアは誰でも編集できる百科事典です。".repeat(10);
        let html =
            format!("<html><head><title>T</title></head><body><h1>h</h1><p>{ja}</p></body></html>");
        let page = parse_html(&html, "https://example.com/");
        assert!(
            page.word_count > 100,
            "Japanese article counted as {} words",
            page.word_count
        );

        // Latin text is unaffected.
        let page = parse_html(
            "<html><head><title>T</title></head><body><p>one two three four five</p></body></html>",
            "https://example.com/",
        );
        assert_eq!(page.word_count, 5);
    }

    /// URL schemes are case-insensitive and browsers trim href whitespace, so
    /// `MAILTO:` and a padded `javascript:` are as non-navigational as their
    /// canonical forms. Counting them masked the dead-end check on a page whose
    /// only anchor was a mailto.
    #[test]
    fn non_navigational_schemes_are_not_links() {
        let html = r#"<html><head><title>T</title></head><body>
            <a href="MAILTO:sales@example.com">mail</a>
            <a href="  javascript:alert(1)  ">js</a>
            <a href="data:text/html,<h1>hi</h1>">data</a>
            <a href="TEL:+15551234">tel</a>
            <a href="/real">real</a>
        </body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.internal_links.len(), 1);
        assert_eq!(page.internal_links[0].url, "https://example.com/real");
    }

    /// Google: hreflang `<link>` tags "must be inside a well-formed <head>".
    /// Counting body-placed ones as valid gave a broken cluster a clean report.
    #[test]
    fn hreflang_outside_head_is_ignored() {
        let html = r#"<!doctype html><html><head><title>T</title>
            <link rel="alternate" hreflang="en" href="/en/">
        </head><body><h1>x</h1>
            <link rel="alternate" hreflang="de" href="/de/">
        </body></html>"#;
        let page = parse_html(html, "https://example.com/");
        assert_eq!(page.hreflang_tags.len(), 1);
        assert_eq!(page.hreflang_tags[0].lang, "en");
    }

    /// Zero-width characters have no rendered advance, so a stray BOM must not
    /// push a title toward the length and pixel thresholds.
    #[test]
    fn zero_width_characters_do_not_count_toward_length() {
        let plain = parse_html(
            "<html><head><title>ZeroWidthTitle Here</title></head><body>x</body></html>",
            "https://example.com/",
        );
        let zw = parse_html(
            "<html><head><title>Zero\u{200b}Width\u{200b}Title\u{200d} Here\u{feff}</title></head><body>x</body></html>",
            "https://example.com/",
        );
        assert_eq!(plain.title_length, zw.title_length);
        assert_eq!(plain.title_pixel_width, zw.title_pixel_width);
    }
}
