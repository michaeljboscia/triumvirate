//! Daemon runtime core boundary.
//!
//! This crate is the extraction target for daemon-only orchestration logic.

use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
use serde::{Serialize, de::DeserializeOwned};
use shared_types::{MemoryEntry, SessionState};
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::instrument;
use uuid::Uuid;

pub mod version;
pub mod metrics;
pub mod observability;
pub mod sequencer;
pub mod pid;
pub mod replay;
pub mod pantheon_session;
pub use version::{NAME, VERSION};
pub use sequencer::EventSequencer;
pub use pid::PidFile;
pub use replay::{EventReplayBuffer, ReplayResult, DEFAULT_CAPACITY as REPLAY_BUFFER_DEFAULT_CAPACITY};
pub use pantheon_session::{
    PANTHEON_SESSION, PantheonSessionContext, current_parent_session_id,
    current_pantheon_session, current_root_session_id,
};

#[instrument(skip_all)]
pub fn dead_drop_dir(root: &Path) -> PathBuf {
    root.join("dead-drop")
}

#[instrument(skip_all)]
pub fn triumvirate_home_dir() -> anyhow::Result<PathBuf> {
    if let Ok(override_dir) = std::env::var("TRIUMVIRATE_HOME") {
        return Ok(PathBuf::from(override_dir));
    }
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("failed to determine user home directory"))?;
    Ok(home.join(".triumvirate"))
}

pub struct DeadDropTicket<'a> {
    pub agent: &'a str,
    pub message: &'a str,
    pub reason: &'a str,
    pub cwd: Option<&'a str>,
    pub repo: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub id: &'a str,
}

#[instrument(skip_all)]
pub fn create_dead_drop_ticket(
    root: &Path,
    ticket: DeadDropTicket<'_>,
) -> anyhow::Result<PathBuf> {
    let dir = dead_drop_dir(root);
    fs::create_dir_all(&dir)?;
    let file = dir.join(format!("{}-{}.md", ticket.id, ticket.agent));
    let body = format!(
        "# Dead Drop Fallback\n\nid: {}\nagent: {}\nreason: {}\n\
cwd: {}\nrepo: {}\nbranch: {}\n\n## Original Request\n{message}\n",
        ticket.id,
        ticket.agent,
        ticket.reason,
        ticket.cwd.unwrap_or_default(),
        ticket.repo.unwrap_or_default(),
        ticket.branch.unwrap_or_default(),
        message = ticket.message
    );
    fs::write(&file, body)?;
    Ok(file)
}

#[instrument(skip_all)]
pub fn count_dead_drop_tickets(root: &Path) -> anyhow::Result<usize> {
    let dir = dead_drop_dir(root);
    if !dir.exists() {
        return Ok(0);
    }
    Ok(fs::read_dir(dir)?.filter_map(Result::ok).count())
}

#[instrument(skip_all)]
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

#[instrument(skip_all)]
pub fn acknowledge_dead_drop_ticket(root: &Path, path: &str) -> anyhow::Result<()> {
    let root = dead_drop_dir(root).canonicalize()?;
    let requested = PathBuf::from(path).canonicalize()?;
    if !requested.starts_with(&root) {
        anyhow::bail!("path is outside dead-drop directory");
    }
    fs::remove_file(requested)?;
    Ok(())
}

#[instrument(skip_all)]
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

#[instrument(skip_all)]
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

#[instrument(skip_all)]
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

#[instrument(skip_all)]
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

#[instrument(skip_all)]
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

#[instrument(skip_all)]
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

#[instrument(skip_all)]
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

#[instrument(skip_all)]
pub fn sessions_file_path(root: &Path) -> PathBuf {
    root.join("sessions.json")
}

#[instrument(skip_all)]
pub fn load_json_file<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    if !path.exists() {
        anyhow::bail!("file does not exist: {}", path.display());
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str::<T>(&raw)?)
}

#[instrument(skip_all)]
pub fn load_json_file_if_exists<T: DeserializeOwned + Default>(path: &Path) -> anyhow::Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    load_json_file(path)
}

#[instrument(skip_all)]
pub fn persist_json_file<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

