#[cfg(feature = "cloudflare")]
mod cloudflare_tests {
    use slither_core::cloudflare::CloudflareClient;

    #[test]
    fn test_from_explicit_credentials() {
        let client = CloudflareClient::new(
            Some("test-account-id".to_string()),
            Some("test-api-token".to_string()),
        );
        assert!(client.is_some());
        let client = client.unwrap();
        assert_eq!(client.account_id(), "test-account-id");
    }

    #[test]
    #[serial_test::serial]
    fn test_missing_credentials_returns_none() {
        let account_backup = std::env::var("CLOUDFLARE_ACCOUNT_ID").ok();
        let token_backup = std::env::var("CLOUDFLARE_API_TOKEN").ok();
        std::env::remove_var("CLOUDFLARE_ACCOUNT_ID");
        std::env::remove_var("CLOUDFLARE_API_TOKEN");

        let client = CloudflareClient::new(None, None);
        assert!(client.is_none());

        if let Some(v) = account_backup {
            std::env::set_var("CLOUDFLARE_ACCOUNT_ID", v);
        }
        if let Some(v) = token_backup {
            std::env::set_var("CLOUDFLARE_API_TOKEN", v);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_partial_credentials_returns_none() {
        let token_backup = std::env::var("CLOUDFLARE_API_TOKEN").ok();
        std::env::remove_var("CLOUDFLARE_API_TOKEN");

        let client = CloudflareClient::new(Some("test-account-id".to_string()), None);
        assert!(client.is_none());

        if let Some(v) = token_backup {
            std::env::set_var("CLOUDFLARE_API_TOKEN", v);
        }
    }

    #[test]
    fn test_base_url_format() {
        let client =
            CloudflareClient::new(Some("abc123".to_string()), Some("token456".to_string()))
                .unwrap();
        assert_eq!(
            client.base_url(),
            "https://api.cloudflare.com/client/v4/accounts/abc123/browser-rendering"
        );
    }

    #[test]
    fn test_not_configured_message() {
        let msg = CloudflareClient::not_configured_message();
        assert!(msg.contains("slither setup cloudflare"));
        assert!(msg.contains("free tier"));
    }
}
#[cfg(feature = "cloudflare")]
mod cf_render_tests {
    use slither_core::cloudflare::render::parse_render_response;

    #[test]
    fn test_parse_render_response() {
        let json = r#"{"success": true, "result": "<html><body><h1>Rendered</h1></body></html>"}"#;
        let html = parse_render_response(json).unwrap();
        assert!(html.contains("<h1>Rendered</h1>"));
    }
}

#[cfg(feature = "cloudflare")]
mod cf_extract_tests {
    use slither_core::cloudflare::extract::{
        format_seo_result, parse_extract_response, seo_preset_schema,
    };

    #[test]
    fn test_seo_preset_schema_has_required_fields() {
        let schema = seo_preset_schema();
        let val: serde_json::Value = serde_json::from_str(&schema).unwrap();
        let props = &val["schema"]["properties"];
        assert!(props["business_name"].is_object());
        assert!(props["phone"].is_object());
        assert!(props["services"].is_object());
        assert!(props["service_areas"].is_object());
    }

    #[test]
    fn test_parse_extract_response() {
        let json = r#"{"success": true, "result": {"business_name": "Joe's Plumbing", "phone": "(713) 555-0147"}}"#;
        let result = parse_extract_response(json).unwrap();
        assert_eq!(result["business_name"], "Joe's Plumbing");
        assert_eq!(result["phone"], "(713) 555-0147");
    }

    #[test]
    fn test_format_seo_result() {
        let json = serde_json::json!({
            "business_name": "Joe's Plumbing",
            "business_type": "Plumber",
            "phone": "(713) 555-0147",
            "services": ["Drain Cleaning", "Water Heater Repair"],
            "service_areas": ["Houston", "Katy"]
        });
        let formatted = format_seo_result(&json);
        assert!(formatted.contains("Joe's Plumbing"));
        assert!(formatted.contains("Plumber"));
        assert!(formatted.contains("(713) 555-0147"));
        assert!(formatted.contains("Drain Cleaning"));
        assert!(formatted.contains("Houston"));
    }
}

#[test]
fn test_crawl_config_default_backend_is_local() {
    let config = slither_core::CrawlConfig::default();
    assert_eq!(config.backend, "local");
    assert!(config.cf_account_id.is_none());
    assert!(config.cf_api_token.is_none());
    assert!(!config.skip_header_check);
}

mod head_request_tests {
    use slither_core::crawler::head_requests::merge_head_response;

    #[test]
    fn test_merge_head_response_fills_security_headers() {
        let mut page = slither_core::crawler::parser::parse_html(
            "<html><head><title>Test</title></head><body></body></html>",
            "https://example.com/",
        );
        page.status = 200;
        page.response_time_ms = 0;

        let headers = vec![
            (
                "strict-transport-security".to_string(),
                "max-age=31536000".to_string(),
            ),
            (
                "content-security-policy".to_string(),
                "default-src 'self'".to_string(),
            ),
            ("x-frame-options".to_string(), "DENY".to_string()),
        ];

        merge_head_response(&mut page, &headers, 42);

        assert!(page.security_headers.has_hsts);
        assert!(page.security_headers.has_csp);
        assert!(page.security_headers.has_x_frame_options);
        assert_eq!(page.response_time_ms, 42);
    }

    #[test]
    fn test_merge_head_response_with_empty_headers() {
        let mut page = slither_core::crawler::parser::parse_html(
            "<html><head><title>Test</title></head><body></body></html>",
            "https://example.com/",
        );
        page.status = 200;
        page.response_time_ms = 0;

        merge_head_response(&mut page, &[], 15);

        assert!(!page.security_headers.has_hsts);
        assert!(!page.security_headers.has_csp);
        assert_eq!(page.response_time_ms, 15);
    }
}
