use anyhow::{Context, Result};

use super::models::PageSpeedResult;
use super::PageSpeedClient;

const API_BASE: &str = "https://www.googleapis.com/pagespeedonline/v5/runPagespeed";

pub async fn fetch_pagespeed(client: &PageSpeedClient, url: &str) -> Result<PageSpeedResult> {
    client.wait_for_rate_limit().await;

    let mut req = client.http().get(API_BASE).query(&[
        ("url", url),
        ("strategy", client.strategy()),
        ("category", "performance"),
    ]);

    if let Some(key) = client.api_key() {
        req = req.query(&[("key", key)]);
    }

    // `reqwest::Error`'s Display includes the full request URL, and the API key
    // travels in the query string — so a transport failure would print the key
    // into logs (callers surface these at warn level). Strip the URL first.
    let resp = req
        .send()
        .await
        .map_err(|e| e.without_url())
        .context("PageSpeed API request failed")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        // Truncate by chars, not bytes, to avoid panicking on a multi-byte
        // boundary in a localized error message.
        let snippet: String = body.chars().take(200).collect();
        anyhow::bail!("PageSpeed API error ({}): {}", status, snippet);
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| e.without_url())
        .context("Failed to parse PageSpeed API response")?;

    parse_pagespeed_response(&json, url, client.strategy())
}

fn parse_pagespeed_response(
    json: &serde_json::Value,
    url: &str,
    strategy: &str,
) -> Result<PageSpeedResult> {
    let lighthouse = &json["lighthouseResult"];
    let audits = &lighthouse["audits"];

    // Default to 0 if field missing — API normally always returns this.
    // Round rather than truncate so 0.899 shows as 90 like the PSI UI.
    let performance_score = lighthouse["categories"]["performance"]["score"]
        .as_f64()
        .map(|s| (s * 100.0).round() as u32)
        .unwrap_or(0);

    // Default to 0 if field missing — API normally always returns this
    let lcp_ms = audits["largest-contentful-paint"]["numericValue"]
        .as_f64()
        .unwrap_or(0.0);
    let fcp_ms = audits["first-contentful-paint"]["numericValue"]
        .as_f64()
        .unwrap_or(0.0);
    let cls = audits["cumulative-layout-shift"]["numericValue"]
        .as_f64()
        .unwrap_or(0.0);
    let ttfb = audits["server-response-time"]["numericValue"]
        .as_f64()
        .unwrap_or(0.0);

    // INP comes from CrUX field data — navigation-mode Lighthouse has no INP
    // audit, so reading the lab audit always yielded 0.0 ("Good") for every
    // page. Read the real field metric (in ms) and leave it None when absent.
    let inp_ms = json["loadingExperience"]["metrics"]["INTERACTION_TO_NEXT_PAINT"]["percentile"]
        .as_f64()
        .or_else(|| {
            json["originLoadingExperience"]["metrics"]["INTERACTION_TO_NEXT_PAINT"]["percentile"]
                .as_f64()
        });

    Ok(PageSpeedResult {
        url: url.to_string(),
        performance_score,
        lcp_ms,
        inp_ms,
        cls,
        fcp_ms,
        ttfb_ms: ttfb,
        strategy: strategy.to_string(),
        source: "api".to_string(),
    })
}