#[instrument(skip_all)]
pub fn persist_json_file_if_enabled<T: Serialize>(
    maybe_path: Option<&PathBuf>,
    value: &T,
) -> anyhow::Result<()> {
    let Some(path) = maybe_path else {
        return Ok(());
    };
    persist_json_file(path, value)
}

#[instrument(skip_all)]
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

#[instrument(skip_all)]
pub fn launchd_plist_path() -> anyhow::Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("failed to determine user home directory"))?
        .join("Library/LaunchAgents/com.triumvirate.daemon-v2.plist"))
}

#[instrument(skip_all)]
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

#[instrument(skip_all)]
pub fn unix_time_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[instrument(skip_all)]
pub fn daemon_bind_addr(var_value: Option<&str>) -> String {
    var_value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("127.0.0.1:8080")
        .to_string()
}

#[instrument(skip_all)]
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

/// Narrow interface for session storage, received by mcp-tools inter_agent handlers.
/// Implemented in Wave 1 T-003.
pub trait SessionStore: Send + Sync {}

/// Narrow interface for agent execution, received by mcp-tools inter_agent handlers.
/// Implemented in Wave 1 T-003.
pub trait AgentExecutor: Send + Sync {}

/// Narrow interface for ABE task tracking, received by mcp-tools abe handlers.
/// Implemented in Wave 1 T-004.
pub trait TaskTrackerHandle: Send + Sync {}

/// Narrow interface for ledger/lessons/memory access, received by mcp-tools knowledge handlers.
/// Implemented in Wave 1 T-006.
pub trait LedgerStoreFactory: Send + Sync {}

#[instrument(skip_all)]
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

#[derive(Debug, Clone)]
pub struct DaemonState<TAbeTasks> {
    pub token: String,
    pub queues: QueueRegistry,
    pub bind_addr: String,
    pub sessions: Arc<Mutex<HashMap<String, SessionState>>>,
    pub sessions_file: Option<PathBuf>,
    pub abe_tasks: TAbeTasks,
    pub ledger_project_lru: Arc<Mutex<VecDeque<PathBuf>>>,
    pub marker_parse_window: Arc<Mutex<VecDeque<(Instant, bool)>>>,
    pub metrics: Arc<metrics::DaemonMetrics>,
    pub ws_events: tokio::sync::broadcast::Sender<String>,
}

impl<TAbeTasks> DaemonState<TAbeTasks> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        token: String,
        queues: QueueRegistry,
        bind_addr: String,
        sessions: Arc<Mutex<HashMap<String, SessionState>>>,
        sessions_file: Option<PathBuf>,
        abe_tasks: TAbeTasks,
        ledger_project_lru: Arc<Mutex<VecDeque<PathBuf>>>,
        marker_parse_window: Arc<Mutex<VecDeque<(Instant, bool)>>>,
        metrics: Arc<metrics::DaemonMetrics>,
        ws_events: tokio::sync::broadcast::Sender<String>,
    ) -> Self {
        Self {
            token,
            queues,
            bind_addr,
            sessions,
            sessions_file,
            abe_tasks,
            ledger_project_lru,
            marker_parse_window,
            metrics,
            ws_events,
        }
    }
}

pub fn encode_ws_event(event_type: &str, payload: serde_json::Value) -> String {
    serde_json::json!({
        "type": event_type,
        "ts_ms": unix_time_ms(),
        "payload": payload
    })
    .to_string()
}

pub fn publish_ws_event<TAbeTasks>(
    state: &DaemonState<TAbeTasks>,
    event_type: &str,
    payload: serde_json::Value,
) {
    let _ = state.ws_events.send(encode_ws_event(event_type, payload));
}

#[instrument(skip_all)]
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
        let _ = super::create_dead_drop_ticket(&root, super::DeadDropTicket {
            agent: "gemini",
            message: "hello",
            reason: "timeout",
            cwd: None,
            repo: None,
            branch: None,
            id,
        })?;
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
                working_state: None,
                token_usage: None,
                tool_name: None,
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

    #[test]
    fn daemon_bind_addr_defaults_and_overrides() {
        assert_eq!(super::daemon_bind_addr(None), "127.0.0.1:8080");
        assert_eq!(
            super::daemon_bind_addr(Some("0.0.0.0:9000")),
            "0.0.0.0:9000"
        );
        assert_eq!(super::daemon_bind_addr(Some("   ")), "127.0.0.1:8080");
    }
}
