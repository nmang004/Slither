use std::sync::Arc;

use serde_json::json;
use tokio::sync::{mpsc, Semaphore};
use tracing::{error, info, warn};

use slither_core::jobs::{JobStatus, WebhookManager, WebhookPayload};
use slither_core::models::CrawlEvent;
use slither_core::report;
use slither_core::{CrawlConfig, PipelineInput};

use crate::AppState;

// ---------------------------------------------------------------------------
// Execute a crawl job in the background
// ---------------------------------------------------------------------------

pub async fn execute_crawl_job(
    state: Arc<AppState>,
    job_id: String,
    config: CrawlConfig,
    semaphore: Arc<Semaphore>,
) {
    // Acquire semaphore permit — limits concurrent crawls.
    let _permit = match semaphore.acquire().await {
        Ok(permit) => permit,
        Err(_) => {
            error!(job_id = %job_id, "Semaphore closed, cannot start crawl");
            let _ = state.job_manager.set_error(&job_id, "Semaphore closed");
            let _ = state.job_manager.update_status(&job_id, JobStatus::Failed);
            return;
        }
    };

    // Mark job as running. The transition is guarded against terminal states,
    // so a `false` here means the job was cancelled while it waited for a
    // semaphore permit — honor that and never start the crawl.
    match state.job_manager.update_status(&job_id, JobStatus::Running) {
        Ok(true) => {}
        Ok(false) => {
            info!(job_id = %job_id, "Job is no longer queued (cancelled or finished); skipping crawl");
            // Still a terminal outcome for whoever is waiting on this job. It
            // used to return silently, which left a one-shot `webhook_url`
            // registration unfired forever.
            fire_terminal_webhook(&state, &job_id).await;
            return;
        }
        Err(e) => {
            error!(job_id = %job_id, "Failed to mark job as running: {e}");
            return;
        }
    }
    fire_webhook(&state, &job_id, "job.running").await;

    info!(job_id = %job_id, url = %config.seed_url, "Starting crawl job");

    // Create event channel for progress updates.
    let (event_tx, mut event_rx) = mpsc::channel::<CrawlEvent>(256);

    // Spawn a listener task that forwards QueueUpdate events to job progress.
    let progress_state = Arc::clone(&state);
    let progress_job_id = job_id.clone();
    let progress_listener = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if let CrawlEvent::QueueUpdate {
                crawled,
                queued,
                estimated_total,
            } = event
            {
                let progress = json!({
                    "pages_crawled": crawled,
                    "pages_queued": queued,
                    "estimated_total": estimated_total,
                });
                if let Err(e) = progress_state
                    .job_manager
                    .update_progress(&progress_job_id, progress)
                {
                    warn!(job_id = %progress_job_id, "Failed to update progress: {e}");
                }
            }
        }
    });

    // TODO: Cancelling an *already running* crawl only updates the DB status and
    // does not abort the in-flight task. A proper fix would require passing a
    // CancellationToken per job and wiring it into the crawl loop, which is a
    // bigger refactor. (Cancelling a job that is still queued is handled: the
    // guarded Running transition above refuses to start it.)

    // Spawn the actual crawl and wait for the result.
    let crawl_config = config.clone();
    let crawl_handle = tokio::spawn(slither_core::crawler::crawl(crawl_config, event_tx));

    let crawl_result = match crawl_handle.await {
        Ok(Ok(result)) => result,
        Ok(Err(e)) => {
            // Crawl returned an error.
            error!(job_id = %job_id, "Crawl failed: {e}");
            let _ = state.job_manager.set_error(&job_id, &e.to_string());
            let _ = state.job_manager.update_status(&job_id, JobStatus::Failed);
            fire_terminal_webhook(&state, &job_id).await;
            // Wait for the progress listener to drain.
            let _ = progress_listener.await;
            return;
        }
        Err(join_err) => {
            // The crawl task panicked or was cancelled.
            error!(job_id = %job_id, "Crawl task panicked: {join_err}");
            let _ = state
                .job_manager
                .set_error(&job_id, &format!("Crawl task panicked: {join_err}"));
            let _ = state.job_manager.update_status(&job_id, JobStatus::Failed);
            fire_terminal_webhook(&state, &job_id).await;
            let _ = progress_listener.await;
            return;
        }
    };

    // Wait for the progress listener to finish draining events.
    let _ = progress_listener.await;

    // PageSpeed enrichment — enrich pages and re-run the pipeline so scores
    // reflect CWV data when --pagespeed / pagespeed=true is set.
    let crawl_result = {
        let robots_txt = crawl_result.robots_txt.clone();
        // Carry sitemap discovery forward: re-running the pipeline with None
        // would drop sitemap coverage analysis from every server-side crawl.
        let sitemap_data = crawl_result.sitemap_data.clone();
        let mut pages = crawl_result.pages;
        slither_core::enrich_pagespeed(&mut pages, &config).await;

        match slither_core::run_post_crawl_pipeline(PipelineInput {
            pages,
            config: config.clone(),
            duration_ms: crawl_result.crawl_metadata.duration_ms,
            pages_discovered: crawl_result.crawl_metadata.pages_discovered,
            pages_crawled: crawl_result.crawl_metadata.pages_crawled,
            pages_skipped_robots: crawl_result.crawl_metadata.pages_skipped_robots,
            pages_errored: crawl_result.crawl_metadata.pages_errored,
            backend: crawl_result.crawl_metadata.backend.clone(),
            sitemap_data,
            robots_txt,
        }) {
            Ok(result) => result,
            Err(e) => {
                error!(job_id = %job_id, "Post-crawl pipeline failed: {e}");
                let _ = state.job_manager.set_error(&job_id, &e.to_string());
                let _ = state.job_manager.update_status(&job_id, JobStatus::Failed);
                fire_terminal_webhook(&state, &job_id).await;
                return;
            }
        }
    };

    // Determine the output directory.
    let output_dir = match state.job_manager.get_job(&job_id) {
        Ok(Some(job)) => match job.output_dir {
            Some(dir) => dir,
            None => {
                error!(job_id = %job_id, "Job has no output directory");
                let _ = state
                    .job_manager
                    .set_error(&job_id, "Job has no output directory");
                let _ = state.job_manager.update_status(&job_id, JobStatus::Failed);
                fire_terminal_webhook(&state, &job_id).await;
                return;
            }
        },
        Ok(None) => {
            error!(job_id = %job_id, "Job not found after crawl completed");
            return;
        }
        Err(e) => {
            error!(job_id = %job_id, "Failed to read job from DB: {e}");
            return;
        }
    };

    let output_path = std::path::PathBuf::from(&output_dir);

    // Write output files.
    let mut files: Vec<String> = Vec::new();

    // crawl.json
    match report::json::serialize_crawl_result(&crawl_result) {
        Ok(json_str) => {
            let path = output_path.join("crawl.json");
            if let Err(e) = std::fs::write(&path, &json_str) {
                warn!(job_id = %job_id, "Failed to write crawl.json: {e}");
            } else {
                files.push("crawl.json".to_string());
            }
        }
        Err(e) => warn!(job_id = %job_id, "Failed to serialize crawl result: {e}"),
    }

    // report.html
    match report::html::render_html_report(&crawl_result) {
        Ok(html_str) => {
            let path = output_path.join("report.html");
            if let Err(e) = std::fs::write(&path, &html_str) {
                warn!(job_id = %job_id, "Failed to write report.html: {e}");
            } else {
                files.push("report.html".to_string());
            }
        }
        Err(e) => warn!(job_id = %job_id, "Failed to render HTML report: {e}"),
    }

    // export.csv
    match report::csv::generate_csv(&crawl_result) {
        Ok(csv_str) => {
            let path = output_path.join("export.csv");
            if let Err(e) = std::fs::write(&path, &csv_str) {
                warn!(job_id = %job_id, "Failed to write export.csv: {e}");
            } else {
                files.push("export.csv".to_string());
            }
        }
        Err(e) => warn!(job_id = %job_id, "Failed to generate CSV: {e}"),
    }

    // Build result summary.
    let summary = &crawl_result.summary;
    let result_summary = json!({
        "health_score": summary.health_score,
        "grade": summary.grade,
        "pages_crawled": crawl_result.crawl_metadata.pages_crawled,
        "issues_found": summary.total_issues,
        "critical_issues": summary.critical_issues,
        "warning_issues": summary.warning_issues,
        "info_issues": summary.info_issues,
        "files": files,
    });

    if let Err(e) = state
        .job_manager
        .update_result_summary(&job_id, result_summary)
    {
        warn!(job_id = %job_id, "Failed to update result summary: {e}");
    }

    // Mark completed. The transition is guarded against terminal states, so a
    // `false` means something else already finished this job — a cancel that
    // landed mid-crawl, or another process reclaiming it as orphaned. Say so:
    // this used to be swallowed, leaving a job that read `cancelled` (or
    // `failed`) beside a fully populated result summary with no explanation.
    match state
        .job_manager
        .update_status(&job_id, JobStatus::Completed)
    {
        Ok(true) => info!(
            job_id = %job_id,
            pages = crawl_result.crawl_metadata.pages_crawled,
            grade = %crawl_result.summary.grade,
            "Crawl job completed"
        ),
        Ok(false) => warn!(
            job_id = %job_id,
            pages = crawl_result.crawl_metadata.pages_crawled,
            "Crawl finished but the job already held a terminal status; \
             its results were written and its real status is reported as-is"
        ),
        Err(e) => {
            error!(job_id = %job_id, "Failed to mark job as completed: {e}");
            return;
        }
    }

    fire_terminal_webhook(&state, &job_id).await;
}

