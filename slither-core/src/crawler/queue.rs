use std::collections::VecDeque;
use tokio::sync::Mutex;

/// An async work queue that tracks URLs to crawl with their depth.
pub struct CrawlQueue {
    inner: Mutex<VecDeque<QueueEntry>>,
}

#[derive(Debug, Clone)]
pub struct QueueEntry {
    pub url: String,
    pub depth: u32,
}

impl Default for CrawlQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl CrawlQueue {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
        }
    }

    pub async fn push(&self, url: String, depth: u32) {
        let mut queue = self.inner.lock().await;
        queue.push_back(QueueEntry { url, depth });
    }

    pub async fn pop(&self) -> Option<QueueEntry> {
        let mut queue = self.inner.lock().await;
        queue.pop_front()
    }

    pub async fn len(&self) -> usize {
        let queue = self.inner.lock().await;
        queue.len()
    }

    pub async fn is_empty(&self) -> bool {
        let queue = self.inner.lock().await;
        queue.is_empty()
    }
}
