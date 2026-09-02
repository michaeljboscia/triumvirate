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

/// The default panel, overridable with `TRIUMVIRATE_PEER_REVIEWERS` (comma separated).
///
/// FIND-GROK-04. Dropping a reviewer used to require patching this file. An operator whose grok
/// is slow, rate limited, or logged out had no way to run a review without it except editing
/// Rust, which means in practice they turn the whole review off instead.
///
/// The default is unchanged and still includes grok, which was an explicit ruling. This is
/// about HOW the seat is filled, not whether it exists.
///
/// Unknown names are kept rather than filtered: an unroutable reviewer must fail loudly at
/// dispatch, not vanish from the panel silently, which would look like the review passing.
pub fn default_reviewers() -> Vec<String> {
    const DEFAULT: &[&str] = &["codex", "gemini", "grok", "claude"];
    match std::env::var("TRIUMVIRATE_PEER_REVIEWERS") {
        Ok(raw) => {
            let picked: Vec<String> = raw
                .split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            if picked.is_empty() {
                DEFAULT.iter().map(|s| s.to_string()).collect()
            } else {
                picked
            }
        }
        Err(_) => DEFAULT.iter().map(|s| s.to_string()).collect(),
    }
}

/// Serialises every test that touches `TRIUMVIRATE_PEER_REVIEWERS`.
///
/// It lives at crate scope because the tests that MUTATE that variable and the tests that READ
/// it are in two different modules, and a per-module lock does not serialise across them. That
/// is not hypothetical: `u_pr_grok_is_a_default_reviewer` began failing intermittently the
/// moment the roster tests were added, Antigravity reported it, and I dismissed it as its own
/// mutation. It was real.
#[cfg(test)]
pub(crate) fn reviewer_env_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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
            // REQ-GROK-003: grok is a DEFAULT reviewer. "Peer review" means all three CLI
            // peers (codex, antigravity/gemini, grok) plus claude. DeepSeek is deliberately
            // absent: it is HTTP with no filesystem access through the bridge, so it can only
            // review method-level questions, and it is consulted explicitly when that is wanted.
            reviewers: default_reviewers(),
            round_robin: Arc::new(Mutex::new(0)),
        })
    }

    /// The default reviewer panel. Exposed so callers can assert that every reviewer is
    /// actually dispatchable; a reviewer that is not routes review requests to a dead agent.
    pub fn reviewer_names(&self) -> Vec<String> {
        self.reviewers.clone()
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
    ) -> anyhow::Result<Option<String>> {
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

        let inflight: i64 = conn.query_row(
            "SELECT COUNT(*) FROM reviews WHERE state = 'in_progress'",
            [],
            |row| row.get(0),
        )?;
        if inflight >= max_inflight_limit() as i64 {
            return Ok(None);
        }

        let next_pending: Option<String> = conn
            .query_row(
                "SELECT review_id
                 FROM reviews
                 WHERE state = 'pending'
                 ORDER BY datetime(requested_at) ASC, rowid ASC
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(next_review_id) = next_pending {
            conn.execute(
                "UPDATE reviews
                 SET state = 'in_progress'
                 WHERE review_id = ?1 AND state = 'pending'",
                [next_review_id.as_str()],
            )?;
            return Ok(Some(next_review_id));
        }

        Ok(None)
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

    #[test]
    fn submit_review_promotes_oldest_pending_when_slot_frees() {
        let temp = tempfile::tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool")).expect("spool");
        let _store = LedgerStore::open(project_root.clone()).expect("open ledger");
        let engine = PeerReviewEngine::new(project_root).expect("engine");
        let first = engine
            .request_review(ReviewRequest {
                fleet_id: Some("fleet-1".to_string()),
                author_agent: "codex".to_string(),
                artifact: "diff-1".to_string(),
                review_type: "code".to_string(),
            })
            .expect("request first");
        let second = engine
            .request_review(ReviewRequest {
                fleet_id: Some("fleet-1".to_string()),
                author_agent: "codex".to_string(),
                artifact: "diff-2".to_string(),
                review_type: "code".to_string(),
            })
            .expect("request second");
        let third = engine
            .request_review(ReviewRequest {
                fleet_id: Some("fleet-1".to_string()),
                author_agent: "codex".to_string(),
                artifact: "diff-3".to_string(),
                review_type: "code".to_string(),
            })
            .expect("request third");

        let conn = engine.open_conn().expect("open conn");
        conn.execute(
            "UPDATE reviews SET state = 'pending', requested_at = datetime('now', '+1 second')
             WHERE review_id = ?1",
            [second.review_id.as_str()],
        )
        .expect("set second pending");
        conn.execute(
            "UPDATE reviews SET state = 'pending', requested_at = datetime('now', '+2 seconds')
             WHERE review_id = ?1",
            [third.review_id.as_str()],
        )
        .expect("set third pending");
        drop(conn);

        let promoted = engine
            .submit_review(&first.review_id, "approve", Some("ok"))
            .expect("submit first");
        assert_eq!(promoted.as_deref(), Some(second.review_id.as_str()));
        let second_after = engine
            .get_review(&second.review_id)
            .expect("get second")
            .expect("second present");
        assert_eq!(second_after.state, "in_progress");

        let promoted = engine
            .submit_review(&second.review_id, "approve", Some("ok"))
            .expect("submit second");
        assert_eq!(promoted.as_deref(), Some(third.review_id.as_str()));
        let third_after = engine
            .get_review(&third.review_id)
            .expect("get third")
            .expect("third present");
        assert_eq!(third_after.state, "in_progress");

    }
    /// grok is a DEFAULT reviewer, not opt-in. Asserted here so removing it is a decision.
    #[test]
    fn u_pr_grok_is_a_default_reviewer() {
        // Shares the lock with panel_roster_tests, which mutate TRIUMVIRATE_PEER_REVIEWERS.
        let _guard = crate::reviewer_env_guard();
        // SAFETY: held under that lock.
        unsafe { std::env::remove_var("TRIUMVIRATE_PEER_REVIEWERS") };
        let tmp = std::env::temp_dir().join("tv-peer-review-grok-test");
        std::fs::create_dir_all(&tmp).unwrap();
        let engine = PeerReviewEngine::new(tmp).unwrap();
        let r = engine.reviewers.clone();
        for expected in ["codex", "gemini", "grok", "claude"] {
            assert!(r.iter().any(|x| x == expected), "{expected} must be a default reviewer");
        }
        // "Every reviewer is dispatchable" is asserted in triumvirate's integration suite,
        // which can see both crates; peer-review does not depend on mcp-bridge.
    }

    /// An author must never review its own output, including grok.
    #[test]
    fn u_pr_grok_cannot_review_itself() {
        let tmp = std::env::temp_dir().join("tv-peer-review-grok-self");
        std::fs::create_dir_all(&tmp).unwrap();
        let engine = PeerReviewEngine::new(tmp).unwrap();
        // next_reviewer must be able to route around grok when grok is the author.
        let picked = engine.next_reviewer("grok").unwrap();
        assert_ne!(picked, "grok", "the author cannot be its own reviewer");
    }

}

#[cfg(test)]
mod panel_roster_tests {
    use super::*;

    use super::reviewer_env_guard as env_guard;

    /// The default panel is unchanged and still seats grok. That was an explicit ruling.
    /// RED IF: a reviewer is dropped from the default.
    #[test]
    fn the_default_panel_is_the_four_peers() {
        let _guard = env_guard();
        // SAFETY: single assertion on a process-global, removed immediately. Kept here rather
        // than spread across tests because a leaked value would silently change the default.
        unsafe { std::env::remove_var("TRIUMVIRATE_PEER_REVIEWERS") };
        assert_eq!(default_reviewers(), vec!["codex", "gemini", "grok", "claude"]);
    }

    /// FIND-GROK-04: dropping a reviewer must be an env change, not a patch to this file.
    ///
    /// An operator whose grok is slow, rate limited or logged out previously had to edit Rust
    /// to run a review without it, which in practice means turning the whole review off.
    ///
    /// RED IF: the env override stops being honoured.
    #[test]
    fn an_operator_can_drop_a_reviewer_without_a_code_change() {
        let _guard = env_guard();
        unsafe { std::env::set_var("TRIUMVIRATE_PEER_REVIEWERS", "codex, gemini") };
        let got = default_reviewers();
        unsafe { std::env::remove_var("TRIUMVIRATE_PEER_REVIEWERS") };
        assert_eq!(got, vec!["codex", "gemini"]);
        assert!(!got.contains(&"grok".to_string()));
    }

    /// An empty or whitespace-only override must NOT silently produce an empty panel, which
    /// would look exactly like a review that passed.
    /// RED IF: an empty list stops falling back to the default.
    #[test]
    fn an_empty_override_falls_back_rather_than_disabling_review() {
        let _guard = env_guard();
        unsafe { std::env::set_var("TRIUMVIRATE_PEER_REVIEWERS", "  , ,") };
        let got = default_reviewers();
        unsafe { std::env::remove_var("TRIUMVIRATE_PEER_REVIEWERS") };
        assert_eq!(
            got.len(),
            4,
            "an empty roster must fall back to the default, never disable the panel silently"
        );
    }
}