// ---------------------------------------------------------------------------
// Webhook helper
// ---------------------------------------------------------------------------

/// The webhook event that describes a job in `status`, or `None` when the job
/// has not finished.
fn terminal_event(status: &JobStatus) -> Option<&'static str> {
    match status {
        JobStatus::Completed => Some("job.completed"),
        JobStatus::Cancelled => Some("job.cancelled"),
        JobStatus::Failed => Some("job.failed"),
        JobStatus::Queued | JobStatus::Running => None,
    }
}

/// Announce a job's *actual* terminal state.
///
/// The executor cannot assume which terminal event it is delivering: the store
/// refuses to overwrite a terminal status, so a crawl that ran to the end may
/// still be `cancelled`. Firing `job.completed` regardless produced
/// `{"event":"job.completed","job":{"status":"cancelled"}}` — self-contradictory,
/// and it burned the job's one-shot registration on the wrong event, which is
/// the one delivery a caller gets. Read the status back and name it.
///
/// Delivery is at-least-once for *persistent* subscriptions: a cancel announces
/// the terminal state immediately (a running crawl is not aborted, so waiting
/// for it would delay the notification by the length of the crawl) and this
/// runs again when that crawl actually stops. A job's one-shot `webhook_url` is
/// unaffected — the store marks it fired, so it is delivered exactly once.
pub(crate) async fn fire_terminal_webhook(state: &Arc<AppState>, job_id: &str) {
    let event = match state.job_manager.get_job(job_id) {
        Ok(Some(job)) => match terminal_event(&job.status) {
            Some(event) => event,
            None => {
                warn!(
                    job_id = %job_id,
                    status = job.status.as_str(),
                    "job is not in a terminal state at the end of execution; no event fired"
                );
                return;
            }
        },
        Ok(None) => {
            // Deleted mid-flight: there is nothing left to describe, and the
            // one-shot registration went with it.
            warn!(job_id = %job_id, "job disappeared before its terminal event could fire");
            return;
        }
        Err(e) => {
            warn!(job_id = %job_id, "cannot read job to determine its terminal event: {e}");
            return;
        }
    };
    fire_webhook(state, job_id, event).await;
}

