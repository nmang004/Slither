pub mod crawl;
pub mod render;

use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Errors from Playwright/Chrome browser interactions.
#[derive(Debug, thiserror::Error)]
pub enum PlaywrightError {
    #[error("Chrome/Chromium not found. Install Chrome or pass --chrome-path.")]
    ChromeNotFound,

    #[error("Failed to launch browser: {0}")]
    LaunchFailed(String),

    #[error("Browser error: {0}")]
    BrowserError(String),

    #[error("Page error: {0}")]
    PageError(String),

    #[error("Navigation timeout")]
    Timeout,
}

impl From<chromiumoxide::error::CdpError> for PlaywrightError {
    fn from(e: chromiumoxide::error::CdpError) -> Self {
        PlaywrightError::BrowserError(e.to_string())
    }
}

/// High-level wrapper around a chromiumoxide `Browser` instance.
///
/// Manages the browser process lifecycle and the background handler task
/// that drives the WebSocket connection. The `Browser` is wrapped in an
/// `Arc` so it can be shared cheaply across concurrent rendering tasks.
pub struct PlaywrightClient {
    browser: Arc<Browser>,
    _handler: JoinHandle<()>,
}

impl PlaywrightClient {
    /// Launch a new Chrome/Chromium instance.
    ///
    /// If `chrome_path` is `None`, auto-detection is attempted via
    /// [`detect_chrome`].
    pub async fn launch(
        chrome_path: Option<&str>,
        headless: bool,
    ) -> Result<Self, PlaywrightError> {
        let path = match chrome_path {
            Some(p) => PathBuf::from(p),
            None => detect_chrome().ok_or(PlaywrightError::ChromeNotFound)?,
        };

        let mut builder = BrowserConfig::builder()
            .chrome_executable(path)
            .arg("--disable-gpu")
            .arg("--disable-dev-shm-usage");

        // --no-sandbox disables the Chrome sandbox — a renderer exploit on an
        // attacker-controlled page then has full access to the user's account.
        // Only apply it where the sandbox can't run (root, common in Docker/CI)
        // or when explicitly opted in via SLITHER_NO_SANDBOX=1.
        let force_no_sandbox = matches!(
            std::env::var("SLITHER_NO_SANDBOX").ok().as_deref(),
            Some("1") | Some("true") | Some("TRUE")
        );
        let is_root = {
            #[cfg(unix)]
            {
                // SAFETY: geteuid is always safe to call.
                unsafe { libc::geteuid() == 0 }
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        if force_no_sandbox || is_root {
            tracing::warn!("Launching Chrome with --no-sandbox (root or SLITHER_NO_SANDBOX set)");
            builder = builder.arg("--no-sandbox");
        }

        if !headless {
            builder = builder.with_head();
        }

        let config = builder
            .build()
            .map_err(|e| PlaywrightError::LaunchFailed(e.to_string()))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| PlaywrightError::LaunchFailed(e.to_string()))?;

        let handle = tokio::spawn(async move {
            while let Some(h) = handler.next().await {
                if h.is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            browser: Arc::new(browser),
            _handler: handle,
        })
    }

    /// Access the underlying `Browser` instance (as an `Arc`).
    pub fn browser(&self) -> Arc<Browser> {
        Arc::clone(&self.browser)
    }

    /// Gracefully close the browser.
    pub async fn close(self) {
        match Arc::try_unwrap(self.browser) {
            Ok(mut browser) => {
                let _ = browser.close().await;
            }
            Err(_arc) => {
                // Browser still has references — it will be cleaned up on drop.
                tracing::debug!("Browser has outstanding references, will be cleaned up on drop");
            }
        }
        self._handler.abort();
    }
}

/// Auto-detect a Chrome or Chromium executable on the current platform.
///
/// Checks well-known installation paths on macOS, Linux, and Windows,
/// then falls back to `which` for PATH-based discovery.
pub fn detect_chrome() -> Option<PathBuf> {
    // Platform-specific well-known paths
    let candidates: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
            PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
            PathBuf::from(
                "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            ),
        ]
    } else if cfg!(target_os = "windows") {
        let program_files = std::env::var("PROGRAMFILES").unwrap_or_default();
        let program_files_x86 = std::env::var("PROGRAMFILES(X86)").unwrap_or_default();
        let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
        vec![
            PathBuf::from(format!(
                r"{}\Google\Chrome\Application\chrome.exe",
                program_files
            )),
            PathBuf::from(format!(
                r"{}\Google\Chrome\Application\chrome.exe",
                program_files_x86
            )),
            PathBuf::from(format!(
                r"{}\Google\Chrome\Application\chrome.exe",
                local_app_data
            )),
            PathBuf::from(format!(
                r"{}\Chromium\Application\chrome.exe",
                program_files
            )),
        ]
    } else {
        // Linux
        vec![
            PathBuf::from("/usr/bin/google-chrome"),
            PathBuf::from("/usr/bin/google-chrome-stable"),
            PathBuf::from("/usr/bin/chromium"),
            PathBuf::from("/usr/bin/chromium-browser"),
            PathBuf::from("/snap/bin/chromium"),
        ]
    };

    // Check well-known paths first
    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    // Fall back to `which` for PATH-based discovery
    for name in &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ] {
        if let Ok(path) = which::which(name) {
            return Some(path);
        }
    }

    None
}
