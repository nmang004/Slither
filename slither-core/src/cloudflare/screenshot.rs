use crate::cloudflare::{CloudflareClient, CloudflareError};
use serde::Serialize;

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Configuration for a screenshot capture.
pub struct ScreenshotConfig {
    pub url: String,
    pub full_page: bool,
    pub format: String,
    pub quality: Option<u32>,
    pub selector: Option<String>,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            full_page: false,
            format: "png".to_string(),
            quality: None,
            selector: None,
            viewport_width: 1920,
            viewport_height: 1080,
        }
    }
}

// ---------------------------------------------------------------------------
// Request types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotRequest {
    url: String,
    screenshot_options: ScreenshotOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    selector: Option<String>,
    viewport: Viewport,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotOptions {
    full_page: bool,
    #[serde(rename = "type")]
    format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    quality: Option<u32>,
}

#[derive(Debug, Serialize)]
struct Viewport {
    width: u32,
    height: u32,
}

// ---------------------------------------------------------------------------
// API call
// ---------------------------------------------------------------------------

/// Take a screenshot of a page via the Cloudflare Browser Rendering
/// `/screenshot` endpoint. Returns the raw image bytes.
pub async fn take_screenshot(
    client: &CloudflareClient,
    config: &ScreenshotConfig,
) -> Result<Vec<u8>, CloudflareError> {
    let body = ScreenshotRequest {
        url: config.url.clone(),
        screenshot_options: ScreenshotOptions {
            full_page: config.full_page,
            format: config.format.clone(),
            quality: config.quality,
        },
        selector: config.selector.clone(),
        viewport: Viewport {
            width: config.viewport_width,
            height: config.viewport_height,
        },
    };

    let endpoint = format!("{}/screenshot", client.base_url());

    let resp = client.http().post(&endpoint).json(&body).send().await?;

    let status = resp.status();

    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(CloudflareError::AuthFailed);
    }

    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(CloudflareError::ApiError(format!(
            "HTTP {} — {}",
            status.as_u16(),
            text
        )));
    }

    let bytes = resp.bytes().await?;
    Ok(bytes.to_vec())
}
