use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobType {
    Crawl,
    Inspect,
    Extract,
    Screenshot,
    Entity,
}

impl JobType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Crawl => "crawl",
            Self::Inspect => "inspect",
            Self::Extract => "extract",
            Self::Screenshot => "screenshot",
            Self::Entity => "entity",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "crawl" => Some(Self::Crawl),
            "inspect" => Some(Self::Inspect),
            "extract" => Some(Self::Extract),
            "screenshot" => Some(Self::Screenshot),
            "entity" => Some(Self::Entity),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Job struct
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    #[serde(rename = "type")]
    pub job_type: JobType,
    pub status: JobStatus,
    pub domain: String,
    pub url: String,
    pub config: Value,
    pub progress: Option<Value>,
    pub result_summary: Option<Value>,
    pub output_dir: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

// ---------------------------------------------------------------------------
// ListJobsFilter
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ListJobsFilter {
    pub status: Option<String>,
    pub job_type: Option<String>,
    pub domain: Option<String>,
    pub limit: u32,
    pub offset: u32,
}

impl Default for ListJobsFilter {
    fn default() -> Self {
        Self {
            status: None,
            job_type: None,
            domain: None,
            limit: 20,
            offset: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// JobManager
// ---------------------------------------------------------------------------

/// How often a live process must refresh its ownership heartbeat.
///
/// Drive this from a thread that cannot be starved by application work: a
/// missed heartbeat is what makes another process consider these jobs orphaned.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

/// A heartbeat older than this means the owning process is no longer running.
///
/// Eight missed beats. The heartbeat is the *only* liveness signal used: a pid
/// is not trustworthy across pid reuse (a recycled pid reads as alive and would
/// strand the jobs forever) nor across containers sharing a SLITHER_HOME (a pid
/// from another namespace reads as dead and would reclaim live work). A process
/// that is running refreshes its own row; one that is not, cannot.
pub const OWNER_STALE_AFTER: Duration = Duration::from_secs(120);

/// Error recorded on a job whose owning process disappeared mid-flight.
pub const ORPHANED_JOB_ERROR: &str = "Owning slither process exited before the job finished";

pub struct JobManager {
    conn: Arc<Mutex<Connection>>,
    jobs_dir: PathBuf,
    /// Identifies *this* process instance. Jobs created here are stamped with
    /// it, and recovery never reclaims a job whose owner is still beating.
    owner_id: String,
}

impl JobManager {
    pub fn new(conn: Connection, jobs_dir: PathBuf) -> Self {
        let manager = Self {
            conn: Arc::new(Mutex::new(conn)),
            jobs_dir,
            owner_id: Uuid::new_v4().to_string(),
        };
        // Best-effort: a manager that cannot register still works, its jobs are
        // simply eligible for recovery by another process once unowned.
        if let Err(e) = manager.heartbeat() {
            tracing::warn!("failed to register job ownership record: {e}");
        }
        // Every process that opens the store adds a row here, including
        // short-lived CLI invocations that never run recovery, so clean up on
        // the way in as well as during recovery.
        if let Err(e) = manager.prune_stale_owners() {
            tracing::debug!("could not prune stale ownership records: {e}");
        }
        manager
    }

    /// Drop ownership records that stopped beating and own no jobs.
    ///
    /// A stale owner that still has jobs is kept, so those jobs can be
    /// attributed until recovery deals with them.
    pub fn prune_stale_owners(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM job_owners
              WHERE heartbeat_at < ?1
                AND id NOT IN (SELECT owner_id FROM jobs WHERE owner_id IS NOT NULL)",
            rusqlite::params![Self::stale_before().to_rfc3339()],
        )?;
        Ok(())
    }

    /// The heartbeat cutoff: an owner that has not beaten since this is gone.
    fn stale_before() -> chrono::DateTime<Utc> {
        Utc::now()
            - chrono::Duration::from_std(OWNER_STALE_AFTER)
                .unwrap_or_else(|_| chrono::TimeDelta::zero())
    }

    /// The ownership token stamped on every job this manager creates.
    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    /// Publish "this process is alive" for the ownership record.
    ///
    /// Upserts rather than updates so a row pruned by another process's
    /// recovery pass is recreated instead of silently never beating again.
    pub fn heartbeat(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO job_owners (id, pid, started_at, heartbeat_at) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(id) DO UPDATE SET heartbeat_at = ?3",
            rusqlite::params![self.owner_id, std::process::id() as i64, now],
        )?;
        Ok(())
    }

    /// Returns a clone of the `Arc<Mutex<Connection>>` so other managers
    /// (e.g. WebhookManager) can share the same database connection.
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    pub fn jobs_dir(&self) -> &PathBuf {
        &self.jobs_dir
    }

    // -- create -----------------------------------------------------------

    const INSERT_JOB_SQL: &'static str =
        "INSERT INTO jobs (id, type, status, domain, url, config, output_dir, created_at, owner_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)";

    /// Build the row values for a new queued job. Pure — the caller creates the
    /// output directory and performs the insert, so a job that is refused can
    /// leave nothing behind.
    fn new_queued_job(&self, job_type: JobType, url: &str, domain: &str, config: Value) -> Job {
        let id = Uuid::new_v4().to_string();
        let short_id = &id[..6];
        let date = Utc::now().format("%Y-%m-%d").to_string();
        // Sanitize domain for use in filesystem path: keep only safe chars, truncate.
        let safe_domain: String = domain
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
            .take(63)
            .collect();
        let output_dir = self
            .jobs_dir
            .join(format!("{date}_{short_id}_{safe_domain}"));

        Job {
            id,
            job_type,
            status: JobStatus::Queued,
            domain: domain.to_string(),
            url: url.to_string(),
            config,
            progress: None,
            result_summary: None,
            output_dir: Some(output_dir.to_string_lossy().to_string()),
            error: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            completed_at: None,
        }
    }

    fn create_output_dir(job: &Job) -> Result<()> {
        let dir = job.output_dir.as_deref().unwrap_or_default();
        std::fs::create_dir_all(dir).with_context(|| format!("Failed to create output dir: {dir}"))
    }

    pub fn create_job(
        &self,
        job_type: JobType,
        url: &str,
        domain: &str,
        config: Value,
    ) -> Result<Job> {
        let job = self.new_queued_job(job_type, url, domain, config);
        Self::create_output_dir(&job)?;
        let config_str = serde_json::to_string(&job.config)?;

        {
            let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            conn.execute(
                Self::INSERT_JOB_SQL,
                rusqlite::params![
                    job.id,
                    job.job_type.as_str(),
                    job.status.as_str(),
                    job.domain,
                    job.url,
                    config_str,
                    job.output_dir,
                    job.created_at,
                    self.owner_id,
                ],
            )?;
        }

        Ok(job)
    }

    /// Create a job only if fewer than `cap` jobs are already queued.
    ///
    /// Returns `None` when the cap is reached. The count and the insert run in
    /// one IMMEDIATE transaction: checking the depth with a separate query first
    /// let a burst of concurrent requests all observe the same under-cap count
    /// and every one of them insert, overshooting the ceiling.
    pub fn create_job_if_under_cap(
        &self,
        job_type: JobType,
        url: &str,
        domain: &str,
        config: Value,
        cap: u64,
    ) -> Result<Option<Job>> {
        let job = self.new_queued_job(job_type, url, domain, config);
        let config_str = serde_json::to_string(&job.config)?;

        {
            let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

            let queued: i64 = tx.query_row(
                "SELECT COUNT(*) FROM jobs WHERE status = 'queued'",
                [],
                |row| row.get(0),
            )?;
            if queued as u64 >= cap {
                return Ok(None);
            }

            // Only once the job is known to fit, so a refused one leaves no
            // orphaned directory behind.
            Self::create_output_dir(&job)?;

            tx.execute(
                Self::INSERT_JOB_SQL,
                rusqlite::params![
                    job.id,
                    job.job_type.as_str(),
                    job.status.as_str(),
                    job.domain,
                    job.url,
                    config_str,
                    job.output_dir,
                    job.created_at,
                    self.owner_id,
                ],
            )?;
            tx.commit()?;
        }

        Ok(Some(job))
    }

    // -- get --------------------------------------------------------------

    pub fn get_job(&self, id: &str) -> Result<Option<Job>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, type, status, domain, url, config, progress, result_summary,
                    output_dir, error, created_at, started_at, completed_at
             FROM jobs WHERE id = ?1",
        )?;

