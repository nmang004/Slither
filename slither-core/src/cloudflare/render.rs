use crate::cloudflare::{CloudflareClient, CloudflareError};
use serde::Serialize;

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderRequest {
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    wait_for_selector: Option<WaitForSelector>,
    #[serde(skip_serializing_if = "Option::is_none")]
    goto_options: Option<GotoOptions>,
}

#[derive(Debug, Serialize)]
struct WaitForSelector {
    selector: String,
}

#[derive(Debug, Serialize)]
struct GotoOptions {
    timeout: u64,
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Extract the rendered HTML string from a CF `/content` JSON response.
///
/// Expected shape: `{"success": true, "result": "<html>..."}`
pub fn parse_render_response(json_str: &str) -> Result<String, CloudflareError> {
    let val: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| CloudflareError::ApiError(e.to_string()))?;

    let html = val["result"]
        .as_str()
        .ok_or_else(|| CloudflareError::ApiError("missing result field in response".into()))?;

    Ok(html.to_string())
}

// ---------------------------------------------------------------------------
// API call
// ---------------------------------------------------------------------------

/// Fetch the JS-rendered HTML of a page via the Cloudflare Browser Rendering
/// `/content` endpoint.
pub async fn render_page(
    client: &CloudflareClient,
    url: &str,
    wait_for: Option<&str>,
    timeout_ms: Option<u64>,
) -> Result<String, CloudflareError> {
    let body = RenderRequest {
        url: url.to_string(),
        wait_for_selector: wait_for.map(|s| WaitForSelector {
            selector: s.to_string(),
        }),
        goto_options: timeout_ms.map(|t| GotoOptions { timeout: t }),
    };

    let endpoint = format!("{}/content", client.base_url());

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

    parse_render_response(&text)
}
