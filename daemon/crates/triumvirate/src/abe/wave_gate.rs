use std::collections::HashSet;
use std::process::Command;
use tracing::instrument;
use std::time::Instant;

use daemon_core::{encode_ws_event, metrics::DaemonMetrics};
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveTask {
    pub task_id: String,
    pub allowed_files: Vec<String>,
    pub validation_status: String,
}

#[instrument(skip_all)]
pub fn validate_no_overlap(tasks: &[WaveTask]) -> anyhow::Result<()> {
    let mut seen = HashSet::new();
    for task in tasks {
        for file in &task.allowed_files {
            let inserted = seen.insert(file.clone());
            if !inserted {
                anyhow::bail!("overlapping allowed_files entry detected: {file}");
            }
        }
    }
    Ok(())
}

#[instrument(skip_all)]
pub fn gate_wave<F>(
    tasks: &[WaveTask],
    test_command: &str,
    gemini_review: F,
) -> anyhow::Result<String>
where
    F: Fn(&[WaveTask]) -> anyhow::Result<String>,
{
    gate_wave_with_metrics(tasks, test_command, gemini_review, 0, None, None)
}

pub fn gate_wave_with_metrics<F>(
    tasks: &[WaveTask],
    test_command: &str,
    gemini_review: F,
    wave: u32,
    metrics: Option<&DaemonMetrics>,
    ws_events: Option<&broadcast::Sender<String>>,
) -> anyhow::Result<String>
where
    F: Fn(&[WaveTask]) -> anyhow::Result<String>,
{
    let started = Instant::now();
    if let Some(ws_events) = ws_events {
        let _ = ws_events.send(encode_ws_event(
            "abe_wave_state",
            serde_json::json!({
                "wave": wave,
                "status": "running",
                "task_count": tasks.len(),
                "duration_ms": 0,
            }),
        ));
    }
    validate_no_overlap(tasks)?;
    if tasks
        .iter()
        .any(|t| !t.validation_status.eq_ignore_ascii_case("PASS"))
    {
        anyhow::bail!("wave gate failed: at least one task is not PASS");
    }
    if !test_command.trim().is_empty() {
        let output = Command::new("sh")
            .arg("-lc")
            .arg(test_command)
            .output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("wave gate test suite failed: {}", stderr.trim());
        }
    }
    let gemini_summary = gemini_review(tasks)?;
    let test_result = if test_command.trim().is_empty() {
        "skipped"
    } else {
        "pass"
    };
    tracing::info!(
        wave,
        test_result,
        review_verdict = %gemini_summary,
        "abe_wave_gate_passed"
    );

    let summary = format!(
        "wave_gate=PASS tasks={} files={} review={}",
        tasks.len(),
        tasks.iter().map(|t| t.allowed_files.len()).sum::<usize>(),
        gemini_summary
    );
    if let Some(metrics) = metrics {
        metrics
            .abe_wave_duration_seconds
            .with_label_values(&[&wave.to_string()])
            .observe(started.elapsed().as_secs_f64());
    }
    if let Some(ws_events) = ws_events {
        let _ = ws_events.send(encode_ws_event(
            "abe_wave_state",
            serde_json::json!({
                "wave": wave,
                "status": "gate_passed",
                "task_count": tasks.len(),
                "duration_ms": started.elapsed().as_millis(),
            }),
        ));
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::{gate_wave, validate_no_overlap, WaveTask};

    #[test]
    fn no_overlap_validation_detects_duplicates() {
        let tasks = vec![
            WaveTask {
                task_id: "T-1".to_string(),
                allowed_files: vec!["a.rs".to_string()],
                validation_status: "PASS".to_string(),
            },
            WaveTask {
                task_id: "T-2".to_string(),
                allowed_files: vec!["a.rs".to_string()],
                validation_status: "PASS".to_string(),
            },
        ];
        assert!(validate_no_overlap(&tasks).is_err());
    }

    #[test]
    fn gate_requires_all_pass() {
        let tasks = vec![
            WaveTask {
                task_id: "T-1".to_string(),
                allowed_files: vec!["a.rs".to_string()],
                validation_status: "PASS".to_string(),
            },
            WaveTask {
                task_id: "T-2".to_string(),
                allowed_files: vec!["b.rs".to_string()],
                validation_status: "BLOCKED".to_string(),
            },
        ];
        assert!(gate_wave(&tasks, "true", |_| Ok("clean".to_string())).is_err());
    }

    #[test]
    fn gate_runs_test_command_and_review() {
        let tasks = vec![WaveTask {
            task_id: "T-1".to_string(),
            allowed_files: vec!["a.rs".to_string()],
            validation_status: "PASS".to_string(),
        }];
        let summary = gate_wave(&tasks, "true", |_| Ok("clean".to_string())).expect("gate pass");
        assert!(summary.contains("review=clean"));
        assert!(gate_wave(&tasks, "false", |_| Ok("clean".to_string())).is_err());
    }
}
