//! Daemon runtime core boundary.
//!
//! This crate is the extraction target for daemon-only orchestration logic.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
use serde::{Serialize, de::DeserializeOwned};
use shared_types::MemoryEntry;
use tokio::sync::Mutex;
use uuid::Uuid;

pub fn dead_drop_dir(root: &Path) -> PathBuf {
    root.join("dead-drop")
}

pub fn triumvirate_home_dir() -> anyhow::Result<PathBuf> {
    if let Ok(override_dir) = std::env::var("TRIUMVIRATE_HOME") {
        return Ok(PathBuf::from(override_dir));
    }
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("failed to determine user home directory"))?;
    Ok(home.join(".triumvirate"))
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

pub fn sessions_file_path(root: &Path) -> PathBuf {
    root.join("sessions.json")
}

pub fn load_json_file<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    if !path.exists() {
        anyhow::bail!("file does not exist: {}", path.display());
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str::<T>(&raw)?)
}

pub fn load_json_file_if_exists<T: DeserializeOwned + Default>(path: &Path) -> anyhow::Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    load_json_file(path)
}

pub fn persist_json_file<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

pub fn persist_json_file_if_enabled<T: Serialize>(
    maybe_path: Option<&PathBuf>,
    value: &T,
) -> anyhow::Result<()> {
    let Some(path) = maybe_path else {
        return Ok(());
    };
    persist_json_file(path, value)
}

pub fn ensure_daemon_token(root: &Path) -> anyhow::Result<String> {
    let token_path = root.join("daemon.token");
    if let Some(parent) = token_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if token_path.exists() {
        let existing = fs::read_to_string(&token_path)?;
        let token = existing.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let token = Uuid::new_v4().to_string();
    fs::write(&token_path, format!("{token}\n"))?;
    Ok(token)
}

pub fn launchd_plist_path() -> anyhow::Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("failed to determine user home directory"))?
        .join("Library/LaunchAgents/com.triumvirate.daemon-v2.plist"))
}

pub fn render_launch_agent_plist(exe_path: &str, home_dir: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.triumvirate.daemon-v2</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe_path}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>EnvironmentVariables</key>
  <dict>
    <key>TRIUMVIRATE_HOME</key>
    <string>{home_dir}</string>
  </dict>
  <key>StandardOutPath</key>
  <string>{home_dir}/daemon.log</string>
  <key>StandardErrorPath</key>
  <string>{home_dir}/daemon.err.log</string>
</dict>
</plist>
"#
    )
}

pub fn unix_time_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn resolve_context(
    cwd: Option<&String>,
    repo: Option<&String>,
    branch: Option<&String>,
) -> (Option<String>, Option<String>, Option<String>) {
    let resolved_cwd = cwd
        .cloned()
        .or_else(|| std::env::current_dir().ok().map(|p| p.display().to_string()));

    let resolved_repo = repo
        .cloned()
        .or_else(|| git_probe(&resolved_cwd, &["rev-parse", "--show-toplevel"]));

    let resolved_branch = branch
        .cloned()
        .or_else(|| git_probe(&resolved_cwd, &["rev-parse", "--abbrev-ref", "HEAD"]));

    (resolved_cwd, resolved_repo, resolved_branch)
}

fn git_probe(cwd: &Option<String>, args: &[&str]) -> Option<String> {
    let cwd = cwd.as_ref()?;
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if value.is_empty() { None } else { Some(value) }
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

pub fn project_queue_key(cwd: Option<&String>, repo: Option<&String>) -> String {
    if let Some(repo) = repo {
        return format!("repo:{repo}");
    }
    if let Some(cwd) = cwd {
        return format!("cwd:{cwd}");
    }
    "global".to_string()
}

pub type QueueRegistry = Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>;

pub async fn acquire_project_queue(registry: &QueueRegistry, key: String) -> Arc<Mutex<()>> {
    let mut queues = registry.lock().await;
    queues
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
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

    #[test]
    fn project_queue_key_prefers_repo_then_cwd() {
        assert_eq!(
            super::project_queue_key(Some(&"/tmp/a".to_string()), Some(&"triumvirate".to_string())),
            "repo:triumvirate"
        );
        assert_eq!(
            super::project_queue_key(Some(&"/tmp/a".to_string()), None),
            "cwd:/tmp/a"
        );
        assert_eq!(super::project_queue_key(None, None), "global");
    }

    #[test]
    fn token_and_sessions_roundtrip() -> anyhow::Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!("daemon-core-state-{now}"));
        std::fs::create_dir_all(&root)?;

        let token_one = super::ensure_daemon_token(&root)?;
        let token_two = super::ensure_daemon_token(&root)?;
        assert_eq!(token_one, token_two);

        let sessions = HashMap::from([("s1".to_string(), "gemini".to_string())]);
        let path = super::sessions_file_path(&root);
        super::persist_json_file(&path, &sessions)?;
        let loaded: HashMap<String, String> = super::load_json_file(&path)?;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get("s1").map(String::as_str), Some("gemini"));

        let loaded_default: HashMap<String, String> =
            super::load_json_file_if_exists(&root.join("missing.json"))?;
        assert!(loaded_default.is_empty());

        super::persist_json_file_if_enabled(Some(&path), &sessions)?;
        super::persist_json_file_if_enabled::<HashMap<String, String>>(None, &HashMap::new())?;

        let _ = std::fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn resolve_context_prefers_explicit_values() {
        let (cwd, repo, branch) = super::resolve_context(
            Some(&"/tmp/x".to_string()),
            Some(&"my-repo".to_string()),
            Some(&"feat/test".to_string()),
        );
        assert_eq!(cwd.as_deref(), Some("/tmp/x"));
        assert_eq!(repo.as_deref(), Some("my-repo"));
        assert_eq!(branch.as_deref(), Some("feat/test"));
    }

    #[test]
    fn launch_plist_render_has_expected_label() {
        let plist = super::render_launch_agent_plist("/usr/local/bin/triumvirate", "/tmp/tri");
        assert!(plist.contains("com.triumvirate.daemon-v2"));
        assert!(plist.contains("<string>daemon</string>"));
    }

    #[test]
    fn home_dir_prefers_env_override() -> anyhow::Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let override_dir = std::env::temp_dir().join(format!("daemon-core-home-{now}"));
        let override_dir_str = override_dir.display().to_string();

        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::set_var("TRIUMVIRATE_HOME", &override_dir_str) };
        let resolved = super::triumvirate_home_dir()?;
        assert_eq!(resolved, PathBuf::from(&override_dir_str));
        // SAFETY: test controls env var lifecycle in-process.
        unsafe { std::env::remove_var("TRIUMVIRATE_HOME") };
        Ok(())
    }
}
