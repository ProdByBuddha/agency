//! Sensory Expansion (Watchdog)
//! 
//! "Nerve Endings" for the agency. Provides proactive sensors that
//! monitor external resources and trigger tasks in the queue.

use std::sync::Arc;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, error, warn};
use notify::{Watcher, RecursiveMode, Event, RecommendedWatcher};
use crate::orchestrator::queue::TaskQueue;
use serde_json::json;
use anyhow::Result;
use reqwest::Client;
use rss::Channel;
use std::collections::HashMap;

/// Types of sensors supported
pub enum SensorType {
    File(PathBuf),
    Http(String, Duration),
    Rss(String, Duration),
}

/// The Sensory Cortex manages all active proactive sensors
pub struct SensoryCortex {
    queue: Arc<dyn TaskQueue>,
    http_client: Client,
    // Store previous states to detect changes
    http_history: Arc<Mutex<HashMap<String, String>>>, // URL -> Content Hash
    rss_history: Arc<Mutex<HashMap<String, String>>>,  // URL -> Last GUID/Title
}

impl SensoryCortex {
    pub fn new(queue: Arc<dyn TaskQueue>) -> Self {
        Self {
            queue,
            http_client: Client::new(),
            http_history: Arc::new(Mutex::new(HashMap::new())),
            rss_history: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Start a File Watcher (Local Awareness)
    pub async fn watch_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref().to_path_buf();
        let queue = self.queue.clone();
        
        info!("👀 Sensory Cortex: Watching file/dir: {:?}", path);

        // We use RecommendedWatcher which is sync, so we bridge to async with a channel
        let (tx, mut rx) = tokio::sync::mpsc::channel(100);

        let mut watcher = RecommendedWatcher::new(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                let _ = tx.blocking_send(event);
            }
        }, notify::Config::default())?;

        watcher.watch(&path, RecursiveMode::Recursive)?;

        // Handle events in background
        tokio::spawn(async move {
            // Keep watcher alive by moving it into the task
            let _watcher = watcher;
            while let Some(event) = rx.recv().await {
                if event.kind.is_modify() || event.kind.is_create() {
                    for path in event.paths {
                        let path_str = path.to_string_lossy().to_string();
                        // Ignore noisy hidden files like .DS_Store or swap files
                        if path_str.contains("/.") || path_str.contains("~") { continue; }
                        
                        info!("🔔 Sensory Trigger: File change detected at {}", path_str);
                        let goal = format!("I noticed a change in the file '{}'. Please analyze the change and determine if any action is needed.", path_str);
                        let _ = queue.enqueue("autonomous_goal", json!(goal)).await;
                    }
                }
            }
        });

        Ok(())
    }

    /// Start an HTTP Watcher (Web Awareness)
    pub async fn watch_http(&self, url: String, interval: Duration) -> Result<()> {
        let client = self.http_client.clone();
        let history = self.http_history.clone();
        let queue = self.queue.clone();
        let url_clone = url.clone();

        info!("🌐 Sensory Cortex: Monitoring URL: {} (every {:?})", url, interval);

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                match client.get(&url_clone).send().await {
                    Ok(resp) => {
                        if let Ok(body) = resp.text().await {
                            // Simple hash-based change detection (using length + first 1000 chars for now)
                            // In a real impl, use SHA256.
                            let current_state = format!("{}:{}", body.len(), &body[..body.len().min(1000)]);
                            
                            let mut hist = history.lock().await;
                            let prev_state = hist.get(&url_clone).cloned();
                            
                            if let Some(prev) = prev_state {
                                if prev != current_state {
                                    info!("🔔 Sensory Trigger: Content change at {}", url_clone);
                                    let goal = format!("The website {} has changed. Please visit the URL and summarize what is new.", url_clone);
                                    let _ = queue.enqueue("autonomous_goal", json!(goal)).await;
                                    hist.insert(url_clone.clone(), current_state);
                                }
                            } else {
                                // First run, just record
                                hist.insert(url_clone.clone(), current_state);
                            }
                        }
                    }
                    Err(e) => warn!("Watchdog failed to poll {}: {}", url_clone, e),
                }
            }
        });

        Ok(())
    }

    /// Start an RSS Watcher (News Awareness)
    pub async fn watch_rss(&self, url: String, interval: Duration) -> Result<()> {
        let client = self.http_client.clone();
        let history = self.rss_history.clone();
        let queue = self.queue.clone();
        let url_clone = url.clone();

        info!("📻 Sensory Cortex: Monitoring RSS: {} (every {:?})", url, interval);

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                match client.get(&url_clone).send().await {
                    Ok(resp) => {
                        if let Ok(bytes) = resp.bytes().await {
                            if let Ok(channel) = Channel::read_from(&bytes[..]) {
                                if let Some(latest_item) = channel.items().first() {
                                    let latest_id = latest_item.guid().map(|g| g.value())
                                        .or(latest_item.title())
                                        .unwrap_or("unknown")
                                        .to_string();
                                    
                                    let mut hist = history.lock().await;
                                    let last_id = hist.get(&url_clone).cloned();
                                    
                                    if let Some(prev) = last_id {
                                        if prev != latest_id {
                                            let title = latest_item.title().unwrap_or("New Update");
                                            info!("🔔 Sensory Trigger: New RSS item: {}", title);
                                            let goal = format!("A new update was posted to the RSS feed {}: '{}'. Please read the full item and assess its relevance.", url_clone, title);
                                            let _ = queue.enqueue("autonomous_goal", json!(goal)).await;
                                            hist.insert(url_clone.clone(), latest_id);
                                        }
                                    } else {
                                        hist.insert(url_clone.clone(), latest_id);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => warn!("Watchdog failed to poll RSS {}: {}", url_clone, e),
                }
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::queue::SqliteTaskQueue;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_sensory_cortex_init() {
        let tmp = NamedTempFile::new().unwrap();
        let queue = Arc::new(SqliteTaskQueue::new(tmp.path()).await.unwrap());
        let cortex = SensoryCortex::new(queue);
        
        // Initialization should not fail
        assert!(cortex.http_history.lock().await.is_empty());
    }

    #[tokio::test]
    async fn test_watch_file_registration() {
        let tmp_db = NamedTempFile::new().unwrap();
        let queue = Arc::new(SqliteTaskQueue::new(tmp_db.path()).await.unwrap());
        let cortex = SensoryCortex::new(queue);
        
        let tmp_watch = NamedTempFile::new().unwrap();
        let res = cortex.watch_file(tmp_watch.path()).await;
        
        assert!(res.is_ok(), "Watch file should register correctly");
    }
}
