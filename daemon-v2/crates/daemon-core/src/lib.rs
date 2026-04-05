//! Daemon runtime core boundary.
//!
//! This crate is the extraction target for daemon-only orchestration logic.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use shared_types::MemoryEntry;

pub fn dead_drop_dir(root: &Path) -> PathBuf {
    root.join("dead-drop")
}

pub fn create_dead_drop_ticket(
    root: &Path,
    agent: &str,
    message: &str,
    reason: &str,
    cwd: &Option<String>,
    repo: &Option<String>,
    branch: &Option<String>,
    id: &str,
) -> anyhow::Result<PathBuf> {
    let dir = dead_drop_dir(root);
    fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{id}-{agent}.md"));
    let body = format!(
        "# Dead Drop Fallback\n\nid: {id}\nagent: {agent}\nreason: {reason}\n\
cwd: {}\nrepo: {}\nbranch: {}\n\n## Original Request\n{message}\n",
        cwd.clone().unwrap_or_default(),
        repo.clone().unwrap_or_default(),
        branch.clone().unwrap_or_default()
    );
    fs::write(&file, body)?;
    Ok(file)
}

pub fn count_dead_drop_tickets(root: &Path) -> anyhow::Result<usize> {
    let dir = dead_drop_dir(root);
    if !dir.exists() {
        return Ok(0);
    }
    Ok(fs::read_dir(dir)?.filter_map(Result::ok).count())
}

pub fn list_dead_drop_tickets(root: &Path, limit: usize) -> anyhow::Result<Vec<PathBuf>> {
    let dir = dead_drop_dir(root);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect::<Vec<_>>();
    files.sort();
    files.reverse();
    files.truncate(limit);
    Ok(files)
}

pub fn acknowledge_dead_drop_ticket(root: &Path, path: &str) -> anyhow::Result<()> {
    let root = dead_drop_dir(root).canonicalize()?;
    let requested = PathBuf::from(path).canonicalize()?;
    if !requested.starts_with(&root) {
        anyhow::bail!("path is outside dead-drop directory");
    }
    fs::remove_file(requested)?;
    Ok(())
}

pub fn gc_dead_drop_tickets(root: &Path, max_age_days: u64) -> anyhow::Result<usize> {
    let dir = dead_drop_dir(root);
    if !dir.exists() {
        return Ok(0);
    }
    let max_age = Duration::from_secs(max_age_days.saturating_mul(24 * 60 * 60));
    let now = SystemTime::now();
    let mut removed = 0usize;
    for entry in fs::read_dir(dir)?.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age >= max_age && fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn append_memory_entry(root: &Path, entry: &MemoryEntry) -> anyhow::Result<()> {
    let path = root.join("memory.jsonl");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

pub fn read_memory_entries(root: &Path) -> anyhow::Result<Vec<MemoryEntry>> {
    let path = root.join("memory.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let body = fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<MemoryEntry>(line) {
            out.push(entry);
        }
    }
    Ok(out)
}

pub fn write_scratchpad(
    root: &Path,
    project: &str,
    topic: &str,
    content: &str,
    now_ms: u128,
) -> anyhow::Result<PathBuf> {
    let safe_project = sanitize_name(project);
    let safe_topic = sanitize_name(topic);
    let dir = root.join("scratchpad").join(safe_project);
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{now_ms}-{safe_topic}.md"));
    fs::write(&path, content)?;
    Ok(path)
}

pub fn list_scratchpad(root: &Path, project: &str) -> anyhow::Result<Vec<PathBuf>> {
    let safe_project = sanitize_name(project);
    let dir = root.join("scratchpad").join(safe_project);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

pub fn append_outbox_event(root: &Path, event: &shared_types::OutboxEvent) -> anyhow::Result<()> {
    let path = root.join("outbox.jsonl");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(event)?;
    line.push('\n');
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

pub fn read_outbox_events(root: &Path) -> anyhow::Result<Vec<shared_types::OutboxEvent>> {
    let path = root.join("outbox.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let body = fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<shared_types::OutboxEvent>(line) {
            out.push(event);
        }
    }
    Ok(out)
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
}

#[cfg(test)]
mod tests {
    use shared_types::MemoryEntry;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn dead_drop_count_roundtrip() -> anyhow::Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!("daemon-core-dd-{now}"));
        let id = "abc";
        let _ = super::create_dead_drop_ticket(
            &root,
            "gemini",
            "hello",
            "timeout",
            &None,
            &None,
            &None,
            id,
        )?;
        assert_eq!(super::count_dead_drop_tickets(&root)?, 1);
        let _ = std::fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn memory_and_scratchpad_roundtrip() -> anyhow::Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!("daemon-core-ms-{now}"));
        std::fs::create_dir_all(&root)?;

        let entry = MemoryEntry {
            id: "1".to_string(),
            namespace: "ns".to_string(),
            key: "k".to_string(),
            value: "v".to_string(),
            ts_ms: 1,
        };
        super::append_memory_entry(&root, &entry)?;
        let entries = super::read_memory_entries(&root)?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].value, "v");

        let path = super::write_scratchpad(&root, "proj-a", "topic", "hello", 123)?;
        assert!(path.exists());
        let files = super::list_scratchpad(&root, "proj-a")?;
        assert_eq!(files.len(), 1);

        let _ = std::fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn outbox_roundtrip() -> anyhow::Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!("daemon-core-outbox-{now}"));
        std::fs::create_dir_all(&root)?;

        super::append_outbox_event(
            &root,
            &shared_types::OutboxEvent {
                ts_ms: 1,
                request_id: "r1".to_string(),
                tool: "ask_agent".to_string(),
                status: "DONE".to_string(),
                agent: Some("gemini".to_string()),
                detail: "ok".to_string(),
                cwd: None,
                repo: None,
                branch: None,
            },
        )?;
        let events = super::read_outbox_events(&root)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].request_id, "r1");

        let _ = std::fs::remove_dir_all(root);
        Ok(())
    }
}
