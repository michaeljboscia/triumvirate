use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::metrics::MetricsSummary;
use crate::workload::WorkloadProfile;

#[derive(Debug, Serialize)]
pub struct RunReport {
    pub run_id: String,
    pub profile: WorkloadProfile,
    pub workers: usize,
    pub duration_secs: u64,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub metrics: MetricsSummary,
}

pub fn write_reports(report: &RunReport, out_dir: &Path) -> Result<(PathBuf, PathBuf)> {
    fs::create_dir_all(out_dir)?;

    let json_path = out_dir.join(format!("{}.json", report.run_id));
    let md_path = out_dir.join(format!("{}.md", report.run_id));

    let json = serde_json::to_string_pretty(report)?;
    fs::write(&json_path, json)?;

    let markdown = render_markdown(report);
    fs::write(&md_path, markdown)?;

    Ok((json_path, md_path))
}

fn render_markdown(report: &RunReport) -> String {
    format!(
        "# SQLite Concurrency Stress Test\n\n\
Run ID: `{}`\n\
Profile: `{:?}`\n\
Workers: `{}`\n\
Duration: `{}` seconds\n\
Started (UTC): `{}`\n\
Finished (UTC): `{}`\n\n\
## Latency\n\n\
- p50: `{:.3}` ms\n\
- p95: `{:.3}` ms\n\
- p99: `{:.3}` ms\n\
- p99.9: `{:.3}` ms\n\n\
## Reliability\n\n\
- SQLITE_BUSY retries: `{}`\n\
- BUSY rate: `{:.4}`\n\
- Successful ops: `{}`\n\
- Failed ops: `{}`\n\n\
## WAL and Host\n\n\
- WAL peak: `{:.3}` MB\n\
- Avg iowait: `{:.3}` %\n",
        report.run_id,
        report.profile,
        report.workers,
        report.duration_secs,
        report.started_at,
        report.finished_at,
        report.metrics.p50_ms,
        report.metrics.p95_ms,
        report.metrics.p99_ms,
        report.metrics.p99_9_ms,
        report.metrics.busy_retries,
        report.metrics.busy_rate,
        report.metrics.successful_ops,
        report.metrics.failed_ops,
        report.metrics.wal_peak_mb,
        report.metrics.avg_iowait_pct,
    )
}
