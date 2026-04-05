//! Daemon runtime core boundary.
//!
//! This crate is the extraction target for daemon-only orchestration logic.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

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

#[cfg(test)]
mod tests {
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
}
