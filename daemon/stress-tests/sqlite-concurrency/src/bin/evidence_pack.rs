use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use serde_json::Value;

#[derive(Debug, Parser)]
#[command(name = "evidence-pack")]
struct Args {
    #[arg(long, default_value = "./results")]
    results_dir: PathBuf,

    #[arg(long, default_value = "./results/EVIDENCE_PACK.md")]
    out: PathBuf,
}

#[derive(Debug, Clone)]
struct MatrixReport {
    run_id: String,
    profile: String,
    workers: u64,
    started_at: Option<String>,
    duration_sec: Option<f64>,
    p95_ms: f64,
    p99_ms: f64,
    busy_rate: f64,
    wal_peak_mb: f64,
    successful_ops: u64,
    failed_ops: u64,
    process_cpu_pct: f64,
    system_load_avg_1m: f64,
}

#[derive(Debug, Clone)]
struct CrashReport {
    source: PathBuf,
    run_id: Option<String>,
    started_at: Option<String>,
    duration_sec: Option<f64>,
    total_trials: u64,
    all_integrity_ok: bool,
    trials: Vec<CrashTrial>,
}

#[derive(Debug, Clone)]
struct CrashTrial {
    trial_id: String,
    pre_crash_count: i64,
    post_crash_count: i64,
    delta: i64,
    integrity_ok: bool,
    wal_replay_ms: f64,
}

#[derive(Debug, Clone)]
struct InventoryItem {
    source: PathBuf,
    shape: &'static str,
    run_id: Option<String>,
    started_at: Option<String>,
    duration_sec: Option<f64>,
}

#[derive(Debug, Clone)]
struct UnknownReport {
    source: PathBuf,
    reason: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let json_files = discover_json_files(&args.results_dir)?;

    let mut matrix_reports = Vec::new();
    let mut crash_reports = Vec::new();
    let mut unknown_reports = Vec::new();
    let mut inventory = Vec::new();

    for path in json_files {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read JSON report: {}", path.display()))?;
        let value: Value = serde_json::from_str(&content)
            .with_context(|| format!("failed to parse JSON report: {}", path.display()))?;

        if is_matrix_report(&value) {
            let report = parse_matrix_report(&value);
            inventory.push(InventoryItem {
                source: path.clone(),
                shape: "matrix",
                run_id: Some(report.run_id.clone()),
                started_at: report.started_at.clone(),
                duration_sec: report.duration_sec,
            });
            matrix_reports.push(report);
            continue;
        }

        if is_crash_report(&value) {
            let report = parse_crash_report(&path, &value);
            inventory.push(InventoryItem {
                source: path.clone(),
                shape: "crash_trial",
                run_id: report.run_id.clone(),
                started_at: report.started_at.clone(),
                duration_sec: report.duration_sec,
            });
            crash_reports.push(report);
            continue;
        }

        inventory.push(InventoryItem {
            source: path.clone(),
            shape: "unknown",
            run_id: get_string(&value, "run_id"),
            started_at: get_string(&value, "started_at"),
            duration_sec: get_f64(&value, "duration_sec").or_else(|| get_f64(&value, "duration_secs")),
        });
        unknown_reports.push(UnknownReport {
            source: path,
            reason: "unknown report shape (missing expected keys)".to_string(),
        });
    }

    matrix_reports.sort_by(compare_matrix_reports);
    crash_reports.sort_by(|a, b| a.source.cmp(&b.source));
    for report in &mut crash_reports {
        report.trials.sort_by(compare_trials);
    }
    inventory.sort_by(|a, b| a.source.cmp(&b.source));
    unknown_reports.sort_by(|a, b| a.source.cmp(&b.source));

    let markdown = render_markdown(&matrix_reports, &crash_reports, &unknown_reports, &inventory);
    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create output directory for evidence pack: {}",
                parent.display()
            )
        })?;
    }
    fs::write(&args.out, markdown)
        .with_context(|| format!("failed to write output markdown: {}", args.out.display()))?;

    println!("wrote {}", args.out.display());
    Ok(())
}

