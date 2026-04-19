use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Barrier;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ValueEnum)]
pub enum WorkloadProfileName {
    Sustained,
    Wave,
    Herd,
    LongTx,
    MixedRw,
    TraceReplay,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "name", rename_all = "kebab-case")]
pub enum WorkloadProfile {
    Sustained,
    Wave,
    Herd,
    LongTx { hold_ms: u64 },
    MixedRw { read_pct: u8 },
    TraceReplay {
        trace_path: PathBuf,
        event_type_filter: Option<String>,
    },
}

impl WorkloadProfile {
    pub fn from_cli(
        profile: WorkloadProfileName,
        tx_hold_ms: u64,
        read_pct: u8,
        trace_file: Option<PathBuf>,
        event_type_filter: Option<String>,
    ) -> Result<Self> {
        match profile {
            WorkloadProfileName::Sustained => Ok(Self::Sustained),
            WorkloadProfileName::Wave => Ok(Self::Wave),
            WorkloadProfileName::Herd => Ok(Self::Herd),
            WorkloadProfileName::LongTx => Ok(Self::LongTx {
                hold_ms: tx_hold_ms,
            }),
            WorkloadProfileName::MixedRw => Ok(Self::MixedRw { read_pct }),
            WorkloadProfileName::TraceReplay => Ok(Self::TraceReplay {
                trace_path: trace_file.ok_or_else(|| {
                    anyhow!("--trace-file is required when --profile trace-replay")
                })?,
                event_type_filter,
            }),
        }
    }

    pub fn tx_hold_ms(&self) -> u64 {
        match self {
            Self::LongTx { hold_ms } => *hold_ms,
            _ => 0,
        }
    }

    pub fn is_reader_worker(&self, worker_id: usize, total_workers: usize) -> bool {
        match self {
            Self::MixedRw { read_pct } => worker_id < reader_count(total_workers, *read_pct),
            _ => false,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct TraceEventIn {
    pub event_id: String,
    pub event_type: String,
    pub subject: String,
    pub schema_version: u16,
    pub emitted_at: DateTime<Utc>,
    pub correlation_id: Option<String>,
    pub payload: serde_json::Value,
}

pub fn load_trace_events(
    trace_path: &Path,
    event_type_filter: Option<&str>,
) -> Result<Vec<TraceEventIn>> {
    let file = File::open(trace_path)?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();

    for (line_no, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let event: TraceEventIn = serde_json::from_str(&line)
            .map_err(|e| anyhow!("failed to parse trace JSONL line {}: {e}", line_no + 1))?;

        if let Some(filter) = event_type_filter {
            if !event.event_type.contains(filter) {
                continue;
            }
        }
        events.push(event);
    }

    events.sort_by_key(|event| event.emitted_at);

    if events.is_empty() {
        return Err(anyhow!(
            "trace file {} produced zero events after filtering",
            trace_path.display()
        ));
    }

    Ok(events)
}

pub fn assign_trace_events(workers: usize, events: Vec<TraceEventIn>) -> Result<Vec<Vec<TraceEventIn>>> {
    if workers == 0 {
        return Err(anyhow!("--workers must be greater than zero"));
    }

    let mut assignments = vec![Vec::new(); workers];
    let mut round_robin = 0usize;

    for event in events {
        let worker_index = match event.correlation_id.as_deref() {
            Some(correlation_id) => stable_worker_index(correlation_id, workers),
            None => {
                let current = round_robin % workers;
                round_robin = round_robin.wrapping_add(1);
                current
            }
        };
        assignments[worker_index].push(event);
    }

    Ok(assignments)
}

fn stable_worker_index(value: &str, workers: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    (hasher.finish() as usize) % workers
}

fn reader_count(total_workers: usize, read_pct: u8) -> usize {
    if total_workers == 0 || read_pct == 0 {
        return 0;
    }
    ((total_workers * usize::from(read_pct)) / 100).min(total_workers)
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
            WorkloadProfile::Sustained
            | WorkloadProfile::LongTx { .. }
            | WorkloadProfile::MixedRw { .. } => {
                let sleep_secs = 3 + (self.jitter() % 8);
                tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
            }
            WorkloadProfile::TraceReplay { .. } => {}
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
