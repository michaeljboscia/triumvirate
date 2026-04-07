use std::path::{Path, PathBuf};

use shared_types::{
    DrainResult, GcResult, HealthStatus, Lesson, ManualRecord, NewLesson, RawEvent, SessionDetail,
    Summary,
};

#[derive(Debug)]
pub struct LedgerStore {
    project_root: PathBuf,
}

impl LedgerStore {
    pub fn open(project_root: PathBuf) -> anyhow::Result<Self> {
        if !project_root.is_absolute() {
            anyhow::bail!("project_root must be an absolute path");
        }
        Ok(Self { project_root })
    }

    pub fn ingest_event(&self, _event: RawEvent) -> anyhow::Result<()> {
        anyhow::bail!("not implemented")
    }

    pub fn drain_spool(&self, _spool_dir: &Path) -> anyhow::Result<DrainResult> {
        anyhow::bail!("not implemented")
    }

    pub fn query(&self, _query: &str, _limit: usize) -> anyhow::Result<Vec<Summary>> {
        anyhow::bail!("not implemented")
    }

    pub fn get_session(&self, _session_id: &str) -> anyhow::Result<SessionDetail> {
        anyhow::bail!("not implemented")
    }

    pub fn record(&self, _record: ManualRecord) -> anyhow::Result<()> {
        anyhow::bail!("not implemented")
    }

    pub fn health(&self) -> anyhow::Result<HealthStatus> {
        anyhow::bail!("not implemented")
    }

    pub fn add_lesson(&self, _lesson: NewLesson) -> anyhow::Result<i64> {
        anyhow::bail!("not implemented")
    }

    pub fn query_lessons(&self, _query: &str, _min_confidence: f64) -> anyhow::Result<Vec<Lesson>> {
        anyhow::bail!("not implemented")
    }

    pub fn validate_lesson(&self, _lesson_id: i64) -> anyhow::Result<()> {
        anyhow::bail!("not implemented")
    }

    pub fn gc(&self) -> anyhow::Result<GcResult> {
        anyhow::bail!("not implemented")
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::LedgerStore;

    #[test]
    fn open_rejects_relative_paths() {
        let err = LedgerStore::open(PathBuf::from("relative/path"))
            .expect_err("relative paths must be rejected");
        assert!(err.to_string().contains("absolute path"));
    }

    #[test]
    fn open_accepts_absolute_paths() {
        let store = LedgerStore::open(PathBuf::from("/tmp/triumvirate-project"))
            .expect("absolute paths should be accepted");
        assert_eq!(store.project_root(), PathBuf::from("/tmp/triumvirate-project"));
    }
}
