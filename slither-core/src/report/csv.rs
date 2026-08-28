use crate::models::crawl_result::CrawlResult;
use anyhow::Result;

pub fn generate_csv(result: &CrawlResult) -> Result<String> {
    let mut csv = String::new();

    // Header (31 columns)
    csv.push_str("url,status,title,title_length,meta_description,meta_description_length,");
    csv.push_str("h1_count,h1_text,word_count,response_time_ms,depth,canonical,");
    csv.push_str("is_https,internal_links,external_links,images,images_missing_alt,");
    csv.push_str("has_schema,schema_types,meta_robots,x_robots_tag,is_indexable,");
    csv.push_str("hreflang_count,readability_score,");
    csv.push_str("lcp_ms,inp_ms,cls,performance_score,");
    csv.push_str("script_count,render_blocking_scripts,console_error_count\n");

    for page in &result.pages {
        let h1_text = page.h1.join("; ");
        let schema_types = page.schema_types.join("; ");
        let meta_robots = page.meta_robots.as_deref().unwrap_or("");
        let x_robots_tag = page.x_robots_tag.as_deref().unwrap_or("");
        // Only meta_robots was exported, so a page noindexed by header looked
        // identical to an indexable one and a "filter for noindex" pass over the
        // spreadsheet silently missed it.
        //
        // The rule is the shared one — 2xx HTML that is not noindex — not merely
        // "not noindex". Testing only the robots directives marked every 404,
        // 500 and 403 Indexable: one crawl exported 8 indexable rows while
        // `slither sitemap`, which uses the shared rule, emitted 5.
        let is_indexable = crate::analysis::is_indexable_html(page);
        // Same rule the issue check and the crawl summary use: `data:` URI
        // placeholders and 1×1 pixels are decorative and correctly carry no alt
        // text. Counting them here put three different numbers for one metric in
        // one run — 306 in the CSV against 3 in the issue list for the same page.
        let imgs_missing_alt = page
            .images
            .iter()
            .filter(|i| i.alt.is_none() && i.needs_alt_text())
            .count();
        let readability = page
            .readability_score
            .map(|s| format!("{:.1}", s))
            .unwrap_or_default();
        let lcp = page.lcp_ms.map(|v| format!("{:.0}", v)).unwrap_or_default();
        let inp = page.inp_ms.map(|v| format!("{:.0}", v)).unwrap_or_default();
        let cls_val = page.cls.map(|v| format!("{:.3}", v)).unwrap_or_default();
        let perf_score = page
            .performance_score
            .map(|v| v.to_string())
            .unwrap_or_default();
        let script_count = page.scripts.len();
        let render_blocking = page
            .scripts
            .iter()
            .filter(|s| !s.is_async && !s.is_defer && !s.is_module)
            .count();
        let console_error_count = page.console_errors.len();

        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_escape(&page.url),
            page.status,
            csv_escape(page.title.as_deref().unwrap_or("")),
            page.title_length.unwrap_or(0),
            csv_escape(page.meta_description.as_deref().unwrap_or("")),
            page.meta_description_length.unwrap_or(0),
            page.h1.len(),
            csv_escape(&h1_text),
            page.word_count,
            page.response_time_ms,
            page.depth,
            csv_escape(page.canonical.as_deref().unwrap_or("")),
            page.is_https,
            page.internal_links.len(),
            page.external_links.len(),
            page.images.len(),
            imgs_missing_alt,
            !page.schema_types.is_empty(),
            csv_escape(&schema_types),
            csv_escape(meta_robots),
            csv_escape(x_robots_tag),
            is_indexable,
            page.hreflang_tags.len(),
            readability,
            lcp,
            inp,
            cls_val,
            perf_score,
            script_count,
            render_blocking,
            console_error_count,
        ));
    }

    Ok(csv)
}

fn csv_escape(value: &str) -> String {
    // Neutralize spreadsheet formula injection: a cell whose text begins with
    // =, +, -, @, or a leading tab/CR is treated as a formula by Excel/Sheets.
    // Crawled titles/anchors can contain `=HYPERLINK(...)` or `@SUM(...)`, so
    // prefix a single quote to force text interpretation. (RFC 4180 quoting
    // alone does not prevent this.)
    let needs_formula_guard = value.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let guarded = if needs_formula_guard {
        format!("'{value}")
    } else {
        value.to_string()
    };

    if guarded.contains(',')
        || guarded.contains('"')
        || guarded.contains('\n')
        || guarded.contains('\r')
    {
        format!("\"{}\"", guarded.replace('"', "\"\""))
    } else {
        guarded
    }
}

