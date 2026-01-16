//! Circadian Rhythm (Scheduler)
//! 
//! Manages the "Biological Clock" of the agency, scheduling recurring
//! maintenance tasks (habits) and future intentions.

use tokio_cron_scheduler::{Job, JobScheduler};
use std::sync::Arc;
use tracing::{info, error};
use crate::orchestrator::queue::TaskQueue;
use serde_json::json;

pub struct AgencyScheduler {
    scheduler: JobScheduler,
    queue: Arc<dyn TaskQueue>,
}

impl AgencyScheduler {
    pub async fn new(queue: Arc<dyn TaskQueue>) -> anyhow::Result<Self> {
        let scheduler = JobScheduler::new().await?;
        Ok(Self { scheduler, queue })
    }

    /// Start the biological clock
    pub async fn start(&self) -> anyhow::Result<()> {
        self.scheduler.start().await?;
        Ok(())
    }

    /// Define a new recurring habit
    pub async fn add_habit(&self, name: &str, schedule: &str, task_kind: &str, payload: serde_json::Value) -> anyhow::Result<()> {
        let queue = self.queue.clone();
        let kind = task_kind.to_string();
        let payload = payload.clone();
        let name = name.to_string();
        let name_clone = name.clone();

        let job = Job::new_async(schedule, move |_uuid, _l| {
            let q = queue.clone();
            let k = kind.clone();
            let p = payload.clone();
            let n = name_clone.clone();
            Box::pin(async move {
                info!("⏰ Circadian Rhythm: Triggering habit '{}'", n);
                // We enqueue the task into the persistent queue.
                // The Supervisor's background worker will actually execute it.
                if let Err(e) = q.enqueue(&k, p).await {
                    error!("Failed to enqueue habit '{}': {}", n, e);
                }
            })
        })?;

        self.scheduler.add(job).await?;
        info!("📅 Habit scheduled: '{}' ({})", name, schedule);
        Ok(())
    }

    /// Initialize default "Health" habits
    pub async fn init_defaults(&self) -> anyhow::Result<()> {
        // Hourly: System Health Check
        // "0 0 * * * *" = Every hour at minute 0
        self.add_habit(
            "Hourly Health Check", 
            "0 0 * * * *", 
            "autonomous_goal", 
            json!("Perform a self-health check of the agency system. Report on memory usage, queue depth, and uptime.")
        ).await?;

        // Daily: Memory Consolidation (Midnight)
        self.add_habit(
            "Daily Dreaming", 
            "0 0 0 * * *", 
            "memory_consolidation", 
            json!({})
        ).await?;

        // 5 Minutes: Visual Observation (Proactive Grounding)
        // Every 5 minutes at second 0
        self.add_habit(
            "Visual Observation", 
            "0 */5 * * * *", 
            "visual_observation", 
            json!({})
        ).await?;

        Ok(())
    }
}
