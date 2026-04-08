use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaveTask {
    pub task_id: String,
    pub allowed_files: Vec<String>,
    pub validation_status: String,
}

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

pub fn gate_wave(tasks: &[WaveTask]) -> anyhow::Result<String> {
    validate_no_overlap(tasks)?;
    if tasks
        .iter()
        .any(|t| !t.validation_status.eq_ignore_ascii_case("PASS"))
    {
        anyhow::bail!("wave gate failed: at least one task is not PASS");
    }

    let summary = format!(
        "wave_gate=PASS tasks={} files={} ",
        tasks.len(),
        tasks.iter().map(|t| t.allowed_files.len()).sum::<usize>()
    );
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
        assert!(gate_wave(&tasks).is_err());
    }
}
