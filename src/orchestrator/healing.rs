//! Self-Healing Engine (The Doctor)
//! 
//! Monitors the agency's nervous system (logs) for distress signals (errors)
//! and proactively schedules self-repair tasks.

use std::sync::Arc;
use std::path::{Path, PathBuf};
use tokio::time::{interval, Duration};
use tokio::fs;
use tracing::{info, error, warn, debug};
use crate::orchestrator::queue::TaskQueue;
use serde_json::json;
use anyhow::Result;

pub struct HealingEngine {
    queue: Arc<dyn TaskQueue>,
    log_dir: PathBuf,
}

impl HealingEngine {
    pub fn new(queue: Arc<dyn TaskQueue>) -> Self {
        Self {
            queue,
            log_dir: PathBuf::from("logs"),
        }
    }

    /// Start the diagnostic loop
    pub async fn start(self) {
        info!("👨‍⚕️ Healing Engine: Doctor is in. Monitoring logs for systemic errors...");
        
        let mut ticker = interval(Duration::from_secs(60)); // Check every minute
        
        loop {
            ticker.tick().await;
            if let Err(e) = self.diagnose().await {
                error!("Healing Engine: Diagnosis failure: {}", e);
            }
        }
    }

    async fn diagnose(&self) -> Result<()> {
        // 1. Find the latest log file
        let mut entries = fs::read_dir(&self.log_dir).await?;
        let mut log_files = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.is_file() && path.to_string_lossy().contains("agency.log") {
                log_files.push(path);
            }
        }
        
        log_files.sort();
        let latest_log = match log_files.last() {
            Some(p) => p,
            None => return Ok(()),
        };

        // 2. Read the tail of the log
        // We only look at the last 50 lines to detect recent "fever"
        let content = fs::read_to_string(latest_log).await?;
        let lines: Vec<&str> = content.lines().rev().take(50).collect();

        // 3. Look for error patterns
        let mut critical_errors = Vec::new();
        for line in lines {
            if line.contains("ERROR") || line.contains("panic") || line.contains("failed") {
                critical_errors.push(line.to_string());
            }
        }

        if !critical_errors.is_empty() {
            info!("👨‍⚕️ Healing Engine: Detected {} symptoms. Scheduling self-repair.", critical_errors.len());
            
            let symptoms = critical_errors.join("\n");
            let goal = format!(
                "SELF-HEALING MISSION: I have detected the following errors in my system logs. Please use the mutation_engine and codebase_explorer tools to diagnose the root cause and apply a permanent fix. \n\nSYMPTOMS:\n{}", 
                symptoms
            );

            // Enqueue a high-priority repair task
            let _ = self.queue.enqueue("autonomous_goal", json!(goal)).await;
        }

        Ok(())
    }
}