fn discover_json_files(results_dir: &Path) -> Result<Vec<PathBuf>> {
    if !results_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    visit_dir(results_dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn visit_dir(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read directory {}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to read file type for {}", path.display()))?;
        if file_type.is_dir() {
            visit_dir(&path, files)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn is_matrix_report(value: &Value) -> bool {
    value.get("run_id").is_some()
        && value.get("workers").is_some()
        && value.get("profile").is_some()
        && value.get("metrics").is_some()
}

fn is_crash_report(value: &Value) -> bool {
    value.get("trials").is_some() && value.get("all_integrity_ok").is_some() && value.get("total_trials").is_some()
}

fn parse_matrix_report(value: &Value) -> MatrixReport {
    let metrics = value.get("metrics").unwrap_or(&Value::Null);
    MatrixReport {
        run_id: get_string(value, "run_id").unwrap_or_else(|| "n/a".to_string()),
        profile: get_string(value, "profile").unwrap_or_else(|| "unknown".to_string()),
        workers: get_u64(value, "workers").unwrap_or(0),
        started_at: get_string(value, "started_at"),
        duration_sec: get_f64(value, "duration_sec").or_else(|| get_f64(value, "duration_secs")),
        p95_ms: get_f64(metrics, "p95_ms").unwrap_or(0.0),
        p99_ms: get_f64(metrics, "p99_ms").unwrap_or(0.0),
        busy_rate: get_f64(metrics, "busy_rate").unwrap_or(0.0),
        wal_peak_mb: get_f64(metrics, "wal_peak_mb").unwrap_or(0.0),
        successful_ops: get_u64(metrics, "successful_ops").unwrap_or(0),
        failed_ops: get_u64(metrics, "failed_ops").unwrap_or(0),
        process_cpu_pct: get_f64(metrics, "process_cpu_pct").unwrap_or(0.0),
        system_load_avg_1m: get_f64(metrics, "system_load_avg_1m").unwrap_or(0.0),
    }
}

fn parse_crash_report(source: &Path, value: &Value) -> CrashReport {
    let trials = value
        .get("trials")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().map(parse_trial).collect::<Vec<_>>())
        .unwrap_or_default();

    CrashReport {
        source: source.to_path_buf(),
        run_id: get_string(value, "run_id"),
        started_at: get_string(value, "started_at"),
        duration_sec: get_f64(value, "duration_sec").or_else(|| get_f64(value, "duration_secs")),
        total_trials: get_u64(value, "total_trials").unwrap_or(trials.len() as u64),
        all_integrity_ok: get_bool(value, "all_integrity_ok").unwrap_or(false),
        trials,
    }
}

fn parse_trial(value: &Value) -> CrashTrial {
    let pre = get_i64(value, "pre_crash_count").unwrap_or(0);
    let post = get_i64(value, "post_crash_count").unwrap_or(0);
    CrashTrial {
        trial_id: get_string(value, "trial_id").unwrap_or_else(|| "unknown".to_string()),
        pre_crash_count: pre,
        post_crash_count: post,
        delta: get_i64(value, "delta").unwrap_or(post - pre),
        integrity_ok: get_bool(value, "integrity_ok").unwrap_or(false),
        wal_replay_ms: get_f64(value, "wal_replay_ms").unwrap_or(0.0),
    }
}

fn compare_matrix_reports(a: &MatrixReport, b: &MatrixReport) -> Ordering {
    profile_sort_key(&a.profile)
        .cmp(&profile_sort_key(&b.profile))
        .then(a.profile.cmp(&b.profile))
        .then(a.workers.cmp(&b.workers))
        .then(a.run_id.cmp(&b.run_id))
}

fn compare_trials(a: &CrashTrial, b: &CrashTrial) -> Ordering {
    parse_trial_id(&a.trial_id)
        .cmp(&parse_trial_id(&b.trial_id))
        .then(a.trial_id.cmp(&b.trial_id))
}

fn parse_trial_id(value: &str) -> Option<u64> {
    value.parse::<u64>().ok()
}

fn profile_sort_key(profile: &str) -> u8 {
    match profile {
        "sustained" => 0,
        "wave" => 1,
        "herd" => 2,
        "long-tx" => 3,
        "mixed-rw" => 4,
        _ => 255,
    }
}

fn render_markdown(
    matrix_reports: &[MatrixReport],
    crash_reports: &[CrashReport],
    unknown_reports: &[UnknownReport],
    inventory: &[InventoryItem],
) -> String {
    let mut out = String::new();
    out.push_str("# EVIDENCE PACK\n\n");
    out.push_str("Canonical aggregation of sqlite-concurrency stress-test results.\n\n");

    out.push_str("## Concurrency Matrix\n\n");
    if matrix_reports.is_empty() {
        out.push_str("- No matrix reports found.\n\n");
    } else {
        for profile in ["sustained", "wave", "herd", "long-tx", "mixed-rw"] {
            let profile_rows = matrix_reports
                .iter()
                .filter(|report| report.profile == profile)
                .collect::<Vec<_>>();
            if profile_rows.is_empty() {
                continue;
            }
            out.push_str(&format!("### Profile: `{}`\n\n", profile));
            out.push_str("| workers | p99_ms | p95_ms | busy_rate | wal_peak_mb | successful_ops | failed_ops | process_cpu_pct | system_load_avg_1m |\n");
            out.push_str("|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n");
            for row in profile_rows {
                let p99_cell = flagged_number(row.p99_ms, row.p99_ms > 500.0, "[FLAG P99]");
                let busy_cell = flagged_number(row.busy_rate, row.busy_rate > 0.01, "[FLAG BUSY]");
                out.push_str(&format!(
                    "| {} | {} | {:.3} | {} | {:.3} | {} | {} | {:.2} | {:.2} |\n",
                    row.workers,
                    p99_cell,
                    row.p95_ms,
                    busy_cell,
                    row.wal_peak_mb,
                    row.successful_ops,
                    row.failed_ops,
                    row.process_cpu_pct,
                    row.system_load_avg_1m
                ));
            }
            out.push('\n');
        }
    }

    out.push_str("## Crash-Recovery Trials\n\n");
    if crash_reports.is_empty() {
        out.push_str("- No crash-recovery reports found.\n\n");
    } else {
        for report in crash_reports {
            let avg_wal_replay_ms = if report.trials.is_empty() {
                0.0
            } else {
                report.trials.iter().map(|trial| trial.wal_replay_ms).sum::<f64>() / report.trials.len() as f64
            };
            out.push_str(&format!(
                "### Report: `{}`\n\n",
                report
                    .run_id
                    .as_deref()
                    .unwrap_or_else(|| report.source.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown"))
            ));
            out.push_str(&format!(
                "- Summary: {} trials, all_integrity_ok=`{}`, avg wal_replay_ms=`{:.3}`\n\n",
                report.total_trials, report.all_integrity_ok, avg_wal_replay_ms
            ));
            out.push_str("| trial_id | pre_crash_count | post_crash_count | delta | integrity_ok | wal_replay_ms |\n");
            out.push_str("|---|---:|---:|---:|---:|---:|\n");
            for trial in &report.trials {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {:.3} |\n",
                    trial.trial_id,
                    trial.pre_crash_count,
                    trial.post_crash_count,
                    trial.delta,
                    trial.integrity_ok,
                    trial.wal_replay_ms
                ));
            }
            out.push('\n');
        }
    }

    let summary = summarize(matrix_reports, crash_reports);
    out.push_str("## Summary Verdict\n\n");
    out.push_str(&format!(
        "- Total successful ops: `{}`\n- Total failed ops: `{}`\n- Total ops: `{}`\n- Breaches: p99=`{}`, busy_rate=`{}`, failed_ops=`{}`, crash_integrity=`{}`\n",
        summary.total_successful_ops,
        summary.total_failed_ops,
        summary.total_ops,
        summary.p99_breaches,
        summary.busy_breaches,
        summary.failure_breaches,
        summary.crash_breaches
    ));
    out.push('\n');
    out.push_str(&format!(
        "- ADR-001 Tier 1 claim `Latency p99 <= 500ms`: **{}**\n",
        pass_fail(summary.p99_breaches == 0)
    ));
    out.push_str(&format!(
        "- ADR-001 Tier 1 claim `Contention busy_rate <= 0.01`: **{}**\n",
        pass_fail(summary.busy_breaches == 0)
    ));
    out.push_str(&format!(
        "- ADR-001 Tier 1 claim `Operational reliability (failed_ops == 0)`: **{}**\n",
        pass_fail(summary.failure_breaches == 0)
    ));
    out.push_str(&format!(
        "- ADR-001 Tier 1 claim `Crash integrity preserved`: **{}**\n",
        pass_fail(summary.crash_breaches == 0)
    ));
    out.push_str(&format!(
        "- Aggregate ADR-001 Tier 1 verdict: **{}**\n\n",
        pass_fail(
            summary.p99_breaches == 0
                && summary.busy_breaches == 0
                && summary.failure_breaches == 0
                && summary.crash_breaches == 0
        )
    ));

    out.push_str("## Run Inventory\n\n");
    if inventory.is_empty() {
        out.push_str("- No JSON files discovered.\n\n");
    } else {
        out.push_str("| source_json | shape | run_id | started_at | duration_sec |\n");
        out.push_str("|---|---|---|---|---:|\n");
        for item in inventory {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                item.source.display(),
                item.shape,
                item.run_id.clone().unwrap_or_else(|| "n/a".to_string()),
                item.started_at.clone().unwrap_or_else(|| "n/a".to_string()),
                format_duration(item.duration_sec)
            ));
        }
        out.push('\n');
    }

    if !unknown_reports.is_empty() {
        out.push_str("## Skipped Unknown Reports\n\n");
        for unknown in unknown_reports {
            out.push_str(&format!(
                "- `{}`: {}\n",
                unknown.source.display(),
                unknown.reason
            ));
        }
        out.push('\n');
    }

    out
}

