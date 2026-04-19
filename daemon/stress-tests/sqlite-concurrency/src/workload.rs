use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::ValueEnum;
use serde::Serialize;
use tokio::sync::Barrier;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ValueEnum)]
pub enum WorkloadProfile {
    Sustained,
    Wave,
    Herd,
}

pub struct WorkerPacer {
    profile: WorkloadProfile,
    worker_id: usize,
    operation_count: u64,
    start: Instant,
    barrier: Option<Arc<Barrier>>,
    next_herd_sync: Instant,
}

impl WorkerPacer {
    pub fn new(
        profile: WorkloadProfile,
        worker_id: usize,
        start: Instant,
        barrier: Option<Arc<Barrier>>,
    ) -> Self {
        Self {
            profile,
            worker_id,
            operation_count: 0,
            start,
            barrier,
            next_herd_sync: start + Duration::from_secs(30),
        }
    }

    pub async fn tick(&mut self) {
        match self.profile {
            WorkloadProfile::Sustained => {
                let sleep_secs = 3 + (self.jitter() % 8);
                tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
            }
            WorkloadProfile::Wave => {
                if self.operation_count == 0 {
                    let skew_ms = (self.worker_id as u64) * 10;
                    tokio::time::sleep(Duration::from_millis(skew_ms)).await;
                } else {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            WorkloadProfile::Herd => {
                if Instant::now() >= self.next_herd_sync {
                    if let Some(barrier) = &self.barrier {
                        barrier.wait().await;
                    }
                    self.next_herd_sync += Duration::from_secs(30);
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }

        self.operation_count = self.operation_count.saturating_add(1);
    }

    fn jitter(&self) -> u64 {
        // Deterministic pseudo-random jitter without external RNG crates.
        let elapsed = self.start.elapsed().as_nanos() as u64;
        let mut x = elapsed ^ ((self.worker_id as u64) << 32) ^ self.operation_count;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}
