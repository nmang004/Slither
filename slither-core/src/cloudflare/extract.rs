use crate::cloudflare::{CloudflareClient, CloudflareError};
use serde::Serialize;

// ---------------------------------------------------------------------------
// SEO preset schema
// ---------------------------------------------------------------------------

/// Returns the built-in SEO extraction JSON schema for the Cloudflare
/// Browser Rendering `/json` endpoint.
pub fn seo_preset_schema() -> String {
    r#"{
    "type": "json_schema",
    "schema": {
        "type": "object",
        "properties": {
            "business_name": { "type": "string" },
            "business_type": { "type": "string" },
            "phone": { "type": "string" },
            "email": { "type": "string" },
            "address": { "type": "string" },
            "city": { "type": "string" },
            "state": { "type": "string" },
            "zip": { "type": "string" },
            "services": { "type": "array", "items": { "type": "string" } },
            "service_areas": { "type": "array", "items": { "type": "string" } },
            "social_profiles": { "type": "object" },
            "business_hours": { "type": "string" }
        }
    }
}"#
    .to_string()
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Extract the `result` object from a CF `/json` response.
///
/// Expected shape: `{"success": true, "result": { ... }}`
pub fn parse_extract_response(json_str: &str) -> Result<serde_json::Value, CloudflareError> {
    let val: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| CloudflareError::ApiError(e.to_string()))?;

    let result = val
        .get("result")
        .ok_or_else(|| CloudflareError::ApiError("missing result field in response".into()))?
        .clone();

    Ok(result)
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Format extracted SEO data as human-readable text.
pub fn format_seo_result(data: &serde_json::Value) -> String {
    let mut lines = Vec::new();

    if let Some(v) = data["business_name"].as_str() {
        if !v.is_empty() {
            lines.push(format!("  Business Name:  {}", v));
        }
    }
    if let Some(v) = data["business_type"].as_str() {
        if !v.is_empty() {
            lines.push(format!("  Business Type:  {}", v));
        }
    }
    if let Some(v) = data["phone"].as_str() {
        if !v.is_empty() {
            lines.push(format!("  Phone:          {}", v));
        }
    }
    if let Some(v) = data["email"].as_str() {
        if !v.is_empty() {
            lines.push(format!("  Email:          {}", v));
        }
    }

    // Compose address from address/city/state/zip
    let addr = data["address"].as_str().unwrap_or("");
    let city = data["city"].as_str().unwrap_or("");
    let state = data["state"].as_str().unwrap_or("");
    let zip = data["zip"].as_str().unwrap_or("");
    let mut addr_parts = Vec::new();
    if !addr.is_empty() {
        addr_parts.push(addr.to_string());
    }
    if !city.is_empty() {
        addr_parts.push(city.to_string());
    }
    if !state.is_empty() {
        addr_parts.push(state.to_string());
    }
    if !zip.is_empty() {
        addr_parts.push(zip.to_string());
    }
    let full_address = addr_parts.join(", ");
    if !full_address.is_empty() {
        lines.push(format!("  Address:        {}", full_address));
    }

    // Services (comma-joined array)
    if let Some(arr) = data["services"].as_array() {
        let items: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        if !items.is_empty() {
            lines.push(format!("  Services:       {}", items.join(", ")));
        }
    }

    // Service areas (comma-joined array)
    if let Some(arr) = data["service_areas"].as_array() {
        let items: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
        if !items.is_empty() {
            lines.push(format!("  Service Areas:  {}", items.join(", ")));
        }
    }

    // Business hours
    if let Some(v) = data["business_hours"].as_str() {
        if !v.is_empty() {
            lines.push(format!("  Hours:          {}", v));
        }
    }

    // Social profiles
    if let Some(obj) = data["social_profiles"].as_object() {
        if !obj.is_empty() {
            lines.push("  Social:".to_string());
            for (platform, url) in obj {
                if let Some(url_str) = url.as_str() {
                    lines.push(format!("    {}: {}", platform, url_str));
                }
            }
        }
    }

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ExtractRequest {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "waitForSelector")]
    wait_for_selector: Option<WaitForSelector>,
}

#[derive(Debug, Serialize)]
struct WaitForSelector {
    selector: String,
}

// ---------------------------------------------------------------------------
// API call
// ---------------------------------------------------------------------------

/// AI-powered data extraction via the Cloudflare Browser Rendering `/json`
/// endpoint.
///
/// - If `prompt` is Some, it is sent as the `prompt` field.
/// - If `schema` is Some, it is sent as the `response_format` field.
/// - If neither is provided, the built-in SEO preset schema is used as
///   `response_format`.
/// - `wait_for` optionally waits for a CSS selector before extraction.
pub async fn extract(
    client: &CloudflareClient,
    url: &str,
    prompt: Option<&str>,
    schema: Option<&str>,
    wait_for: Option<&str>,
) -> Result<serde_json::Value, CloudflareError> {
    let response_format = if let Some(s) = schema {
        let val: serde_json::Value =
            serde_json::from_str(s).map_err(|e| CloudflareError::ApiError(e.to_string()))?;
        Some(val)
    } else if prompt.is_none() {
        // No prompt and no schema — use the SEO preset
        let preset = seo_preset_schema();
        let val: serde_json::Value =
            serde_json::from_str(&preset).map_err(|e| CloudflareError::ApiError(e.to_string()))?;
        Some(val)
    } else {
        None
    };

    let body = ExtractRequest {
        url: url.to_string(),
        prompt: prompt.map(|s| s.to_string()),
        response_format,
        wait_for_selector: wait_for.map(|s| WaitForSelector {
            selector: s.to_string(),
        }),
    };

    let endpoint = format!("{}/json", client.base_url());

    let resp = client.http().post(&endpoint).json(&body).send().await?;

    let status = resp.status();

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(CloudflareError::AuthFailed);
    }

    let text = resp.text().await?;

    if !status.is_success() {
        return Err(CloudflareError::ApiError(format!(
            "HTTP {} — {}",
            status.as_u16(),
            text
        )));
    }

    parse_extract_response(&text)
}