        let mut rows = stmt.query_map(rusqlite::params![id], row_to_job)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    // -- list -------------------------------------------------------------

    /// Count jobs with a given status. Unlike `list_jobs`, this is not capped by
    /// a `limit`, so it can be used for accurate queue-depth checks.
    pub fn count_jobs_by_status(&self, status: &str) -> Result<u64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE status = ?1",
            [status],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    pub fn list_jobs(&self, filter: &ListJobsFilter) -> Result<Vec<Job>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        let mut where_clauses: Vec<String> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut idx = 1u32;

        if let Some(ref status) = filter.status {
            where_clauses.push(format!("status = ?{idx}"));
            params.push(Box::new(status.clone()));
            idx += 1;
        }
        if let Some(ref job_type) = filter.job_type {
            where_clauses.push(format!("type = ?{idx}"));
            params.push(Box::new(job_type.clone()));
            idx += 1;
        }
        if let Some(ref domain) = filter.domain {
            where_clauses.push(format!("domain = ?{idx}"));
            params.push(Box::new(domain.clone()));
            idx += 1;
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let sql = format!(
            "SELECT id, type, status, domain, url, config, progress, result_summary,
                    output_dir, error, created_at, started_at, completed_at
             FROM jobs {where_sql}
             ORDER BY created_at DESC
             LIMIT ?{idx} OFFSET ?{}",
            idx + 1
        );

        params.push(Box::new(filter.limit));
        params.push(Box::new(filter.offset));

        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| &**p).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), row_to_job)?;

        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(row?);
        }
        Ok(jobs)
    }

    // -- update_status ----------------------------------------------------

    /// Transition a job to `status`, honoring terminal-state invariants.
    ///
    /// Returns `true` when the row was updated and `false` when a guard
    /// rejected the transition (the job had already reached a terminal state).
    /// Callers that are about to do expensive work — notably the executor
    /// marking a job Running — must check the result: a `false` here means the
    /// job was cancelled while it sat in the queue and must not proceed.
    pub fn update_status(&self, id: &str, status: JobStatus) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let now = Utc::now().to_rfc3339();
        let status_str = status.as_str();

        let rows = match status {
            JobStatus::Running => {
                // A cancelled or failed job must never be resurrected. Without
                // this guard, cancelling a *queued* job was silently undone:
                // when a semaphore slot freed, the executor flipped it back to
                // running and crawled the whole site anyway.
                conn.execute(
                    "UPDATE jobs SET status = ?1, started_at = ?2 \
                     WHERE id = ?3 AND status NOT IN ('cancelled', 'failed', 'completed')",
                    rusqlite::params![status_str, now, id],
                )?
            }
            JobStatus::Completed | JobStatus::Failed => {
                // Do not overwrite a job that was already cancelled or failed —
                // a cancel that races a finishing crawl must win.
                conn.execute(
                    "UPDATE jobs SET status = ?1, completed_at = ?2 \
                     WHERE id = ?3 AND status NOT IN ('cancelled', 'failed')",
                    rusqlite::params![status_str, now, id],
                )?
            }
            JobStatus::Cancelled => {
                // Cancelling only makes sense for work that has not finished;
                // a completed or failed job keeps its terminal outcome.
                conn.execute(
                    "UPDATE jobs SET status = ?1, completed_at = ?2 \
                     WHERE id = ?3 AND status NOT IN ('completed', 'failed', 'cancelled')",
                    rusqlite::params![status_str, now, id],
                )?
            }
            _ => conn.execute(
                "UPDATE jobs SET status = ?1 WHERE id = ?2",
                rusqlite::params![status_str, id],
            )?,
        };

        Ok(rows > 0)
    }

    // -- update_progress --------------------------------------------------

    pub fn update_progress(&self, id: &str, progress: Value) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let progress_str = serde_json::to_string(&progress)?;
        conn.execute(
            "UPDATE jobs SET progress = ?1 WHERE id = ?2",
            rusqlite::params![progress_str, id],
        )?;
        Ok(())
    }

    // -- update_result_summary -------------------------------------------

    pub fn update_result_summary(&self, id: &str, summary: Value) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let summary_str = serde_json::to_string(&summary)?;
        conn.execute(
            "UPDATE jobs SET result_summary = ?1 WHERE id = ?2",
            rusqlite::params![summary_str, id],
        )?;
        Ok(())
    }

    // -- set_error --------------------------------------------------------

    pub fn set_error(&self, id: &str, error: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE jobs SET error = ?1 WHERE id = ?2",
            rusqlite::params![error, id],
        )?;
        Ok(())
    }

    // -- delete -----------------------------------------------------------

    pub fn delete_job(&self, id: &str) -> Result<bool> {
        // Fetch output_dir before deleting the row.
        let output_dir: Option<String> = {
            let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            conn.query_row(
                "SELECT output_dir FROM jobs WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .ok()
        };

        let deleted = {
            let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
            conn.execute("DELETE FROM jobs WHERE id = ?1", rusqlite::params![id])?
        };

        if deleted > 0 {
            if let Some(dir) = output_dir {
                let path = PathBuf::from(&dir);
                if path.exists() {
                    std::fs::remove_dir_all(&path).ok();
                }
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // -- recover_orphaned_jobs -------------------------------------------

    /// Fail the unfinished jobs whose owning process is provably gone, and
    /// return their ids so the caller can notify webhooks for them.
    ///
    /// A job is reclaimed only when its owner cannot be running:
    ///
    /// * it has no owner row at all — either a row written before ownership
    ///   tracking existed, or one whose owner record was already pruned; or
    /// * its owner's heartbeat is older than [`OWNER_STALE_AFTER`].
    ///
    /// Jobs belonging to *this* manager are never touched, and neither is work
    /// owned by another process that is still beating. Previously this failed
    /// every `running`/`queued` row unconditionally, so merely starting a second
    /// slither against the same `SLITHER_HOME` — attaching an MCP client to the
    /// default home, say — declared the first process's in-flight crawls dead
    /// while they were demonstrably still running, and the terminal-state guard
    /// then blocked the real completion.
    ///
    /// Safe to call repeatedly, so a long-lived process can also run it on a
    /// timer and pick up work orphaned after its own startup.
    pub fn recover_orphaned_jobs(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let now = Utc::now();
        let stale_before = Self::stale_before();
        let stale_before_str = stale_before.to_rfc3339();

        let orphaned: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT j.id, j.owner_id, o.heartbeat_at
                   FROM jobs j LEFT JOIN job_owners o ON o.id = j.owner_id
                  WHERE j.status IN ('running', 'queued')",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;

            let mut ids = Vec::new();
            for row in rows {
                let (id, owner_id, heartbeat_at) = row?;
                // Our own work is live by definition.
                if owner_id.as_deref() == Some(self.owner_id.as_str()) {
                    continue;
                }
                let owner_gone = match heartbeat_at {
                    // No owner record: nothing is going to finish this job.
                    None => true,
                    Some(ts) => match chrono::DateTime::parse_from_rfc3339(&ts) {
                        Ok(beat) => beat.with_timezone(&Utc) < stale_before,
                        // An unparseable heartbeat cannot prove liveness.
                        Err(_) => true,
                    },
                };
                if owner_gone {
                    ids.push(id);
                }
            }
            ids
        };

        let now_str = now.to_rfc3339();
        for id in &orphaned {
            conn.execute(
                "UPDATE jobs SET status = 'failed', error = ?1, completed_at = ?2
                 WHERE id = ?3 AND status IN ('running', 'queued')",
                rusqlite::params![ORPHANED_JOB_ERROR, now_str, id],
            )?;
        }

        // Keep the ownership table from growing by one row per process start:
        // drop stale owners that no longer have any job pointing at them.
        conn.execute(
            "DELETE FROM job_owners
              WHERE heartbeat_at < ?1
                AND id NOT IN (SELECT owner_id FROM jobs WHERE owner_id IS NOT NULL)",
            rusqlite::params![stale_before_str],
        )?;

        Ok(orphaned)
    }
}

// ---------------------------------------------------------------------------
// Helper: map a SQLite row to a Job
// ---------------------------------------------------------------------------

fn row_to_job(row: &rusqlite::Row) -> rusqlite::Result<Job> {
    let type_str: String = row.get(1)?;
    let status_str: String = row.get(2)?;
    let config_str: String = row.get(5)?;
    let progress_str: Option<String> = row.get(6)?;
    let result_summary_str: Option<String> = row.get(7)?;

    Ok(Job {
        id: row.get(0)?,
        job_type: JobType::from_str(&type_str).unwrap_or(JobType::Crawl),
        status: JobStatus::from_str(&status_str).unwrap_or(JobStatus::Queued),
        domain: row.get(3)?,
        url: row.get(4)?,
        config: serde_json::from_str(&config_str).unwrap_or(Value::Null),
        progress: progress_str.and_then(|s| serde_json::from_str(&s).ok()),
        result_summary: result_summary_str.and_then(|s| serde_json::from_str(&s).ok()),
        output_dir: row.get(8)?,
        error: row.get(9)?,
        created_at: row.get(10)?,
        started_at: row.get(11)?,
        completed_at: row.get(12)?,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::db::init_db;
    use tempfile::TempDir;

    fn setup() -> (JobManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        let manager = JobManager::new(conn, tmp.path().to_path_buf());
        (manager, tmp)
    }

    #[test]
    fn test_create_and_get_job() {
        let (mgr, _tmp) = setup();

        let job = mgr
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                serde_json::json!({"depth": 3}),
            )
            .unwrap();

        assert_eq!(job.job_type, JobType::Crawl);
        assert_eq!(job.status, JobStatus::Queued);
        assert_eq!(job.domain, "example.com");
        assert_eq!(job.url, "https://example.com");
        assert!(job.output_dir.is_some());

        let fetched = mgr.get_job(&job.id).unwrap().expect("job should exist");
        assert_eq!(fetched.id, job.id);
        assert_eq!(fetched.job_type, JobType::Crawl);
        assert_eq!(fetched.domain, "example.com");
    }

    #[test]
    fn test_list_jobs_with_filter() {
        let (mgr, _tmp) = setup();

        mgr.create_job(
            JobType::Crawl,
            "https://a.com",
            "a.com",
            serde_json::json!({}),
        )
        .unwrap();

        mgr.create_job(
            JobType::Inspect,
            "https://b.com",
            "b.com",
            serde_json::json!({}),
        )
        .unwrap();

        // No filter — both
        let all = mgr.list_jobs(&ListJobsFilter::default()).unwrap();
        assert_eq!(all.len(), 2);

        // Filter by type
        let crawls = mgr
            .list_jobs(&ListJobsFilter {
                job_type: Some("crawl".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(crawls.len(), 1);
        assert_eq!(crawls[0].job_type, JobType::Crawl);

        let inspects = mgr
            .list_jobs(&ListJobsFilter {
                job_type: Some("inspect".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(inspects.len(), 1);
        assert_eq!(inspects[0].job_type, JobType::Inspect);
    }

    #[test]
    fn test_update_status() {
        let (mgr, _tmp) = setup();

        let job = mgr
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();

        // Mark running
        mgr.update_status(&job.id, JobStatus::Running).unwrap();
        let updated = mgr.get_job(&job.id).unwrap().unwrap();
        assert_eq!(updated.status, JobStatus::Running);
        assert!(updated.started_at.is_some());

        // Mark completed
        mgr.update_status(&job.id, JobStatus::Completed).unwrap();
        let updated = mgr.get_job(&job.id).unwrap().unwrap();
        assert_eq!(updated.status, JobStatus::Completed);
        assert!(updated.completed_at.is_some());
    }

    /// Regression: cancelling a *queued* job used to be silently reversed. The
    /// Running transition was unguarded, so when a semaphore slot freed the
    /// executor flipped cancelled -> running and crawled the whole site.
    #[test]
    fn cancelled_queued_job_cannot_be_marked_running() {
        let (mgr, _tmp) = setup();

        let job = mgr
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();

        assert!(mgr.update_status(&job.id, JobStatus::Cancelled).unwrap());

        // The executor's attempt to start the job must be refused...
        assert!(
            !mgr.update_status(&job.id, JobStatus::Running).unwrap(),
            "a cancelled job must not be resurrected into running"
        );

        // ...and the job must still read as cancelled.
        let updated = mgr.get_job(&job.id).unwrap().unwrap();
        assert_eq!(updated.status, JobStatus::Cancelled);
    }

    /// A finished job keeps its terminal outcome — cancelling it is a no-op.
    #[test]
    fn completed_job_cannot_be_cancelled() {
        let (mgr, _tmp) = setup();

        let job = mgr
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();

        assert!(mgr.update_status(&job.id, JobStatus::Running).unwrap());
        assert!(mgr.update_status(&job.id, JobStatus::Completed).unwrap());
        assert!(!mgr.update_status(&job.id, JobStatus::Cancelled).unwrap());

        let updated = mgr.get_job(&job.id).unwrap().unwrap();
        assert_eq!(updated.status, JobStatus::Completed);
    }

    /// A cancel that lands while a crawl is finishing still wins.
    #[test]
    fn cancel_beats_a_finishing_crawl() {
        let (mgr, _tmp) = setup();

        let job = mgr
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();

        assert!(mgr.update_status(&job.id, JobStatus::Running).unwrap());
        assert!(mgr.update_status(&job.id, JobStatus::Cancelled).unwrap());
        assert!(!mgr.update_status(&job.id, JobStatus::Completed).unwrap());

        let updated = mgr.get_job(&job.id).unwrap().unwrap();
        assert_eq!(updated.status, JobStatus::Cancelled);
    }

    /// The cap is checked inside the insert transaction, so it is exact rather
    /// than "roughly bounded" as the previous count-then-insert check was.
    #[test]
    fn create_job_if_under_cap_stops_exactly_at_the_cap() {
        let (mgr, _tmp) = setup();

        for i in 0..3 {
            let created = mgr
                .create_job_if_under_cap(
                    JobType::Inspect,
                    &format!("https://example.com/{i}"),
                    "example.com",
                    serde_json::json!({}),
                    3,
                )
                .unwrap();
            assert!(created.is_some(), "job {i} should fit under the cap");
        }

        let rejected = mgr
            .create_job_if_under_cap(
                JobType::Inspect,
                "https://example.com/overflow",
                "example.com",
                serde_json::json!({}),
                3,
            )
            .unwrap();
        assert!(rejected.is_none(), "the 4th job must be refused at cap 3");

        assert_eq!(mgr.count_jobs_by_status("queued").unwrap(), 3);
    }

    /// Only *queued* work counts toward the ceiling — jobs that have started or
    /// finished must not keep the queue permanently full.
    #[test]
    fn create_job_if_under_cap_only_counts_queued_jobs() {
        let (mgr, _tmp) = setup();

        let first = mgr
            .create_job_if_under_cap(
                JobType::Inspect,
                "https://example.com/1",
                "example.com",
                serde_json::json!({}),
                1,
            )
            .unwrap()
            .expect("first job fits");

        assert!(mgr
            .create_job_if_under_cap(
                JobType::Inspect,
                "https://example.com/2",
                "example.com",
                serde_json::json!({}),
                1,
            )
            .unwrap()
            .is_none());

        mgr.update_status(&first.id, JobStatus::Running).unwrap();

        assert!(
            mgr.create_job_if_under_cap(
                JobType::Inspect,
                "https://example.com/2",
                "example.com",
                serde_json::json!({}),
                1,
            )
            .unwrap()
            .is_some(),
            "a slot frees once the queued job starts running"
        );
    }

    /// A refused job must not leave an output directory behind.
    #[test]
    fn a_refused_job_creates_no_output_directory() {
        let (mgr, tmp) = setup();

        mgr.create_job_if_under_cap(
            JobType::Inspect,
            "https://example.com/1",
            "example.com",
            serde_json::json!({}),
            1,
        )
        .unwrap()
        .expect("first job fits");

        let before = std::fs::read_dir(tmp.path()).unwrap().count();

        assert!(mgr
            .create_job_if_under_cap(
                JobType::Inspect,
                "https://example.com/2",
                "example.com",
                serde_json::json!({}),
                1,
            )
            .unwrap()
            .is_none());

        let after = std::fs::read_dir(tmp.path()).unwrap().count();
        assert_eq!(before, after, "refused job left a directory behind");
    }

    #[test]
    fn test_delete_job() {
        let (mgr, _tmp) = setup();

        let job = mgr
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();

        let id = job.id.clone();

        assert!(mgr.delete_job(&id).unwrap());
        assert!(mgr.get_job(&id).unwrap().is_none());

        // Deleting again returns false
        assert!(!mgr.delete_job(&id).unwrap());
    }

    // -- ownership / recovery ---------------------------------------------

    /// Two managers over one on-disk database — the way two slither processes
    /// sharing a SLITHER_HOME actually see each other.
    fn two_managers() -> (JobManager, JobManager, TempDir) {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("slither.db");
        let jobs_dir = tmp.path().join("jobs");
        std::fs::create_dir_all(&jobs_dir).unwrap();
        let first = JobManager::new(crate::jobs::db::open_db(&db).unwrap(), jobs_dir.clone());
        let second = JobManager::new(crate::jobs::db::open_db(&db).unwrap(), jobs_dir);
        (first, second, tmp)
    }

    /// Backdate an owner's heartbeat, standing in for a process that died.
    fn age_owner(mgr: &JobManager, owner_id: &str, age: Duration) {
        let when = (Utc::now() - chrono::Duration::from_std(age).unwrap()).to_rfc3339();
        let conn = mgr.conn();
        let conn = conn.lock().unwrap();
        conn.execute(
            "UPDATE job_owners SET heartbeat_at = ?1 WHERE id = ?2",
            rusqlite::params![when, owner_id],
        )
        .unwrap();
    }

    /// A job whose owner stopped beating is reclaimed and reported.
    #[test]
    fn recovery_reclaims_jobs_whose_owner_is_gone() {
        let (first, second, _tmp) = two_managers();

        let job = first
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();
        first.update_status(&job.id, JobStatus::Running).unwrap();

        // The owning process dies: its heartbeat stops advancing.
        age_owner(
            &second,
            first.owner_id(),
            OWNER_STALE_AFTER + Duration::from_secs(30),
        );

        let recovered = second.recover_orphaned_jobs().unwrap();
        assert_eq!(recovered, vec![job.id.clone()]);

        let updated = second.get_job(&job.id).unwrap().unwrap();
        assert_eq!(updated.status, JobStatus::Failed);
        assert_eq!(updated.error.as_deref(), Some(ORPHANED_JOB_ERROR));
        assert!(updated.completed_at.is_some());
    }

    /// The regression this ownership machinery exists for: starting a second
    /// slither on the same SLITHER_HOME used to fail every in-flight job of the
    /// first one. A live owner's work must be left completely alone.
    #[test]
    fn a_second_process_does_not_kill_a_live_process_jobs() {
        let (first, second, _tmp) = two_managers();

        let running = first
            .create_job(
                JobType::Crawl,
                "https://example.com/running",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();
        first
            .update_status(&running.id, JobStatus::Running)
            .unwrap();
        let queued = first
            .create_job(
                JobType::Crawl,
                "https://example.com/queued",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();

        // The second process starts up and runs recovery, exactly as attaching
        // an MCP client to the default home does.
        let recovered = second.recover_orphaned_jobs().unwrap();
        assert!(
            recovered.is_empty(),
            "a live owner's jobs must not be reclaimed, got {recovered:?}"
        );

        assert_eq!(
            first.get_job(&running.id).unwrap().unwrap().status,
            JobStatus::Running
        );
        assert_eq!(
            first.get_job(&queued.id).unwrap().unwrap().status,
            JobStatus::Queued
        );

        // And the first process can still record the real completion.
        assert!(first
            .update_status(&running.id, JobStatus::Completed)
            .unwrap());
    }

    /// A manager never reclaims the work it owns itself, however old the job is.
    #[test]
    fn recovery_never_reclaims_its_own_jobs() {
        let (mgr, _tmp) = setup();

        let job = mgr
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();
        mgr.update_status(&job.id, JobStatus::Running).unwrap();

        // Even with a hopelessly stale heartbeat, these jobs are this process's
        // own responsibility — it is running, it is just not beating.
        age_owner(
            &mgr,
            mgr.owner_id(),
            OWNER_STALE_AFTER + Duration::from_secs(600),
        );

        assert!(mgr.recover_orphaned_jobs().unwrap().is_empty());
        assert_eq!(
            mgr.get_job(&job.id).unwrap().unwrap().status,
            JobStatus::Running
        );
    }

    /// Rows written before ownership tracking existed carry a NULL owner and
    /// nothing will ever finish them, so they are still recoverable.
    #[test]
    fn recovery_reclaims_unowned_legacy_jobs() {
        let (mgr, _tmp) = setup();

        let job = mgr
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();
        mgr.update_status(&job.id, JobStatus::Running).unwrap();
        {
            let conn = mgr.conn();
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE jobs SET owner_id = NULL WHERE id = ?1",
                rusqlite::params![job.id],
            )
            .unwrap();
        }

        assert_eq!(mgr.recover_orphaned_jobs().unwrap(), vec![job.id.clone()]);
        assert_eq!(
            mgr.get_job(&job.id).unwrap().unwrap().status,
            JobStatus::Failed
        );
    }

    /// Recovery must not resurrect or re-stamp jobs that already finished.
    #[test]
    fn recovery_leaves_terminal_jobs_alone() {
        let (first, second, _tmp) = two_managers();

        let done = first
            .create_job(
                JobType::Crawl,
                "https://example.com/done",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();
        first.update_status(&done.id, JobStatus::Running).unwrap();
        first.update_status(&done.id, JobStatus::Completed).unwrap();
        age_owner(
            &second,
            first.owner_id(),
            OWNER_STALE_AFTER + Duration::from_secs(30),
        );

        assert!(second.recover_orphaned_jobs().unwrap().is_empty());
        assert_eq!(
            second.get_job(&done.id).unwrap().unwrap().status,
            JobStatus::Completed
        );
    }

    /// Every process start adds an owner row; stale ones that own nothing must
    /// be pruned, or the table grows without bound.
    #[test]
    fn recovery_prunes_stale_owner_rows_that_own_nothing() {
        let (first, second, _tmp) = two_managers();

        let owners = |mgr: &JobManager| -> i64 {
            let conn = mgr.conn();
            let conn = conn.lock().unwrap();
            conn.query_row("SELECT COUNT(*) FROM job_owners", [], |r| r.get(0))
                .unwrap()
        };

        assert_eq!(owners(&second), 2);
        age_owner(
            &second,
            first.owner_id(),
            OWNER_STALE_AFTER + Duration::from_secs(30),
        );
        second.recover_orphaned_jobs().unwrap();
        assert_eq!(owners(&second), 1, "the dead owner's row should be pruned");
    }

    /// A stale owner that still has jobs keeps its row, so those jobs can be
    /// attributed rather than silently becoming unowned.
    #[test]
    fn recovery_keeps_owner_rows_that_still_have_jobs() {
        let (first, second, _tmp) = two_managers();

        first
            .create_job(
                JobType::Crawl,
                "https://example.com",
                "example.com",
                serde_json::json!({}),
            )
            .unwrap();
        age_owner(
            &second,
            first.owner_id(),
            OWNER_STALE_AFTER + Duration::from_secs(30),
        );
        second.recover_orphaned_jobs().unwrap();

        let conn = second.conn();
        let conn = conn.lock().unwrap();
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM job_owners WHERE id = ?1",
                rusqlite::params![first.owner_id()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1);
    }

    /// A heartbeat re-registers a pruned owner row rather than beating into the
    /// void, so a long-lived process cannot become permanently "dead".
    #[test]
    fn heartbeat_recreates_a_pruned_owner_row() {
        let (mgr, _tmp) = setup();

        {
            let conn = mgr.conn();
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM job_owners", []).unwrap();
        }

        mgr.heartbeat().unwrap();

        let conn = mgr.conn();
        let conn = conn.lock().unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM job_owners WHERE id = ?1",
                rusqlite::params![mgr.owner_id()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
    }
}
