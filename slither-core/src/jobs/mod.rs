pub mod db;
pub mod manager;
pub mod webhook;

pub use manager::{Job, JobManager, JobStatus, JobType, ListJobsFilter};
pub use webhook::{Webhook, WebhookManager, WebhookPayload};

use std::path::PathBuf;

pub fn slither_home() -> PathBuf {
    std::env::var("SLITHER_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                home_dir().join("slither")
            } else {
                home_dir().join(".slither")
            }
        })
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn db_path() -> PathBuf {
    slither_home().join("slither.db")
}

pub fn jobs_dir() -> PathBuf {
    slither_home().join("jobs")
}
