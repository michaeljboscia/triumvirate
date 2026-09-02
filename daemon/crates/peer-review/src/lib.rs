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
    /// FIND-REVIEW-03: true only for a review the in-process mandatory-review dispatch is
    /// conducting. Such a row is writable ONLY by `Submitter::Dispatch`.
    ///
    /// Codex found why the reviewer-name check alone is not a boundary: `review_request` returns
    /// the assigned reviewer to the caller, so an MCP client can name it back and its claim
    /// matches. A name from a request body is an assertion, not an identity. This flag is not
    /// settable from any request body, which is what makes it one.
    pub dispatch_owned: bool,
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
    pub dispatch_owned: bool,
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

/// Who is claiming to submit a verdict.
///
/// FIND-REVIEW-01. `submit_review` used to take `(review_id, verdict, comments)` and nothing
/// else, so it could not have been safe: the caller never said who was speaking. Any client that
/// could reach `review_submit` could land `approve` on a row that no reviewer had ever been
/// dispatched for, and the mandatory-review gate reads that row's state to decide whether the
/// turn ships.
///
/// This is a REQUIRED parameter rather than an `Option`, so adding a new submit path is a
/// decision someone makes in the type system instead of a default that silently authorises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submitter<'a> {
    /// The in-process mandatory-review dispatch, which just received and parsed this reviewer's
    /// own text. It is trusted because it holds the reviewer's answer, not because it says so:
    /// it is not reachable over MCP or HTTP.
    Dispatch,
    /// An external caller (MCP tool / HTTP body) claiming to speak for this agent. Checked
    /// against the reviewer the engine actually assigned.
    Agent(&'a str),
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
            "INSERT INTO reviews (review_id, fleet_id, author_agent, reviewer_agent, artifact, review_type, state, dispatch_owned)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                review_id,
                req.fleet_id,
                req.author_agent,
                reviewer,
                req.artifact,
                req.review_type,
                state,
                i64::from(req.dispatch_owned)
            ],
        )?;
        self.get_review(&review_id)?
            .ok_or_else(|| anyhow::anyhow!("review insert did not persist"))
    }

    /// Record a verdict against a review row.
    ///
    /// FIND-REVIEW-01: an `approve` is only accepted when the row is `in_progress` AND the
    /// submitter is the reviewer the engine assigned (or the in-process dispatch that holds that
    /// reviewer's parsed answer). Everything else is rejected, loudly.
    ///
    /// Why only `approve` carries the identity check: a forged `reject` or `concerns` cannot
    /// launder unreviewed work through the gate, it can only block or annotate, and blocking is
    /// the safe direction. The state guard still applies to every verdict, because a row nobody
    /// is working on has no verdict to give.
    pub fn submit_review(
        &self,
        review_id: &str,
        verdict: &str,
        comments: Option<&str>,
        submitter: Submitter<'_>,
    ) -> anyhow::Result<Option<String>> {
        // FIND-REVIEW-04. The verdict was never validated: whatever string arrived was written
        // to the row verbatim, and the identity check below only fired on an exact,
        // case-insensitive "approve".
        //
        // Grok found the consequence. A raw MCP body of "approve " or "approved" wrote
        // state='done' with NO identity check at all, because neither string equals "approve".
        // The row then reads as a completed review to anything that inspects the ledger.
        //
        // Normalising and rejecting anything unrecognised closes both halves: the identity check
        // can no longer be side-stepped by spelling, and the ledger cannot accumulate verdicts
        // that no reader knows how to interpret.
        let verdict = normalise_verdict(verdict)?;
        let verdict = verdict.as_str();

        let conn = self.open_conn()?;
        let existing = self
            .get_review(review_id)?
            .ok_or_else(|| anyhow::anyhow!("review not found: {review_id}"))?;

        if existing.state != "in_progress" {
            anyhow::bail!(
                "review {review_id} is '{}', not 'in_progress': a verdict can only be recorded \
                 against a review that is actually running",
                existing.state
            );
        }

        // FIND-REVIEW-03. A review the daemon is conducting is not writable from outside, for
        // ANY verdict. Refusing only `approve` here would still leave a denial of service: an
        // MCP client could land a `reject` on a live mandatory review and the dispatch's own
        // submit would then fail as "changed state", which reads like an infrastructure fault.
        if existing.dispatch_owned && !matches!(submitter, Submitter::Dispatch) {
            anyhow::bail!(
                "review {review_id} is being conducted by the daemon's own review dispatch and                  cannot be submitted by a client"
            );
        }

        if verdict == "approve" {
            let assigned = existing.reviewer_agent.as_deref().unwrap_or("");
            let authorised = match submitter {
                Submitter::Dispatch => true,
                Submitter::Agent(name) => {
                    !assigned.is_empty() && name.eq_ignore_ascii_case(assigned)
                }
            };
            if !authorised {
                let claimed = match submitter {
                    Submitter::Dispatch => "in-process dispatch".to_string(),
                    Submitter::Agent("") => "(no reviewer named)".to_string(),
                    Submitter::Agent(name) => name.to_string(),
                };
                anyhow::bail!(
                    "review {review_id} is assigned to '{assigned}', but the approve was \
                     submitted by '{claimed}'"
                );
            }
        }

        // Re-checked in the WHERE clause, not just above, so a concurrent submit cannot slip
        // between the read and the write. `open_conn` sets a busy timeout but not a transaction,
        // and the read above uses a DIFFERENT connection, so the window is real.
        //
        // NO TEST COVERS THIS CLAUSE, and that is stated rather than left to be assumed.
        // Antigravity checked it: removing `AND state = 'in_progress'` from this UPDATE leaves
        // the whole suite green, because every test that attempts an invalid transition is
        // stopped by the Rust guard above before the SQL ever runs. The only way to reach this
        // clause while passing that guard is a state change between the two, and no test
        // simulates one. A threaded test that tried would be racy, and a flaky test here is
        // worse than an honest comment: it teaches people to re-run until green.
        //
        // My commit message for 20be42a listed "SQL guard removed" as a mutation that killed
        // tests. It did not. What was actually run was BOTH guards removed together, which the
        // Rust guard accounts for. The claim was false and is corrected here.
        let updated = conn.execute(
            "UPDATE reviews
             SET verdict = ?2, comments = ?3, reviewed_at = datetime('now'), state = 'done'
             WHERE review_id = ?1 AND state = 'in_progress'",
            rusqlite::params![review_id, verdict, comments],
        )?;
        if updated == 0 {
            anyhow::bail!(
                "review {review_id} changed state while the verdict was being recorded"
            );
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
            "SELECT review_id, fleet_id, author_agent, reviewer_agent, verdict, comments, state,
                    COALESCE(dispatch_owned, 0)
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
                    dispatch_owned: row.get::<_, i64>(7)? != 0,
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

/// The only verdicts a review row may carry.
///
/// `indeterminate` is included because `classify_review_verdict` produces it for a reviewer that
/// answered unusably or did not answer at all, and that outcome has to be recordable: it is the
/// fail-closed verdict, and refusing to store it would turn a blocked turn into a submit error.
///
/// Trimmed and lowercased rather than compared loosely, so the stored string is canonical and
/// every later reader (`review_status`, telemetry, the dashboard) sees one spelling.
fn normalise_verdict(raw: &str) -> anyhow::Result<String> {
    let cleaned = raw.trim().to_ascii_lowercase();
    match cleaned.as_str() {
        "approve" | "concerns" | "reject" | "indeterminate" => Ok(cleaned),
        other => anyhow::bail!(
            "unknown verdict {other:?}: expected one of approve, concerns, reject, indeterminate"
        ),
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

    use super::{PeerReviewEngine, ReviewRequest, Submitter};

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
                    dispatch_owned: false,
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
                dispatch_owned: false,
            })
            .expect("request first");
        let second = engine
            .request_review(ReviewRequest {
                fleet_id: Some("fleet-1".to_string()),
                author_agent: "codex".to_string(),
                artifact: "diff-2".to_string(),
                review_type: "code".to_string(),
                dispatch_owned: false,
            })
            .expect("request second");
        let third = engine
            .request_review(ReviewRequest {
                fleet_id: Some("fleet-1".to_string()),
                author_agent: "codex".to_string(),
                artifact: "diff-3".to_string(),
                review_type: "code".to_string(),
                dispatch_owned: false,
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
            .submit_review(&first.review_id, "approve", Some("ok"), Submitter::Dispatch)
            .expect("submit first");
        assert_eq!(promoted.as_deref(), Some(second.review_id.as_str()));
        let second_after = engine
            .get_review(&second.review_id)
            .expect("get second")
            .expect("second present");
        assert_eq!(second_after.state, "in_progress");

        let promoted = engine
            .submit_review(&second.review_id, "approve", Some("ok"), Submitter::Dispatch)
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

/// FIND-REVIEW-01: `review_submit` cannot approve a review nobody performed.
///
/// The bug these cover: `submit_review` took `(review_id, verdict, comments)` and its UPDATE had
/// no state guard and no notion of who was calling, so `SET state = 'done'` ran against any row
/// in any state for any caller. `enforce_mandatory_peer_review` then reads that row and ships the
/// turn if it says done+approve. That is the rubber stamp rebuilt one layer down: the dispatch
/// was made real in `092f90b`, but the record it writes to could still be written by anyone.
///
/// Every test here was made to fail on purpose before it was kept.
#[cfg(test)]
mod submit_authority_tests {
    use std::fs;

    use ledger::LedgerStore;

    use super::{PeerReviewEngine, ReviewRequest, Submitter};

    /// A project with a real ledger, one review row, and the reviewer the engine picked.
    ///
    /// Returns `(engine, review_id, assigned_reviewer)`. Each call gets its own tempdir, which is
    /// leaked deliberately: the engine reopens the sqlite file by path on every call, so dropping
    /// the `TempDir` mid-test would delete the database out from under it.
    pub(super) fn seeded(author: &str) -> (PeerReviewEngine, String, String) {
        seeded_with_owner(author, false)
    }

    pub(super) fn seeded_dispatch(author: &str) -> (PeerReviewEngine, String, String) {
        seeded_with_owner(author, true)
    }

    fn seeded_with_owner(author: &str, dispatch_owned: bool) -> (PeerReviewEngine, String, String) {
        let temp = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
        let project_root = temp.path().join("project");
        fs::create_dir_all(project_root.join(".triumvirate").join("spool")).expect("spool");
        let _store = LedgerStore::open(project_root.clone()).expect("open ledger");
        let engine = PeerReviewEngine::new(project_root).expect("engine");
        let review = engine
            .request_review(ReviewRequest {
                fleet_id: None,
                author_agent: author.to_string(),
                artifact: "the work under review".to_string(),
                review_type: "agent_output".to_string(),
                dispatch_owned,
            })
            .expect("request review");
        let reviewer = review.reviewer_agent.clone().expect("reviewer assigned");
        (engine, review.review_id, reviewer)
    }

    fn force_state(engine: &PeerReviewEngine, review_id: &str, state: &str) {
        let conn = engine.open_conn().expect("open conn");
        conn.execute(
            "UPDATE reviews SET state = ?2 WHERE review_id = ?1",
            rusqlite::params![review_id, state],
        )
        .expect("force state");
    }

    /// The headline case. A row parked in `pending` has had no reviewer dispatched for it, so
    /// there is no verdict in existence to record. Approving it is forging a review.
    /// RED IF: the state guard is dropped from `submit_review`.
    #[test]
    fn u_pr_approve_on_a_pending_row_is_refused() {
        let (engine, review_id, reviewer) = seeded("codex");
        force_state(&engine, &review_id, "pending");

        let err = engine
            .submit_review(&review_id, "approve", Some("lgtm"), Submitter::Agent(&reviewer))
            .expect_err("a pending review must not be approvable");
        assert!(
            err.to_string().contains("not 'in_progress'"),
            "the error must name the state problem, got: {err}"
        );

        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(after.state, "pending", "the row must be untouched");
        assert_eq!(after.verdict, None, "no verdict may be recorded");
    }

    /// The in-process dispatch is trusted about IDENTITY, not about STATE. It cannot approve a
    /// row that is not running either, because if the row is not `in_progress` then the dispatch
    /// is out of step with the engine and the safe reading of that is "stop".
    /// RED IF: `Submitter::Dispatch` is made to bypass the state check.
    #[test]
    fn u_pr_even_the_dispatch_cannot_approve_a_pending_row() {
        let (engine, review_id, _reviewer) = seeded("codex");
        force_state(&engine, &review_id, "pending");

        let err = engine
            .submit_review(&review_id, "approve", Some("lgtm"), Submitter::Dispatch)
            .expect_err("dispatch must not approve a parked review either");
        assert!(err.to_string().contains("not 'in_progress'"), "got: {err}");
    }

    /// A `failed` row is one `fail_timed_out_reviews` already gave up on. Approving it after the
    /// fact resurrects a review that timed out.
    /// RED IF: the guard only checks for `pending` instead of requiring `in_progress`.
    #[test]
    fn u_pr_approve_on_a_failed_row_is_refused() {
        let (engine, review_id, reviewer) = seeded("codex");
        force_state(&engine, &review_id, "failed");

        let err = engine
            .submit_review(&review_id, "approve", Some("lgtm"), Submitter::Agent(&reviewer))
            .expect_err("a failed review must not be approvable");
        assert!(err.to_string().contains("not 'in_progress'"), "got: {err}");
    }

    /// Submitting twice is the same forgery with extra steps: the second submit lands on a row
    /// that is already `done`.
    /// RED IF: the terminal state stops being terminal.
    #[test]
    fn u_pr_a_review_cannot_be_approved_twice() {
        let (engine, review_id, reviewer) = seeded("codex");
        engine
            .submit_review(&review_id, "reject", Some("no"), Submitter::Agent(&reviewer))
            .expect("first submit");

        let err = engine
            .submit_review(&review_id, "approve", Some("actually fine"), Submitter::Dispatch)
            .expect_err("a completed review must not be re-verdicted");
        assert!(err.to_string().contains("not 'in_progress'"), "got: {err}");

        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(
            after.verdict.as_deref(),
            Some("reject"),
            "the original verdict must survive the overwrite attempt"
        );
    }

    /// Identity. The row is running, but the caller is not the agent the engine assigned.
    /// This is the forge an MCP or HTTP client can attempt: it knows the review_id (review_status
    /// hands it out) and simply claims to be someone.
    /// RED IF: the reviewer comparison is removed or made permissive.
    #[test]
    fn u_pr_approve_by_an_agent_that_is_not_the_reviewer_is_refused() {
        let (engine, review_id, reviewer) = seeded("codex");
        let impostor = if reviewer == "grok" { "gemini" } else { "grok" };
        assert_ne!(impostor, reviewer);

        let err = engine
            .submit_review(&review_id, "approve", Some("lgtm"), Submitter::Agent(impostor))
            .expect_err("only the assigned reviewer may approve");
        assert!(
            err.to_string().contains("assigned to"),
            "the error must name the assignment mismatch, got: {err}"
        );

        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(after.state, "in_progress");
        assert_eq!(after.verdict, None);
    }

    /// An MCP body that names no reviewer at all reaches the engine as the empty agent. It must
    /// not match, and in particular must not match a row whose reviewer_agent is somehow null.
    /// RED IF: the empty-string case is allowed to compare equal.
    #[test]
    fn u_pr_an_unidentified_caller_cannot_approve() {
        let (engine, review_id, _reviewer) = seeded("codex");

        let err = engine
            .submit_review(&review_id, "approve", None, Submitter::Agent(""))
            .expect_err("an unidentified submitter must not approve");
        assert!(err.to_string().contains("assigned to"), "got: {err}");
    }

    /// The positive control. Without this the whole gate could be passing by refusing everything,
    /// which is a different way of being useless.
    /// RED IF: a legitimate approve stops working.
    #[test]
    fn u_pr_the_assigned_reviewer_can_approve() {
        let (engine, review_id, reviewer) = seeded("codex");

        engine
            .submit_review(&review_id, "approve", Some("read it, it holds"), Submitter::Agent(&reviewer))
            .expect("the assigned reviewer may approve");

        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(after.state, "done");
        assert_eq!(after.verdict.as_deref(), Some("approve"));
    }

    /// The name comparison must not be case sensitive: the roster is lowercased at parse time but
    /// a client may well send "Codex".
    /// RED IF: the comparison becomes `==`.
    #[test]
    fn u_pr_the_reviewer_name_match_ignores_case() {
        let (engine, review_id, reviewer) = seeded("codex");
        let shouted = reviewer.to_ascii_uppercase();

        engine
            .submit_review(&review_id, "approve", None, Submitter::Agent(&shouted))
            .expect("case must not decide authority");
        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(after.verdict.as_deref(), Some("approve"));
    }

    /// The in-process dispatch holds the reviewer's own parsed text, so it may submit on that
    /// reviewer's behalf. This is Goal 1.1's second required test: the gate must still be able to
    /// record a real verdict, or mandatory review can never complete at all.
    /// RED IF: `Submitter::Dispatch` loses its authority.
    #[test]
    fn u_pr_the_dispatch_can_submit_a_real_verdict() {
        let (engine, review_id, _reviewer) = seeded("codex");

        engine
            .submit_review(&review_id, "approve", Some("APPROVE\nreasoning"), Submitter::Dispatch)
            .expect("the dispatch may record the verdict it just parsed");

        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(after.state, "done");
        assert_eq!(after.verdict.as_deref(), Some("approve"));
    }

    /// Identity is only checked on `approve`. A forged REJECT cannot launder unreviewed work
    /// through the gate; it can only block, and blocking is the safe direction. Requiring
    /// identity here would let an unroutable client turn a block into a submit error, which the
    /// caller might then treat as an infrastructure fault rather than a verdict.
    /// RED IF: the identity check is widened to all verdicts without deciding to.
    #[test]
    fn u_pr_a_reject_does_not_require_the_assigned_identity() {
        let (engine, review_id, reviewer) = seeded("codex");
        let other = if reviewer == "grok" { "gemini" } else { "grok" };

        engine
            .submit_review(&review_id, "reject", Some("this claim is false"), Submitter::Agent(other))
            .expect("a reject may come from anyone: it can only block");

        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(after.verdict.as_deref(), Some("reject"));
    }

    /// "APPROVE" with decoration is still an approve as far as authority goes. The verdict string
    /// reaching the engine is already normalised by `classify_review_verdict`, but a raw MCP
    /// client can send anything, and `eq_ignore_ascii_case` is the only thing standing between
    /// "Approve" and an unchecked write.
    /// RED IF: the approve detection becomes an exact `== "approve"`.
    #[test]
    fn u_pr_a_capitalised_approve_is_still_an_approve() {
        let (engine, review_id, _reviewer) = seeded("codex");

        let err = engine
            .submit_review(&review_id, "APPROVE", None, Submitter::Agent("nobody"))
            .expect_err("case must not be a way around the authority check");
        assert!(err.to_string().contains("assigned to"), "got: {err}");
    }
}

/// FIND-REVIEW-03: a review the daemon is conducting is not writable by a client.
///
/// Codex raised this against the first version of FIND-REVIEW-01: the reviewer-name check was
/// not an authentication boundary, because `review_request` returns the assigned reviewer to the
/// caller, so an MCP client can simply name it back and match. A name in a request body is an
/// assertion about identity, not identity.
///
/// The boundary that can actually be enforced is ownership, not naming: the mandatory-review
/// dispatch marks its own rows, no request field deserialises into that flag, and such a row
/// accepts writes from `Submitter::Dispatch` only.
#[cfg(test)]
mod dispatch_ownership_tests {
    use super::submit_authority_tests::{seeded, seeded_dispatch};
    use super::Submitter;

    /// The exact attack Codex described: the client knows the review_id and names the correct
    /// reviewer, because `review_request` told it both.
    /// RED IF: the ownership check is removed, or narrowed back to a name comparison.
    #[test]
    fn u_pr_naming_the_right_reviewer_does_not_authorise_a_client() {
        let (engine, review_id, reviewer) = seeded_dispatch("codex");

        let err = engine
            .submit_review(&review_id, "approve", Some("lgtm"), Submitter::Agent(&reviewer))
            .expect_err("knowing the reviewer's name is not being the reviewer");
        assert!(
            err.to_string().contains("conducted by the daemon"),
            "the refusal must be about ownership, not naming; got: {err}"
        );

        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(after.state, "in_progress");
        assert_eq!(after.verdict, None);
    }

    /// Refusing only `approve` would leave a denial of service: a client lands a `reject` on a
    /// live mandatory review, and the dispatch's own submit then fails as "changed state", which
    /// reads like an infrastructure fault rather than an attack.
    /// RED IF: the ownership check is narrowed to approve only.
    #[test]
    fn u_pr_a_client_cannot_reject_a_dispatch_owned_review_either() {
        let (engine, review_id, reviewer) = seeded_dispatch("codex");

        let err = engine
            .submit_review(&review_id, "reject", Some("no"), Submitter::Agent(&reviewer))
            .expect_err("a client must not be able to derail a live review");
        assert!(err.to_string().contains("conducted by the daemon"), "got: {err}");
        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(after.verdict, None);
    }

    /// The dispatch itself must still be able to finish its own review, or mandatory review can
    /// never complete.
    /// RED IF: the ownership check starts refusing Dispatch.
    #[test]
    fn u_pr_the_dispatch_can_still_write_its_own_review() {
        let (engine, review_id, _reviewer) = seeded_dispatch("codex");
        engine
            .submit_review(&review_id, "approve", Some("read it"), Submitter::Dispatch)
            .expect("the dispatch owns this row");
        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(after.verdict.as_deref(), Some("approve"));
    }

    /// Client-requested reviews are unaffected. They are bookkeeping, not a gate: fleet queues
    /// them and an agent picks them up over MCP. Locking them would break that workflow to
    /// defend a boundary they were never part of.
    /// RED IF: ownership starts defaulting to true, which would silently disable review_submit.
    #[test]
    fn u_pr_a_client_requested_review_stays_client_writable() {
        let (engine, review_id, reviewer) = seeded("codex");
        assert!(
            !engine.get_review(&review_id).expect("get").expect("present").dispatch_owned,
            "a review requested through the ordinary path must not be dispatch owned"
        );
        engine
            .submit_review(&review_id, "approve", None, Submitter::Agent(&reviewer))
            .expect("the assigned reviewer may still submit a client review");
    }

    /// The flag has to survive the round trip through sqlite, including on a database created
    /// before the column existed, where COALESCE supplies the default.
    /// RED IF: the INSERT stops writing the column, or the SELECT stops reading it.
    #[test]
    fn u_pr_ownership_round_trips_through_the_database() {
        let (engine, owned, _) = seeded_dispatch("codex");
        let (other_engine, unowned, _) = seeded("codex");
        assert!(engine.get_review(&owned).expect("get").expect("present").dispatch_owned);
        assert!(!other_engine.get_review(&unowned).expect("get").expect("present").dispatch_owned);
    }
}

/// FIND-REVIEW-04: the verdict string is validated, not stored verbatim.
///
/// Grok found this reviewing FIND-REVIEW-01. The identity check fired only on an exact,
/// case-insensitive `"approve"`, and the engine wrote whatever string it was handed. So a raw
/// MCP body of `"approve "` or `"approved"` skipped the identity check entirely and still set
/// `state = 'done'`. The row then reads as a completed review to anything inspecting the ledger.
#[cfg(test)]
mod verdict_normalisation_tests {
    use super::submit_authority_tests::seeded;
    use super::Submitter;

    /// The exact strings Grok named, plus the empty and whitespace cases. Each one used to
    /// write `done` with no identity check, because none of them equals "approve".
    /// RED IF: `normalise_verdict` starts accepting anything outside the four.
    #[test]
    fn u_pr_an_unknown_verdict_leaves_the_row_alone() {
        for sneaky in ["approved", "approve!", "ok", "", "  "] {
            let (engine, review_id, _) = seeded("codex");
            let err = engine
                .submit_review(&review_id, sneaky, None, Submitter::Agent("nobody"))
                .expect_err(&format!("{sneaky:?} must not be accepted as a verdict"));
            assert!(
                err.to_string().contains("unknown verdict"),
                "the refusal must name the verdict problem for {sneaky:?}; got: {err}"
            );
            let after = engine.get_review(&review_id).expect("get").expect("present");
            assert_eq!(after.state, "in_progress", "{sneaky:?} must not complete the review");
            assert_eq!(after.verdict, None, "{sneaky:?} must not be stored");
        }
    }

    /// Whitespace and case must not be a way past the identity check. `" APPROVE "` normalises
    /// to `approve`, so it takes the authority path rather than sliding by as an unknown string.
    /// RED IF: normalisation stops trimming, or the identity compare stops using the normalised
    /// value.
    #[test]
    fn u_pr_a_padded_approve_still_takes_the_identity_path() {
        let (engine, review_id, _) = seeded("codex");
        let err = engine
            .submit_review(&review_id, "  APPROVE  ", None, Submitter::Agent("nobody"))
            .expect_err("a padded approve is still an approve");
        assert!(
            err.to_string().contains("assigned to"),
            "it must be refused for identity, not as an unknown verdict; got: {err}"
        );
    }

    /// The stored string is canonical, so every later reader sees one spelling.
    /// RED IF: the raw string is written instead of the normalised one.
    #[test]
    fn u_pr_the_stored_verdict_is_canonical() {
        let (engine, review_id, reviewer) = seeded("codex");
        engine
            .submit_review(&review_id, " Approve ", None, Submitter::Agent(&reviewer))
            .expect("the assigned reviewer may approve");
        let after = engine.get_review(&review_id).expect("get").expect("present");
        assert_eq!(after.verdict.as_deref(), Some("approve"), "not \" Approve \"");
    }

    /// `indeterminate` must be storable. It is what `classify_review_verdict` produces for a
    /// reviewer that answered unusably or not at all, and refusing it would turn a blocked turn
    /// into a submit error, which reads as infrastructure rather than as a verdict.
    /// RED IF: indeterminate is dropped from the accepted set.
    #[test]
    fn u_pr_the_fail_closed_verdict_is_recordable() {
        for v in ["indeterminate", "reject", "concerns"] {
            let (engine, review_id, _) = seeded("codex");
            engine
                .submit_review(&review_id, v, Some("why"), Submitter::Dispatch)
                .unwrap_or_else(|e| panic!("{v} must be recordable: {e}"));
            let after = engine.get_review(&review_id).expect("get").expect("present");
            assert_eq!(after.verdict.as_deref(), Some(v));
        }
    }
}