#[derive(Debug, Default)]
struct Summary {
    total_successful_ops: u64,
    total_failed_ops: u64,
    total_ops: u64,
    p99_breaches: u64,
    busy_breaches: u64,
    failure_breaches: u64,
    crash_breaches: u64,
}

fn summarize(matrix_reports: &[MatrixReport], crash_reports: &[CrashReport]) -> Summary {
    let mut summary = Summary::default();
    for report in matrix_reports {
        summary.total_successful_ops = summary.total_successful_ops.saturating_add(report.successful_ops);
        summary.total_failed_ops = summary.total_failed_ops.saturating_add(report.failed_ops);
        summary.total_ops = summary.total_ops.saturating_add(report.successful_ops.saturating_add(report.failed_ops));
        if report.p99_ms > 500.0 {
            summary.p99_breaches = summary.p99_breaches.saturating_add(1);
        }
        if report.busy_rate > 0.01 {
            summary.busy_breaches = summary.busy_breaches.saturating_add(1);
        }
        if report.failed_ops > 0 {
            summary.failure_breaches = summary.failure_breaches.saturating_add(1);
        }
    }
    for report in crash_reports {
        if !report.all_integrity_ok || report.trials.iter().any(|trial| !trial.integrity_ok) {
            summary.crash_breaches = summary.crash_breaches.saturating_add(1);
        }
    }
    summary
}

fn flagged_number(value: f64, flagged: bool, flag: &str) -> String {
    if flagged {
        format!("{:.3} {}", value, flag)
    } else {
        format!("{:.3}", value)
    }
}

fn pass_fail(pass: bool) -> &'static str {
    if pass {
        "PASS"
    } else {
        "FAIL"
    }
}

fn format_duration(duration_sec: Option<f64>) -> String {
    duration_sec
        .map(|value| format!("{:.3}", value))
        .unwrap_or_else(|| "n/a".to_string())
}

fn get_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(ToString::to_string)
}

fn get_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|v| {
        v.as_u64().or_else(|| {
            v.as_i64()
                .and_then(|i| if i >= 0 { Some(i as u64) } else { None })
        })
    })
}

fn get_i64(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))
}

fn get_f64(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
}

fn get_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}
