use slither_core::crawler::parser::parse_html;

fn load_fixture() -> String {
    std::fs::read_to_string("tests/fixtures/sample_page.html").expect("Failed to load fixture")
}

#[test]
fn test_parse_title() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert_eq!(
        page.title.as_deref(),
        Some("Mosquito Control Houston | Example Pest")
    );
}

#[test]
fn test_parse_meta_description() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert_eq!(
        page.meta_description.as_deref(),
        Some("Professional mosquito treatment and prevention in Houston TX.")
    );
}

#[test]
fn test_parse_meta_robots() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert_eq!(page.meta_robots.as_deref(), Some("index, follow"));
}

#[test]
fn test_parse_canonical() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert_eq!(
        page.canonical.as_deref(),
        Some("https://example-pest-control.com/mosquito-control")
    );
}

#[test]
fn test_parse_h1() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert_eq!(page.h1, vec!["Houston Mosquito Control Services"]);
}

#[test]
fn test_parse_headings() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    // headings is the full outline in document order, including h1.
    assert!(page.headings.len() >= 5); // h1, h2 x2, h3 x2
    assert_eq!(page.headings[0].level, 1);
    assert_eq!(page.headings[0].text, "Houston Mosquito Control Services");
    // The first h2 follows the h1.
    let first_h2 = page.headings.iter().find(|h| h.level == 2).unwrap();
    assert_eq!(first_h2.text, "Why Choose Us");
}

#[test]
fn test_parse_internal_links() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert_eq!(page.internal_links.len(), 2);
    assert_eq!(page.internal_links[0].anchor, "termite services");
    assert!(!page.internal_links[0].nofollow);
}

#[test]
fn test_parse_external_links() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert_eq!(page.external_links.len(), 1);
    assert!(page.external_links[0].nofollow);
    assert_eq!(page.external_links[0].anchor, "EPA guidelines");
}

#[test]
fn test_parse_images() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert_eq!(page.images.len(), 2);
    assert_eq!(
        page.images[0].alt.as_deref(),
        Some("Mosquito control technician")
    );
    assert_eq!(page.images[0].width, Some(1200));
    assert!(page.images[1].alt.is_none()); // missing alt
}

#[test]
fn test_parse_schema_types() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert!(page.schema_types.contains(&"Service".to_string()));
    assert!(page.schema_types.contains(&"LocalBusiness".to_string()));
}

#[test]
fn test_parse_og_tags() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert_eq!(
        page.og_tags.get("og:title").map(String::as_str),
        Some("Mosquito Control Houston")
    );
}

#[test]
fn test_parse_word_count() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    // Should count visible text only, not scripts/styles
    assert!(page.word_count > 20);
    assert!(!page.body_text.contains("console.log"));
    assert!(!page.body_text.contains("color: #333"));
}

#[test]
fn test_parse_content_hash() {
    let page = parse_html(
        &load_fixture(),
        "https://example-pest-control.com/mosquito-control",
    );
    assert!(!page.content_hash.is_empty());
    assert_eq!(page.content_hash.len(), 64); // SHA-256 hex
}

/// A-JSCOUNT: `<script>` blocks that hold data rather than code (JSON-LD
/// schema, Next.js `__NEXT_DATA__`) must not be counted as JavaScript, or
/// every schema-rich or SSR page looks like it ships excessive JS.
#[test]
fn test_script_extraction_skips_non_javascript_blocks() {
    let html = r#"<html><head>
        <script type="application/ld+json">{"@type":"Organization"}</script>
        <script type="application/json" id="__NEXT_DATA__">{"props":{}}</script>
        <script src="/app.js"></script>
        <script>console.log('inline');</script>
        <script type="text/javascript">var x = 1;</script>
        <script type="module">import a from 'b';</script>
        <script type="TEXT/JavaScript; charset=utf-8">var y = 2;</script>
      </head><body></body></html>"#;

    let page = slither_core::crawler::parser::parse_html(html, "https://example.com/");

    assert_eq!(
        page.scripts.len(),
        5,
        "only executable scripts should be captured, got: {:?}",
        page.scripts
            .iter()
            .map(|s| s.src.clone())
            .collect::<Vec<_>>()
    );
    assert!(
        page.scripts.iter().any(|s| s.is_module),
        "type=module is JavaScript"
    );
    // The JSON-LD block is still available as structured data.
    assert!(
        page.schema_types.iter().any(|t| t == "Organization"),
        "JSON-LD must still be parsed as structured data"
    );
}
