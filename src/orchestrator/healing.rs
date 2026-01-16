//! Self-Healing Engine (The Doctor)
//! 
//! Monitors the agency's nervous system (logs) for distress signals (errors).
//! Implements a "Verification Loop" to track incidents, verify fixes, and escalate failures.

use std::sync::Arc;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use tokio::time::{interval, Duration};
use tokio::sync::Mutex;
use tokio::fs;
use tracing::{info, error, warn, debug};
use crate::orchestrator::queue::TaskQueue;
use serde_json::json;
use anyhow::Result;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
enum IncidentStatus {
    New,
    Fixing,
    Verifying,
    Escalated,
}

#[derive(Debug, Clone)]
struct Incident {
    signature: String,
    first_seen: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    fix_attempted_at: Option<DateTime<Utc>>,
    status: IncidentStatus,
    attempts: u32,
}

pub struct HealingEngine {
    queue: Arc<dyn TaskQueue>,
    log_dir: PathBuf,
    /// Active incidents tracked by error signature
    incidents: Arc<Mutex<HashMap<String, Incident>> >,
}

impl HealingEngine {
    pub fn new(queue: Arc<dyn TaskQueue>) -> Self {
        Self {
            queue,
            log_dir: PathBuf::from("logs"),
            incidents: Arc::new(Mutex::new(HashMap::new()))
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

        // 2. Read the tail of the log (last 100 lines)
        let content = fs::read_to_string(latest_log).await?;
        let lines: Vec<&str> = content.lines().rev().take(100).collect();

        // 3. Analyze for symptoms
        let mut current_errors = Vec::new();
        for line in lines {
            if line.contains("ERROR") || line.contains("panic") || line.contains("failed") {
                // Generate a simplified signature (e.g., remove timestamps/dynamic data if possible)
                // For this prototype, we'll use the raw line content as signature, strictly simplified
                let signature = line.split_whitespace().skip(2).collect::<Vec<&str>>().join(" ");
                current_errors.push((line.to_string(), signature));
            }
        }

        if current_errors.is_empty() {
            // Healthy pulse
            return Ok(())
        }

        // 4. Update Incident State
        let mut incidents = self.incidents.lock().await;
        let now = Utc::now();

        for (full_line, signature) in current_errors {
            let incident = incidents.entry(signature.clone()).or_insert(Incident {
                signature: signature.clone(),
                first_seen: now,
                last_seen: now,
                fix_attempted_at: None,
                status: IncidentStatus::New,
                attempts: 0,
            });

            incident.last_seen = now;

            match incident.status {
                IncidentStatus::New => {
                    info!("👨‍⚕️ Healing Engine: New symptom detected: \"{}\"", signature);
                    incident.status = IncidentStatus::Fixing;
                    incident.fix_attempted_at = Some(now);
                    incident.attempts += 1;

                    self.prescribe_fix(&full_line, false).await?;
                },
                IncidentStatus::Fixing => {
                    // Check if enough time passed for verification (e.g., 5 minutes)
                    if let Some(attempted) = incident.fix_attempted_at {
                        if now.signed_duration_since(attempted).num_minutes() > 5 {
                            // Symptom persists after 5 mins -> Verify Failed -> Escalate
                            warn!("👨‍⚕️ Healing Engine: Fix VERIFICATION FAILED for: \"{}\"", signature);
                            incident.status = IncidentStatus::Escalated;
                            
                            self.prescribe_fix(&full_line, true).await?;
                        } else {
                            debug!("Doctor: Monitoring active patient (Time remaining)...");
                        }
                    }
                },
                IncidentStatus::Verifying => {
                    // Similar logic to Fixing, but maybe distinct workflow
                },
                IncidentStatus::Escalated => {
                    // Already shouted about this. Don't spam unless it's been a LONG time (e.g. 1 hour)
                    if let Some(attempted) = incident.fix_attempted_at {
                        if now.signed_duration_since(attempted).num_minutes() > 60 {
                            warn!("👨‍⚕️ Healing Engine: Recurring CRITICAL symptom: \"{}\"", signature);
                            incident.fix_attempted_at = Some(now); // Reset timer
                            // Re-escalate
                            self.prescribe_fix(&full_line, true).await?;
                        }
                    }
                }
            }
        }

        // Cleanup old resolved incidents (not seen in 24 hours)
        incidents.retain(|_, i| now.signed_duration_since(i.last_seen).num_hours() < 24);

        Ok(())
    }

    async fn prescribe_fix(&self, symptom: &str, critical: bool) -> Result<()> {
        let prefix = if critical { "CRITICAL ESCALATION" } else { "SELF-HEALING MISSION" };
        let urgency = if critical { "IMMEDIATELY" } else { "autonomously" };
        
        let goal = format!(
            "{}: The following error persists in the system logs. The previous fix may have failed. Please DIAGNOSE the codebase {} and apply a robust correction. \n\nSYMPTOM:\n{}", 
            prefix, urgency, symptom
        );

        // Enqueue task
        info!("👨‍⚕️ Doctor: Dispatching {} task.", prefix);
        let _ = self.queue.enqueue("autonomous_goal", json!(goal)).await;
        Ok(())
    }
}
