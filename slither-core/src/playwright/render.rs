use std::sync::{Arc, Mutex};
use std::time::Duration;

use chromiumoxide::browser::Browser;
use chromiumoxide::cdp::js_protocol::runtime::EventExceptionThrown;
use futures::StreamExt;

use super::PlaywrightError;

/// Navigate to a URL via Chrome and return the fully-rendered HTML plus any
/// JavaScript console errors (uncaught exceptions) thrown during rendering.
///
/// After the initial navigation completes, an extra `wait_ms` sleep is
/// applied to give client-side JavaScript time to settle (SPA hydration,
/// lazy-loaded content, etc.).
pub async fn render_page(
    browser: &Browser,
    url: &str,
    wait_ms: u64,
    timeout_secs: u64,
) -> Result<(String, Vec<String>), PlaywrightError> {
    // Open a blank page first so the exception listener is attached before
    // navigation — otherwise early errors during load are missed.
    let page = browser.new_page("about:blank").await.map_err(|e| {
        PlaywrightError::PageError(format!("failed to open page for {}: {}", url, e))
    })?;

    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // Best-effort listener attachment: if it fails, continue without console
    // capture rather than aborting the render.
    let listener_task = match page.event_listener::<EventExceptionThrown>().await {
        Ok(mut stream) => {
            let errors_ref = errors.clone();
            Some(tokio::spawn(async move {
                while let Some(event) = stream.next().await {
                    let msg = event.exception_details.text.clone();
                    if let Ok(mut guard) = errors_ref.lock() {
                        guard.push(msg);
                    }
                }
            }))
        }
        Err(e) => {
            tracing::warn!("Failed to register exception listener: {}", e);
            None
        }
    };

    // Navigate to the target URL
    if let Err(e) = page.goto(url).await {
        let _ = page.close().await;
        if let Some(t) = listener_task {
            t.abort();
        }
        return Err(PlaywrightError::PageError(format!(
            "navigation failed for {}: {}",
            url, e
        )));
    }

    // Wait for the load event, bounded by a timeout. chromiumoxide's
    // wait_for_navigation resolves off an unbounded oneshot, so a page whose
    // load event never settles would otherwise hang the whole crawl forever.
    let nav = tokio::time::timeout(
        Duration::from_secs(timeout_secs.max(1)),
        page.wait_for_navigation(),
    )
    .await;
    match nav {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            let _ = page.close().await;
            if let Some(t) = listener_task {
                t.abort();
            }
            return Err(PlaywrightError::PageError(format!(
                "navigation failed for {}: {}",
                url, e
            )));
        }
        Err(_elapsed) => {
            let _ = page.close().await;
            if let Some(t) = listener_task {
                t.abort();
            }
            return Err(PlaywrightError::Timeout);
        }
    }

    // Extra settle time for JS rendering
    if wait_ms > 0 {
        tokio::time::sleep(Duration::from_millis(wait_ms)).await;
    }

    let html = page.content().await.map_err(|e| {
        PlaywrightError::PageError(format!("failed to get content for {}: {}", url, e))
    })?;

    // Stop the listener and snapshot collected errors
    if let Some(t) = listener_task {
        t.abort();
    }
    let collected = errors.lock().map(|g| g.clone()).unwrap_or_default();

    // Best-effort close; ignore errors (tab may already be gone)
    let _ = page.close().await;

    Ok((html, collected))
}
