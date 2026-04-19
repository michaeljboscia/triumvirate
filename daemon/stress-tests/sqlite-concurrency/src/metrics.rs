use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use hdrhistogram::Histogram;
use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, System};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct MetricsCollector {
    latency: Arc<Mutex<Histogram<u64>>>,
    busy_retries: Arc<AtomicU64>,
    successful_ops: Arc<AtomicU64>,
    failed_ops: Arc<AtomicU64>,
    wal_peak_bytes: Arc<AtomicU64>,
    proc_cpu_milli_sum: Arc<AtomicU64>,
    proc_cpu_samples: Arc<AtomicU64>,
    load_avg_1m_milli: Arc<AtomicU64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsSummary {
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub p99_9_ms: f64,
    pub busy_retries: u64,
    pub busy_rate: f64,
    pub successful_ops: u64,
    pub failed_ops: u64,
    pub wal_peak_mb: f64,
    pub process_cpu_pct: f64,
    pub system_load_avg_1m: f64,
}

impl MetricsCollector {
    pub fn new() -> Result<Self> {
        let histogram = Histogram::<u64>::new_with_bounds(1, 60_000_000, 3)?;
        Ok(Self {
            latency: Arc::new(Mutex::new(histogram)),
            busy_retries: Arc::new(AtomicU64::new(0)),
            successful_ops: Arc::new(AtomicU64::new(0)),
            failed_ops: Arc::new(AtomicU64::new(0)),
            wal_peak_bytes: Arc::new(AtomicU64::new(0)),
            proc_cpu_milli_sum: Arc::new(AtomicU64::new(0)),
            proc_cpu_samples: Arc::new(AtomicU64::new(0)),
            load_avg_1m_milli: Arc::new(AtomicU64::new(0)),
        })
    }

    pub async fn record_latency(&self, elapsed: Duration) {
        let micros = elapsed.as_micros().clamp(1, 60_000_000) as u64;
        let mut histogram = self.latency.lock().await;
        let _ = histogram.record(micros);
    }

    pub fn inc_busy_retry(&self) {
        self.busy_retries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_success(&self) {
        self.successful_ops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_failure(&self) {
        self.failed_ops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn start_samplers(&self, db_path: PathBuf, end_at: Instant) {
        let wal_sampler = self.clone();
        tokio::spawn(async move {
            let wal_path = wal_path_from_db(&db_path);
            while Instant::now() < end_at {
                wal_sampler.sample_wal_size(&wal_path);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            wal_sampler.sample_wal_size(&wal_path);
        });

        let proc_sampler = self.clone();
        tokio::spawn(async move {
            let pid = Pid::from_u32(std::process::id());
            let mut system = System::new();
            system.refresh_process_specifics(pid, ProcessRefreshKind::new().with_cpu());
            tokio::time::sleep(Duration::from_millis(500)).await;
            while Instant::now() < end_at {
                system.refresh_process_specifics(pid, ProcessRefreshKind::new().with_cpu());
                if let Some(proc_info) = system.process(pid) {
                    proc_sampler.record_proc_cpu(f64::from(proc_info.cpu_usage()));
                }
                let la = System::load_average().one;
                proc_sampler
                    .load_avg_1m_milli
                    .store((la * 1000.0) as u64, Ordering::Relaxed);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    pub async fn summary(&self) -> MetricsSummary {
        let histogram = self.latency.lock().await;
        let p50_ms = histogram.value_at_quantile(0.50) as f64 / 1_000.0;
        let p95_ms = histogram.value_at_quantile(0.95) as f64 / 1_000.0;
        let p99_ms = histogram.value_at_quantile(0.99) as f64 / 1_000.0;
        let p99_9_ms = histogram.value_at_quantile(0.999) as f64 / 1_000.0;
        drop(histogram);

        let busy_retries = self.busy_retries.load(Ordering::Relaxed);
        let successful_ops = self.successful_ops.load(Ordering::Relaxed);
        let failed_ops = self.failed_ops.load(Ordering::Relaxed);
        let total_ops = successful_ops + failed_ops;
        let busy_rate = if total_ops == 0 {
            0.0
        } else {
            busy_retries as f64 / total_ops as f64
        };

        let wal_peak_mb = self.wal_peak_bytes.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0);

        let proc_cpu_samples = self.proc_cpu_samples.load(Ordering::Relaxed);
        let process_cpu_pct = if proc_cpu_samples == 0 {
            0.0
        } else {
            (self.proc_cpu_milli_sum.load(Ordering::Relaxed) as f64 / 1000.0)
                / proc_cpu_samples as f64
        };
        let system_load_avg_1m = self.load_avg_1m_milli.load(Ordering::Relaxed) as f64 / 1000.0;

        MetricsSummary {
            p50_ms,
            p95_ms,
            p99_ms,
            p99_9_ms,
            busy_retries,
            busy_rate,
            successful_ops,
            failed_ops,
            wal_peak_mb,
            process_cpu_pct,
            system_load_avg_1m,
        }
    }

    fn sample_wal_size(&self, wal_path: &Path) {
        let size = fs::metadata(wal_path).map(|m| m.len()).unwrap_or(0);
        let mut current = self.wal_peak_bytes.load(Ordering::Relaxed);
        while size > current {
            match self
                .wal_peak_bytes
                .compare_exchange(current, size, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }
    }

    fn record_proc_cpu(&self, value: f64) {
        let milli = (value * 1000.0) as u64;
        self.proc_cpu_milli_sum.fetch_add(milli, Ordering::Relaxed);
        self.proc_cpu_samples.fetch_add(1, Ordering::Relaxed);
    }
}

fn wal_path_from_db(db_path: &Path) -> PathBuf {
    let wal = format!("{}-wal", db_path.display());
    PathBuf::from(wal)
}
