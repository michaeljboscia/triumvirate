use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRequest {
    pub fleet_id: Option<String>,
    pub author_agent: String,
    pub artifact: String,
    pub review_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRecord {
    pub review_id: String,
    pub fleet_id: Option<String>,
    pub author_agent: String,
    pub reviewer_agent: Option<String>,
    pub verdict: Option<String>,
    pub comments: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct PeerReviewEngine {
    db_path: PathBuf,
    reviewers: Vec<String>,
    round_robin: Arc<Mutex<usize>>,
}

impl PeerReviewEngine {
    pub fn new(project_root: PathBuf) -> anyhow::Result<Self> {
        if !project_root.is_absolute() {
            anyhow::bail!("project_root must be absolute");
        }
        Ok(Self {
            db_path: project_root.join(".triumvirate").join("ledger.db"),
            reviewers: vec!["codex".to_string(), "gemini".to_string(), "claude".to_string()],
            round_robin: Arc::new(Mutex::new(0)),
        })
    }

    pub fn request_review(&self, req: ReviewRequest) -> anyhow::Result<ReviewRecord> {
        let reviewer = self.next_reviewer(&req.author_agent)?;
        if reviewer.eq_ignore_ascii_case(&req.author_agent) {
            anyhow::bail!("author cannot review own output");
        }
        let review_id = format!("review-{}", Uuid::new_v4());
        let conn = self.open_conn()?;
        let inflight: i64 = conn.query_row(
            "SELECT COUNT(*) FROM reviews WHERE state = 'in_progress'",
            [],
            |row| row.get(0),
        )?;
        let max_inflight = max_inflight_limit() as i64;
        let state = if inflight < max_inflight {
            "in_progress"
        } else {
            "pending"
        };
        conn.execute(
            "INSERT INTO reviews (review_id, fleet_id, author_agent, reviewer_agent, artifact, review_type, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                review_id,
                req.fleet_id,
                req.author_agent,
                reviewer,
                req.artifact,
                req.review_type,
                state
            ],
        )?;
        self.get_review(&review_id)?
            .ok_or_else(|| anyhow::anyhow!("review insert did not persist"))
    }

    pub fn submit_review(
        &self,
        review_id: &str,
        verdict: &str,
        comments: Option<&str>,
    ) -> anyhow::Result<()> {
        let conn = self.open_conn()?;
        let updated = conn.execute(
            "UPDATE reviews
             SET verdict = ?2, comments = ?3, reviewed_at = datetime('now'), state = 'done'
             WHERE review_id = ?1",
            rusqlite::params![review_id, verdict, comments],
        )?;
        if updated == 0 {
            anyhow::bail!("review not found: {review_id}");
        }
        Ok(())
    }

    pub fn get_review(&self, review_id: &str) -> anyhow::Result<Option<ReviewRecord>> {
        let conn = self.open_conn()?;
        conn.query_row(
            "SELECT review_id, fleet_id, author_agent, reviewer_agent, verdict, comments, state
             FROM reviews
             WHERE review_id = ?1",
            [review_id],
            |row| {
                Ok(ReviewRecord {
                    review_id: row.get(0)?,
                    fleet_id: row.get(1)?,
                    author_agent: row.get(2)?,
                    reviewer_agent: row.get(3)?,
                    verdict: row.get(4)?,
                    comments: row.get(5)?,
                    state: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn fail_timed_out_reviews(&self, timeout_seconds: u64) -> anyhow::Result<usize> {
        let conn = self.open_conn()?;
        let updated = conn.execute(
            "UPDATE reviews
             SET state = 'failed', comments = COALESCE(comments, 'timeout')
             WHERE state = 'in_progress'
               AND datetime(requested_at) < datetime('now', ?1)",
            [format!("-{timeout_seconds} seconds")],
        )?;
        Ok(updated)
    }

    fn next_reviewer(&self, author_agent: &str) -> anyhow::Result<String> {
        let candidates = self
            .reviewers
            .iter()
            .filter(|name| !name.eq_ignore_ascii_case(author_agent))
            .cloned()
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            anyhow::bail!("no non-author reviewers available");
        }
        let mut idx = self
            .round_robin
            .lock()
            .map_err(|_| anyhow::anyhow!("round robin mutex poisoned"))?;
        let reviewer = candidates[*idx % candidates.len()].clone();
        *idx = idx.saturating_add(1);
        Ok(reviewer)
    }

    fn open_conn(&self) -> anyhow::Result<Connection> {
        if !self.db_path.exists() {
            anyhow::bail!("ledger database missing at {}", self.db_path.display());
        }
        let conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(conn)
    }
}

fn max_inflight_limit() -> usize {
    std::env::var("TRIUMVIRATE_REVIEW_MAX_INFLIGHT")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(2)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ledger::LedgerStore;

    use super::{PeerReviewEngine, ReviewRequest};

    #[test]
    fn review_assignment_queue_and_timeout_behave_as_expected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool")).expect("spool");
        let _store = LedgerStore::open(project_root.clone()).expect("open ledger");
        // SAFETY: test controls env var lifecycle.
        unsafe {
            std::env::set_var("TRIUMVIRATE_REVIEW_MAX_INFLIGHT", "2");
        }

        let engine = PeerReviewEngine::new(project_root.clone()).expect("engine");
        let mut ids = Vec::new();
        for idx in 0..5 {
            let review = engine
                .request_review(ReviewRequest {
                    fleet_id: Some("fleet-1".to_string()),
                    author_agent: "codex".to_string(),
                    artifact: format!("diff-{idx}"),
                    review_type: "code".to_string(),
                })
                .expect("request review");
            assert_ne!(
                review
                    .reviewer_agent
                    .as_deref()
                    .expect("reviewer assigned"),
                "codex"
            );
            ids.push(review.review_id);
        }

        let conn = rusqlite::Connection::open(project_root.join(".triumvirate").join("ledger.db"))
            .expect("open sqlite");
        let in_progress: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM reviews WHERE state = 'in_progress'",
                [],
                |row| row.get(0),
            )
            .expect("count in progress");
        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM reviews WHERE state = 'pending'",
                [],
                |row| row.get(0),
            )
            .expect("count pending");
        assert_eq!(in_progress, 2);
        assert_eq!(pending, 3);

        conn.execute(
            "UPDATE reviews
             SET requested_at = datetime('now', '-130 seconds')
             WHERE review_id = ?1",
            [ids[0].as_str()],
        )
        .expect("backdate review");
        let timed_out = engine
            .fail_timed_out_reviews(120)
            .expect("fail timed out reviews");
        assert!(timed_out >= 1);
        let updated_state = engine
            .get_review(&ids[0])
            .expect("get review")
            .expect("review present")
            .state;
        assert_eq!(updated_state, "failed");

        // SAFETY: test controls env var lifecycle.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_REVIEW_MAX_INFLIGHT");
        }
    }
}