#[cfg(test)]
mod tests {
    use super::{csv_escape, generate_csv};
    use crate::models::config::CrawlConfig;
    use crate::models::crawl_result::{CrawlIssues, CrawlMetadata, CrawlResult, ExportSettings};
    use crate::models::page::PageData;

    fn page(url: &str, status: u16) -> PageData {
        let mut p = crate::crawler::parser::parse_html(
            "<html><head><title>T</title></head><body></body></html>",
            url,
        );
        p.status = status;
        p.content_type = Some("text/html".to_string());
        p
    }

    fn result_with(pages: Vec<PageData>) -> CrawlResult {
        let summary = crate::crawler::build_summary(&pages);
        CrawlResult {
            slither_version: "test".to_string(),
            crawl_metadata: CrawlMetadata {
                domain: "test.com".to_string(),
                seed_url: "https://test.com/".to_string(),
                crawl_date: "2026-01-01T00:00:00Z".to_string(),
                duration_ms: 0,
                pages_discovered: pages.len() as u32,
                pages_crawled: pages.len() as u32,
                pages_skipped_robots: 0,
                pages_errored: 0,
                settings: CrawlConfig::default(),
                backend: "local".to_string(),
            },
            export_settings: ExportSettings::default(),
            pages,
            issues: CrawlIssues { issues: Vec::new() },
            summary,
            robots_txt: None,
            sitemap_data: None,
        }
    }

    /// Column value for `column` on the row whose `url` cell matches.
    fn cell(csv: &str, url: &str, column: &str) -> String {
        let mut lines = csv.lines();
        let header: Vec<&str> = lines.next().expect("header").split(',').collect();
        let idx = header
            .iter()
            .position(|h| *h == column)
            .unwrap_or_else(|| panic!("no column {column}"));
        for line in lines {
            let fields: Vec<&str> = line.split(',').collect();
            if fields.first() == Some(&url) {
                return fields[idx].to_string();
            }
        }
        panic!("no row for {url}");
    }

    /// Regression: `is_indexable` tested only the robots directives, so every
    /// 404, 500 and 403 exported as Indexable. One crawl's CSV claimed 8
    /// indexable pages while `slither sitemap`, which applies the shared
    /// `analysis::is_indexable_html` rule, emitted 5 URLs for the same crawl.
    #[test]
    fn csv_does_not_mark_error_pages_indexable() {
        let pages = vec![
            page("https://test.com/ok", 200),
            page("https://test.com/gone", 404),
            page("https://test.com/boom", 500),
            page("https://test.com/denied", 403),
        ];
        let csv = generate_csv(&result_with(pages.clone())).unwrap();

        assert_eq!(cell(&csv, "https://test.com/ok", "is_indexable"), "true");
        for url in [
            "https://test.com/gone",
            "https://test.com/boom",
            "https://test.com/denied",
        ] {
            assert_eq!(
                cell(&csv, url, "is_indexable"),
                "false",
                "{url} is not indexable"
            );
        }

        // The count must equal the one `slither sitemap` would publish.
        let col = csv
            .lines()
            .next()
            .unwrap()
            .split(',')
            .position(|h| h == "is_indexable")
            .unwrap();
        let indexable = csv
            .lines()
            .skip(1)
            .filter(|l| l.split(',').nth(col) == Some("true"))
            .count();
        assert_eq!(
            indexable,
            crate::report::sitemap_gen::indexable_urls(&pages).len(),
            "the CSV and the generated sitemap must agree on indexability"
        );
    }

    /// A non-HTML 200 (a PDF, an image) is not an indexable HTML page either.
    #[test]
    fn csv_does_not_mark_non_html_responses_indexable() {
        let mut pdf = page("https://test.com/doc.pdf", 200);
        pdf.content_type = Some("application/pdf".to_string());
        let csv = generate_csv(&result_with(vec![pdf])).unwrap();
        assert_eq!(
            cell(&csv, "https://test.com/doc.pdf", "is_indexable"),
            "false"
        );
    }

    #[test]
    fn neutralizes_formula_leading_cells() {
        assert_eq!(
            csv_escape("=HYPERLINK(\"x\")"),
            "\"'=HYPERLINK(\"\"x\"\")\""
        );
        assert_eq!(csv_escape("+1+1"), "'+1+1");
        assert_eq!(csv_escape("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(csv_escape("-2"), "'-2");
    }

    #[test]
    fn leaves_plain_text_untouched() {
        assert_eq!(csv_escape("Hello World"), "Hello World");
        assert_eq!(csv_escape("Price: 5"), "Price: 5");
    }

    #[test]
    fn still_quotes_commas_and_quotes() {
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("she said \"hi\""), "\"she said \"\"hi\"\"\"");
    }
}
