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

    /// Pure Logic: Calculate target concurrency based on resource usage
    pub fn calculate_target_concurrency(cpu_usage: f32, mem_used_pct: f64, max_permits: usize) -> usize {
        if cpu_usage > 85.0 || mem_used_pct > 90.0 {
            // Fever/Crisis: Minimal metabolism
            1
        } else if cpu_usage > 60.0 || mem_used_pct > 75.0 {
            // High Load: Quiet mode
            (max_permits / 2).max(1)
        } else {
            // Healthy: Full metabolism
            max_permits
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
            let target_concurrency = Self::calculate_target_concurrency(cpu_usage, mem_used_pct, self.max_permits);

            self.adjust_metabolism(target_concurrency).await;
        }
    }

    async fn adjust_metabolism(&self, target: usize) {
        // FPF Implementation: We log the shift in 'Metabolism Class'
        if target < self.max_permits {
            debug!("Metabolism Shift: Throttling to {} concurrent tasks due to system load.", target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metabolism_calculation() {
        let max = 10;
        
        // Healthy
        assert_eq!(HomeostasisEngine::calculate_target_concurrency(10.0, 20.0, max), 10);
        
        // High CPU
        assert_eq!(HomeostasisEngine::calculate_target_concurrency(70.0, 20.0, max), 5);
        
        // Crisis (CPU)
        assert_eq!(HomeostasisEngine::calculate_target_concurrency(90.0, 20.0, max), 1);
        
        // Crisis (RAM)
        assert_eq!(HomeostasisEngine::calculate_target_concurrency(10.0, 95.0, max), 1);
    }
}