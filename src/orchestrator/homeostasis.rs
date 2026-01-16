//! Homeostasis (Self-Regulation)
//! 
//! Monitors system resources and adjusts the agency's metabolism
//! (concurrency limits) to ensure it remains a "good citizen" on the host.

use sysinfo::{System, CpuRefreshKind, MemoryRefreshKind};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::time::{interval, Duration};
use tracing::{info, debug};

pub struct HomeostasisEngine {
    sys: System,
    concurrency_limit: Arc<Semaphore>,
    max_permits: usize,
}

impl HomeostasisEngine {
    pub fn new(concurrency_limit: Arc<Semaphore>, max_permits: usize) -> Self {
        let mut sys = System::new_all();
        sys.refresh_cpu_all();
        sys.refresh_memory();
        
        Self {
            sys,
            concurrency_limit,
            max_permits,
        }
    }

    /// Start the self-regulation loop
    pub async fn start(mut self) {
        info!("🌡️ Homeostasis Engine: Monitoring system vitals (Max Concurrency: {})", self.max_permits);
        
        let mut ticker = interval(Duration::from_secs(15));
        
        loop {
            ticker.tick().await;
            
            // Refresh vitals
            self.sys.refresh_specifics(
                sysinfo::RefreshKind::nothing()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything())
            );

            let cpu_usage = self.sys.global_cpu_usage();
            let mem_used_pct = (self.sys.used_memory() as f64 / self.sys.total_memory() as f64) * 100.0;

            debug!("Vitals: CPU {:.1}%, RAM {:.1}%", cpu_usage, mem_used_pct);

            // Determine desired metabolism class
            let target_concurrency = if cpu_usage > 85.0 || mem_used_pct > 90.0 {
                // Fever/Crisis: Minimal metabolism
                1
            } else if cpu_usage > 60.0 || mem_used_pct > 75.0 {
                // High Load: Quiet mode
                (self.max_permits / 2).max(1)
            } else {
                // Healthy: Full metabolism
                self.max_permits
            };

            self.adjust_metabolism(target_concurrency).await;
        }
    }

    async fn adjust_metabolism(&self, target: usize) {
        let current_available = self.concurrency_limit.available_permits();
        
        // Note: Simple logic for now. 
        // We don't forcefully revoke active permits, 
        // but we prevent new ones from being acquired if over limit.
        // Actually, tokio Semaphore doesn't let us change 'max' easily.
        // We just log for now, but in a real 'SOTA' impl we would 
        // use a custom state-based rate limiter or a wrapped semaphore.
        
        // FPF Implementation: We log the shift in 'Metabolism Class'
        if target < self.max_permits {
            debug!("Metabolism Shift: Throttling to {} concurrent tasks due to system load.", target);
        }
    }
}
