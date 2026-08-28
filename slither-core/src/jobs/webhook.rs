use std::sync::{Arc, Mutex};

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

/// Ceiling on persistent webhook registrations. Each one adds an outbound
/// request to every job event, so the table is capped rather than unbounded.
pub const MAX_WEBHOOKS: i64 = 100;

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Check a webhook URL without registering it.
///
/// Public so an API handler can reject a bad URL *before* it commits anything
/// else. Registering the job first and validating afterwards turned a client
/// typo into a 500 plus a queued job the caller never got an id for — a queue
/// slot that could never be reclaimed.
pub fn validate_webhook_url(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).map_err(|_| anyhow::anyhow!("Invalid webhook URL: {url}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => anyhow::bail!(
            "Unsupported webhook URL scheme: {scheme}. Only http and https are allowed."
        ),
    }

    // SSRF protection: reject obvious private/loopback destinations at
    // registration time. The authoritative check (with DNS resolution) runs
    // again at dispatch time in `dispatch_webhooks`, closing the gap where a
    // hostname resolves to a private IP or a public URL redirects internally.
    if !crate::net_guard::private_targets_allowed() {
        if let Some(host) = parsed.host_str() {
            let stripped = host.trim_start_matches('[').trim_end_matches(']');
            let is_blocked = crate::net_guard::is_loopback_name(host)
                || stripped
                    .parse::<std::net::IpAddr>()
                    .map(|ip| crate::net_guard::is_blocked_ip(&ip))
                    .unwrap_or(false);
            if is_blocked {
                anyhow::bail!("Webhook URL must not target private or loopback addresses: {host}");
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Webhook {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookPayload {
    pub event: String,
    pub timestamp: String,
    pub job: serde_json::Value,
}

// ---------------------------------------------------------------------------
// WebhookManager
// ---------------------------------------------------------------------------

pub struct WebhookManager {
    conn: Arc<Mutex<Connection>>,
}

impl WebhookManager {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Register a persistent webhook that fires on the specified events.
    pub fn register(&self, url: &str, events: &[String]) -> Result<Webhook> {
        validate_webhook_url(url)?;
        let id = Uuid::new_v4().to_string();
        let created_at = Utc::now().to_rfc3339();
        let events_json = serde_json::to_string(events)?;

        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        // Bound the table: every registration multiplies the outbound requests
        // fired on each job event, so an unbounded list is both a storage and a
        // fan-out problem.
        let existing: i64 =
            conn.query_row("SELECT COUNT(*) FROM webhooks", [], |row| row.get(0))?;
        if existing >= MAX_WEBHOOKS {
            anyhow::bail!(
                "Too many registered webhooks ({existing}); remove one before adding another"
            );
        }

        conn.execute(
            "INSERT INTO webhooks (id, url, events, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, url, events_json, created_at],
        )?;

        Ok(Webhook {
            id,
            url: url.to_string(),
            events: events.to_vec(),
            created_at,
        })
    }

    /// How many persistent webhooks are registered, so a caller can report a
    /// full table as a client error rather than an internal one.
    pub fn count(&self) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Ok(conn.query_row("SELECT COUNT(*) FROM webhooks", [], |row| row.get(0))?)
    }

    /// List all persistent webhooks.
    pub fn list(&self) -> Result<Vec<Webhook>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare("SELECT id, url, events, created_at FROM webhooks ORDER BY created_at DESC")?;

        let rows = stmt.query_map([], |row| {
            let events_str: String = row.get(2)?;
            Ok(Webhook {
                id: row.get(0)?,
                url: row.get(1)?,
                events: serde_json::from_str(&events_str).unwrap_or_default(),
                created_at: row.get(3)?,
            })
        })?;

        let mut webhooks = Vec::new();
        for row in rows {
            webhooks.push(row?);
        }
        Ok(webhooks)
    }

    /// Delete a persistent webhook by ID. Returns true if a row was deleted.
    pub fn delete(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let deleted = conn.execute("DELETE FROM webhooks WHERE id = ?1", rusqlite::params![id])?;
        Ok(deleted > 0)
    }

    /// Register a one-shot webhook for a specific job.
    pub fn register_job_webhook(&self, job_id: &str, url: &str) -> Result<()> {
        validate_webhook_url(url)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO job_webhooks (job_id, url, fired) VALUES (?1, ?2, 0)",
            rusqlite::params![job_id, url],
        )?;
        Ok(())
    }

    /// Get all webhook URLs that should receive a notification for a given
    /// job event. This includes:
    /// - Persistent webhooks whose `events` array contains the event name
    /// - Unfired one-shot webhooks attached to the job (marked as fired after retrieval)
    pub fn get_urls_for_event(&self, job_id: &str, event: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut urls = Vec::new();

        // Persistent webhooks matching the event
        {
            let mut stmt = conn.prepare("SELECT url, events FROM webhooks")?;
            let rows = stmt.query_map([], |row| {
                let url: String = row.get(0)?;
                let events_str: String = row.get(1)?;
                Ok((url, events_str))
            })?;

            for row in rows {
                let (url, events_str) = row?;
                let events: Vec<String> = serde_json::from_str(&events_str).unwrap_or_default();
                if events.iter().any(|e| e == event) {
                    urls.push(url);
                }
            }
        }

        // One-shot webhooks for this job — only fire on terminal events to avoid
        // consuming the one-shot prematurely on intermediate events like "job.running".
        let is_terminal = matches!(event, "job.completed" | "job.failed" | "job.cancelled");
        if is_terminal {
            let mut stmt = conn
                .prepare("SELECT rowid, url FROM job_webhooks WHERE job_id = ?1 AND fired = 0")?;
            let rows = stmt.query_map(rusqlite::params![job_id], |row| {
                let rowid: i64 = row.get(0)?;
                let url: String = row.get(1)?;
                Ok((rowid, url))
            })?;

            let mut rowids = Vec::new();
            for row in rows {
                let (rowid, url) = row?;
                urls.push(url);
                rowids.push(rowid);
            }

            // Mark one-shot webhooks as fired
            for rowid in rowids {
                conn.execute(
                    "UPDATE job_webhooks SET fired = 1 WHERE rowid = ?1",
                    rusqlite::params![rowid],
                )?;
            }
        }

        Ok(urls)
    }
}

// ---------------------------------------------------------------------------
// Async dispatch
// ---------------------------------------------------------------------------

/// Fire-and-forget webhook delivery. Spawns a tokio task per URL that POSTs
/// the JSON payload with a 5-second timeout. On failure, retries up to 3
/// times with exponential backoff (2s, 4s, 8s).
pub async fn dispatch_webhooks(urls: Vec<String>, payload: WebhookPayload) {
    let payload_json = match serde_json::to_string(&payload) {
        Ok(json) => Arc::new(json),
        Err(e) => {
            warn!("Failed to serialize webhook payload: {e}");
            return;
        }
    };

    for url in urls {
        let json = Arc::clone(&payload_json);
        tokio::spawn(async move {
            // Authoritative SSRF check with DNS resolution, immediately before
            // delivery. Redirects are disabled so a validated public URL cannot
            // 302 into an internal address.
            if let Err(msg) = crate::net_guard::check_url_allowed(&url).await {
                warn!("Refusing webhook delivery to {url}: {msg}");
                return;
            }

            // The guarded resolver re-checks the addresses the connector
            // actually dials, so a rebinding answer cannot land a retry on an
            // internal host after the check above passed.
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .dns_resolver(std::sync::Arc::new(crate::net_guard::GuardedResolver))
                .build()
                .unwrap_or_default();

            let delays = [2u64, 4, 8];
            let mut attempts = 0u32;

            loop {
                let result = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .body(json.as_ref().clone())
                    .send()
                    .await;

                match result {
                    Ok(resp) if resp.status().is_success() => break,
                    Ok(resp) => {
                        warn!("Webhook POST to {} returned status {}", url, resp.status());
                    }
                    Err(e) => {
                        warn!("Webhook POST to {} failed: {e}", url);
                    }
                }

                if attempts as usize >= delays.len() {
                    warn!(
                        "Webhook delivery to {} failed after {} retries, giving up",
                        url,
                        delays.len()
                    );
                    break;
                }

                tokio::time::sleep(std::time::Duration::from_secs(delays[attempts as usize])).await;
                attempts += 1;
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::db::init_db;

    fn setup() -> WebhookManager {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        WebhookManager::new(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn test_register_and_list() {
        let mgr = setup();

        let events = vec!["job.completed".to_string(), "job.failed".to_string()];
        let wh = mgr.register("https://example.com/hook", &events).unwrap();

        assert_eq!(wh.url, "https://example.com/hook");
        assert_eq!(wh.events, events);
        assert!(!wh.id.is_empty());

        let list = mgr.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].url, "https://example.com/hook");
        assert_eq!(list[0].events, events);
    }

    #[test]
    fn test_delete() {
        let mgr = setup();

        let wh = mgr
            .register("https://example.com/hook", &["job.completed".to_string()])
            .unwrap();

        assert_eq!(mgr.list().unwrap().len(), 1);
        assert!(mgr.delete(&wh.id).unwrap());
        assert_eq!(mgr.list().unwrap().len(), 0);

        // Deleting again returns false
        assert!(!mgr.delete(&wh.id).unwrap());
    }

    #[test]
    fn test_get_urls_for_event_persistent() {
        let mgr = setup();

        mgr.register(
            "https://a.com/hook",
            &["job.completed".to_string(), "job.failed".to_string()],
        )
        .unwrap();

        mgr.register("https://b.com/hook", &["job.failed".to_string()])
            .unwrap();

        // "job.completed" should match only the first
        let urls = mgr
            .get_urls_for_event("some-job-id", "job.completed")
            .unwrap();
        assert_eq!(urls, vec!["https://a.com/hook"]);

        // "job.failed" should match both
        let mut urls = mgr.get_urls_for_event("some-job-id", "job.failed").unwrap();
        urls.sort();
        assert_eq!(urls, vec!["https://a.com/hook", "https://b.com/hook"]);
    }

    #[test]
    fn test_one_shot_webhook_fires_once() {
        let mgr = setup();

        // We need a job row to satisfy the foreign key.
        // Insert a minimal job directly.
        {
            let conn = mgr.conn.lock().unwrap_or_else(|e| e.into_inner());
            conn.execute(
                "INSERT INTO jobs (id, type, status, domain, url, config, created_at)
                 VALUES ('job-1', 'crawl', 'running', 'example.com', 'https://example.com', '{}', '2026-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        mgr.register_job_webhook("job-1", "https://oneshot.com/hook")
            .unwrap();

        // First call should return the one-shot URL
        let urls = mgr.get_urls_for_event("job-1", "job.completed").unwrap();
        assert_eq!(urls, vec!["https://oneshot.com/hook"]);

        // Second call should return empty (already fired)
        let urls = mgr.get_urls_for_event("job-1", "job.completed").unwrap();
        assert!(urls.is_empty());
    }
}