pub(crate) async fn fire_webhook(state: &Arc<AppState>, job_id: &str, event: &str) {
    let wh_mgr = WebhookManager::new(state.job_manager.conn());

    let urls = match wh_mgr.get_urls_for_event(job_id, event) {
        Ok(urls) => urls,
        Err(e) => {
            warn!(job_id = %job_id, event = %event, "Failed to get webhook URLs: {e}");
            return;
        }
    };

    if urls.is_empty() {
        return;
    }

    // Build the job snapshot for the payload, stripping internal fields like output_dir.
    let job_value = match state.job_manager.get_job(job_id) {
        Ok(Some(job)) => {
            let mut val = serde_json::to_value(&job).unwrap_or(json!({"id": job_id}));
            if let Some(obj) = val.as_object_mut() {
                obj.remove("output_dir");
            }
            val
        }
        _ => json!({"id": job_id}),
    };

    let payload = WebhookPayload {
        event: event.to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        job: job_value,
    };

    slither_core::jobs::webhook::dispatch_webhooks(urls, payload).await;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The event must describe the status, not the code path that reached it.
    /// A crawl that ran to completion after being cancelled delivered
    /// `{"event":"job.completed","job":{"status":"cancelled"}}` — a
    /// self-contradiction that also burned the job's one-shot webhook, which is
    /// the single delivery a caller gets.
    #[test]
    fn the_event_names_the_status_it_describes() {
        assert_eq!(terminal_event(&JobStatus::Completed), Some("job.completed"));
        assert_eq!(terminal_event(&JobStatus::Cancelled), Some("job.cancelled"));
        assert_eq!(terminal_event(&JobStatus::Failed), Some("job.failed"));
    }

    /// Nothing is announced for a job that has not finished.
    #[test]
    fn unfinished_jobs_have_no_terminal_event() {
        assert_eq!(terminal_event(&JobStatus::Queued), None);
        assert_eq!(terminal_event(&JobStatus::Running), None);
    }

    /// Every event this module can emit must be one the webhook API accepts at
    /// registration, or a subscriber can never receive it.
    #[test]
    fn emitted_events_are_all_registerable() {
        let emitted = [
            "job.queued",
            "job.running",
            terminal_event(&JobStatus::Completed).unwrap(),
            terminal_event(&JobStatus::Cancelled).unwrap(),
            terminal_event(&JobStatus::Failed).unwrap(),
        ];
        for event in emitted {
            assert!(
                crate::api::webhooks::VALID_EVENTS.contains(&event),
                "{event} is emitted but cannot be registered for"
            );
        }
    }

    /// And the converse: an event a caller can register for must actually be
    /// emitted by something. `job.queued` and `job.cancelled` were advertised
    /// and accepted at registration while nothing ever fired them.
    #[test]
    fn every_registerable_event_is_emitted_somewhere() {
        let emitted = [
            "job.queued",
            "job.running",
            "job.completed",
            "job.cancelled",
            "job.failed",
        ];
        for event in crate::api::webhooks::VALID_EVENTS {
            assert!(
                emitted.contains(event),
                "{event} can be registered for but nothing emits it"
            );
        }
    }
}
