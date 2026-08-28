use crate::models::config::CrawlConfig;
use crate::models::crawl_result::{CrawlResult, ExportSettings};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Paths to the files written by [`write_crawl_output`].
#[derive(Debug, Clone)]
pub struct OutputPaths {
    pub json: PathBuf,
    pub html: Option<PathBuf>,
    pub csv: Option<PathBuf>,
}

/// Controls how output files are generated. Built from CLI flags or a
/// [`CrawlConfig`].
#[derive(Debug, Clone)]
pub struct WriteConfig {
    pub output_path: Option<String>,
    pub generate_html: bool,
    pub generate_csv: bool,
    pub json_compact: bool,
    pub include_body_text: bool,
    pub summary_only: bool,
}

impl WriteConfig {
    /// Derive a `WriteConfig` from a [`CrawlConfig`].
    pub fn from_crawl_config(config: &CrawlConfig) -> Self {
        Self {
            output_path: config.output_path.clone(),
            generate_html: config.generate_html,
            generate_csv: config.generate_csv,
            json_compact: config.json_compact,
            include_body_text: config.include_body_text,
            summary_only: config.summary_only,
        }
    }
}

/// Derive the companion export path (CSV / HTML) for a JSON output path by
/// swapping the extension.
///
/// A naive `json_path.replace(".json", ".csv")` silently returns the input
/// unchanged when the user passes `--output myreport` (no extension), so every
/// artifact resolves to the same path and the last writer destroys the crawl
/// JSON. Swapping the extension always yields a distinct path; in the corner
/// case where it does not (`--output out.csv`), the extension is appended
/// instead so the JSON is never overwritten.
fn sibling_path(json_path: &str, extension: &str) -> PathBuf {
    let candidate = PathBuf::from(json_path).with_extension(extension);
    if candidate == Path::new(json_path) {
        PathBuf::from(format!("{json_path}.{extension}"))
    } else {
        candidate
    }
}

/// Apply export settings to the result and write JSON (and optionally CSV /
/// HTML) files. Returns the paths that were written so the caller can display
/// them.
pub fn write_crawl_output(
    result: &mut CrawlResult,
    write_config: &WriteConfig,
) -> Result<OutputPaths> {
    // Apply export settings
    result.export_settings = ExportSettings {
        include_body_text: write_config.include_body_text,
        summary_only: write_config.summary_only,
        format: if write_config.json_compact {
            "compact".to_string()
        } else {
            "pretty".to_string()
        },
    };

    let domain = &result.crawl_metadata.domain;

    // Determine JSON output path
    let json_path = write_config.output_path.clone().unwrap_or_else(|| {
        let date = &result.crawl_metadata.crawl_date[..10];
        format!("crawl-{}-{}.json", domain, date)
    });

    // Write JSON
    let json = crate::report::json::serialize_crawl_result(result)?;
    std::fs::write(&json_path, &json)?;

    // Write CSV if enabled
    let csv_path = if write_config.generate_csv {
        let path = sibling_path(&json_path, "csv");
        let csv_data = crate::report::csv::generate_csv(result)?;
        std::fs::write(&path, &csv_data)?;
        Some(path)
    } else {
        None
    };

    // Write HTML report if enabled
    let html_path = if write_config.generate_html {
        let path = sibling_path(&json_path, "html");
        let html = crate::report::html::render_html_report(result)?;
        std::fs::write(&path, &html)?;
        Some(path)
    } else {
        None
    };

    Ok(OutputPaths {
        json: PathBuf::from(json_path),
        html: html_path,
        csv: csv_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `--output myreport` (no extension) used to make the CSV and
    /// HTML paths identical to the JSON path, so the report silently
    /// overwrote the crawl data.
    #[test]
    fn sibling_paths_never_collide_with_an_extensionless_json_path() {
        let json = "myreport";
        let csv = sibling_path(json, "csv");
        let html = sibling_path(json, "html");

        assert_eq!(csv, PathBuf::from("myreport.csv"));
        assert_eq!(html, PathBuf::from("myreport.html"));
        assert_ne!(csv, PathBuf::from(json));
        assert_ne!(html, PathBuf::from(json));
    }

    #[test]
    fn sibling_paths_swap_a_json_extension() {
        assert_eq!(sibling_path("out.json", "csv"), PathBuf::from("out.csv"));
        assert_eq!(sibling_path("out.json", "html"), PathBuf::from("out.html"));
    }

    /// A dotted domain in the default filename must keep its dots — only the
    /// trailing extension is swapped.
    #[test]
    fn sibling_paths_preserve_dots_in_the_stem() {
        assert_eq!(
            sibling_path("crawl-example.com-2026-08-14.json", "html"),
            PathBuf::from("crawl-example.com-2026-08-14.html")
        );
    }

    /// If swapping the extension would land on the JSON path itself, append
    /// rather than overwrite.
    #[test]
    fn sibling_paths_append_when_swapping_would_collide() {
        assert_eq!(sibling_path("out.csv", "csv"), PathBuf::from("out.csv.csv"));
    }
}
