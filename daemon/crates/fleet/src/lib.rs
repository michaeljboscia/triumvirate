//! Fleet orchestration crate.

pub mod worktree;
pub mod tasks;
pub mod orchestrator;
pub mod merge;
pub mod recovery;
pub mod worker_token;
pub mod shim;
pub mod index;
pub mod git_ops;

/// The time a ledger event happened. Events used to carry a hardcoded "2030-01-01T00:00:00Z",
/// a fabricated value in every fleet's history. The ledger dedupes on (session_id, event_type,
/// sequence), never on the timestamp, so a retried write stays a duplicate it can catch.
pub(crate) fn event_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

use shared_types::GitOps;

#[derive(Debug, Clone)]
pub struct FleetEngine<G: GitOps> {
    git_ops: G,
}

impl<G: GitOps> FleetEngine<G> {
    pub fn new(git_ops: G) -> Self {
        Self { git_ops }
    }

    pub fn git_ops(&self) -> &G {
        &self.git_ops
    }
}

#[cfg(test)]
mod event_time_tests {
    /// RED IF: a fleet event is written with anything but the current time.
    #[test]
    fn a_task_completed_event_carries_the_real_time() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("project");
        std::fs::create_dir_all(root.join(".triumvirate").join("spool")).expect("spool");
        let _ = ledger::LedgerStore::open(root.clone()).expect("ledger");
        let tasks = crate::tasks::FleetTaskStore::new(root.clone()).expect("task store");
        tasks.insert_fleet("fleet-time", "t").expect("fleet");
        tasks.insert_task("fleet-time-T-001", "fleet-time", "t", &[]).expect("task");
        let before = chrono::Utc::now();
        tasks.complete_task("fleet-time-T-001").expect("complete");
        let conn = rusqlite::Connection::open(root.join(".triumvirate").join("ledger.db")).expect("sqlite");
        let ts: String = conn
            .query_row(
                "SELECT timestamp FROM events WHERE session_id = 'fleet-time' AND event_type = 'task_completed'",
                [],
                |r| r.get(0),
            )
            .expect("event row");
        let at = chrono::DateTime::parse_from_rfc3339(&ts).expect("rfc3339").with_timezone(&chrono::Utc);
        assert!(at >= before - chrono::Duration::seconds(1) && at <= chrono::Utc::now(), "event time {ts}");
    }

    /// The class, not the one call site: no production code in this crate may write a literal
    /// timestamp. RED IF any `timestamp: "...` string literal appears before a file's tests.
    #[test]
    fn no_production_event_carries_a_literal_timestamp() {
        let src_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let needle = ["timestamp: ", "\""].concat();
        let mut checked = 0;
        for entry in std::fs::read_dir(&src_dir).expect("src dir") {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).expect("read source");
            let prod = src.split("#[cfg(test)]").next().unwrap_or(&src);
            assert!(!prod.contains(&needle), "{} writes a literal event timestamp", path.display());
            checked += 1;
        }
        assert!(checked >= 10, "only {checked} source files found under {}", src_dir.display());
    }
}
