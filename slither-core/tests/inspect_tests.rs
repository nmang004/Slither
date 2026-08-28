mod inspect_tests {
    use slither_core::inspect::{format_compare_table, format_inspect_output};
    use slither_core::models::issue::{Issue, IssueCategory, IssueUrl, Severity};

    fn make_test_issues() -> Vec<Issue> {
        vec![
            Issue {
                category: IssueCategory::Images,
                check: "missing_alt".to_string(),
                display_name: "Missing alt text on 2 images".to_string(),
                severity: Severity::Warning,
                description: String::new(),
                guidance: String::new(),
                urls: vec![IssueUrl {
                    url: "https://example.com/".to_string(),
                    detail: None,
                }],
            },
            Issue {
                category: IssueCategory::Security,
                check: "no_csp".to_string(),
                display_name: "No Content-Security-Policy header".to_string(),
                severity: Severity::Info,
                description: String::new(),
                guidance: String::new(),
                urls: vec![],
            },
        ]
    }

    #[test]
    fn test_format_inspect_output_contains_page_data() {
        let page = slither_core::crawler::parser::parse_html(
            "<html><head><title>Test Page</title><meta name=\"description\" content=\"A test.\"></head><body><h1>Hello</h1><p>Content</p></body></html>",
            "https://example.com/",
        );
        let issues = make_test_issues();
        let output = format_inspect_output("https://example.com/", &page, &issues);
        assert!(output.contains("slither inspect"));
        assert!(output.contains("Test Page"));
        assert!(output.contains("Issues (2)"));
        assert!(output.contains("Missing alt text"));
    }

    #[test]
    fn test_format_inspect_output_no_issues() {
        let page = slither_core::crawler::parser::parse_html(
            "<html><head><title>Test</title></head><body></body></html>",
            "https://example.com/",
        );
        let output = format_inspect_output("https://example.com/", &page, &[]);
        assert!(output.contains("None found"));
    }

    #[test]
    fn test_format_compare_table_shows_delta() {
        let static_page = slither_core::crawler::parser::parse_html(
            "<html><head></head><body></body></html>",
            "https://example.com/",
        );
        let mut rendered_page = slither_core::crawler::parser::parse_html(
            "<html><head><title>Rendered</title></head><body><h1>Hello</h1><p>Lots of content words here for the test to check word count delta.</p></body></html>",
            "https://example.com/",
        );
        // Ensure word count differs
        rendered_page.word_count = 842;

        let output = format_compare_table(
            "https://example.com/",
            &static_page,
            &rendered_page,
            &[],
            &make_test_issues(),
        );
        assert!(output.contains("compare"));
        assert!(output.contains("Static"));
        assert!(output.contains("Rendered"));
        assert!(output.contains("842"));
    }
}
