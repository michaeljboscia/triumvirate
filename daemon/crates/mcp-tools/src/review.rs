use daemon_core::metrics::DaemonMetrics;
use peer_review::{PeerReviewEngine, ReviewRequest as PersistedReviewRequest, Submitter};
use shared_types::{
    ReviewRequestResponse, ReviewRequestTool, ReviewStatusRequest, ReviewStatusResponse,
    ReviewSubmitRequest,
};
use std::path::PathBuf;

pub fn review_request(req: ReviewRequestTool) -> Result<ReviewRequestResponse, String> {
    let project_root = req
        .project_root
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "failed to resolve project root".to_string())?;
    let engine = PeerReviewEngine::new(project_root)
        .map_err(|e| format!("review_request engine init failed: {e}"))?;
    // Captured before the request struct moves them, so the event can name who reviews whom.
    let author_agent = req.author_agent.clone();
    let review_type = req.review_type.clone();
    let result = engine.request_review(PersistedReviewRequest {
        fleet_id: req.fleet_id,
        author_agent: req.author_agent,
        artifact: req.artifact,
        review_type: req.review_type,
        // FIND-REVIEW-03: a review requested over MCP is client bookkeeping, not a gate.
        dispatch_owned: false,
    });
    // Emit BEFORE the `?`, on both outcomes: a review the engine failed to assign is a real
    // failure and would be invisible if we only reported successes (Antigravity's
    // survivorship catch). On failure the reviewer is unknown, so it reports "unassigned".
    mcp_bridge::posthog::record_review_requested(
        result.as_ref().ok().and_then(|r| r.reviewer_agent.as_deref()).unwrap_or("unassigned"),
        &author_agent,
        &review_type,
        result.is_ok(),
    );
    let record = result.map_err(|e| format!("review_request failed: {e}"))?;
    Ok(ReviewRequestResponse {
        review_id: record.review_id,
        reviewer_agent: record.reviewer_agent,
        state: record.state,
    })
}

pub fn review_submit(metrics: &DaemonMetrics, req: ReviewSubmitRequest) -> Result<String, String> {
    let project_root = req
        .project_root
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "failed to resolve project root".to_string())?;
    let engine = PeerReviewEngine::new(project_root)
        .map_err(|e| format!("review_submit engine init failed: {e}"))?;
    // FIND-REVIEW-01: this is the untrusted surface. Whatever the body claims is checked against
    // the reviewer the engine assigned; an unidentified caller submits as the empty agent, which
    // never matches, so it cannot approve.
    let claimed = req.reviewer_agent.as_deref().unwrap_or("");
    let result = engine.submit_review(
        &req.review_id,
        &req.verdict,
        req.comments.as_deref(),
        Submitter::Agent(claimed),
    );
    // Emit on both outcomes, before the `?`: a rejected submit is invisible if we only report
    // successes. The verdict reached only local Prometheus before this.
    mcp_bridge::posthog::record_review_verdict(&req.verdict, result.is_ok());
    let _ = result.map_err(|e| format!("review_submit failed: {e}"))?;
    metrics.reviews_total.inc();
    Ok("ok".to_string())
}

pub fn review_status(req: ReviewStatusRequest) -> Result<ReviewStatusResponse, String> {
    let project_root = req
        .project_root
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "failed to resolve project root".to_string())?;
    let engine = PeerReviewEngine::new(project_root)
        .map_err(|e| format!("review_status engine init failed: {e}"))?;
    let record = engine
        .get_review(&req.review_id)
        .map_err(|e| format!("review_status failed: {e}"))?
        .ok_or_else(|| format!("review not found: {}", req.review_id))?;
    Ok(ReviewStatusResponse {
        review_id: record.review_id,
        reviewer_agent: record.reviewer_agent,
        verdict: record.verdict,
        comments: record.comments,
        state: record.state,
    })
}

/// The peer-review rotation's seat gate (Seven Primes T-204, REQ-090, amendment A-12): a seat may sit in the rotation
/// only if `seat_gate` admits it on `PeerReviewRotation`. The class is internal and the origin is always
/// `Unattended` (a rotation pick is never a person choosing a reviewer); neither comes from a caller field.
pub fn reviewer_gate(seat: &str) -> bool {
    use mcp_bridge::seat_gate::{Origin, Surface, seat_gate};
    seat_gate(seat, Surface::PeerReviewRotation, Origin::Unattended, false).is_ok()
}

/// Install `reviewer_gate` as peer-review's process-wide gate. `triumvirate` `main()` calls this before any mode
/// starts; tests that reach `next_reviewer` call it explicitly. The first install wins.
pub fn install_reviewer_gate() {
    peer_review::set_reviewer_gate(Box::new(reviewer_gate));
}

#[cfg(test)]
mod reviewer_gate_tests {
    use super::*;

    /// The production gate on the real review path: every existing seat is admitted, the three new seats never are
    /// (A-2: PeerReviewRotation is deferred for them), so a roster of only new seats cannot assign anyone, and a mixed
    /// roster assigns the existing seat every time.
    /// RED IF: reviewer_gate stops asking seat_gate with PeerReviewRotation, or review_request can assign a new seat.
    #[test]
    fn review_request_assigns_only_gate_admitted_seats() {
        let _env = crate::PROCESS_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        install_reviewer_gate();
        for seat in ["codex", "gemini", "grok", "claude", "deepseek"] {
            assert!(reviewer_gate(seat), "{seat}");
        }
        for seat in ["kimi", "mistral", "qwen", "not-a-seat"] {
            assert!(!reviewer_gate(seat), "{seat}");
        }
        let root = tempfile::tempdir().expect("tempdir");
        let _ = ledger::LedgerStore::open(root.path().to_path_buf()).expect("ledger");
        let request = |author: &str| {
            review_request(ReviewRequestTool {
                project_root: Some(root.path().display().to_string()),
                fleet_id: None,
                author_agent: author.to_string(),
                artifact: "diff".to_string(),
                review_type: "code".to_string(),
            })
        };
        let prior = std::env::var_os("TRIUMVIRATE_PEER_REVIEWERS");
        // SAFETY: under PROCESS_ENV_LOCK; restored below before any assertion can panic past it.
        unsafe { std::env::set_var("TRIUMVIRATE_PEER_REVIEWERS", "kimi,qwen") };
        let only_new = request("codex");
        unsafe { std::env::set_var("TRIUMVIRATE_PEER_REVIEWERS", "kimi,codex,mistral") };
        let mixed: Vec<_> = (0..4).map(|_| request("claude")).collect();
        unsafe {
            match prior {
                Some(v) => std::env::set_var("TRIUMVIRATE_PEER_REVIEWERS", v),
                None => std::env::remove_var("TRIUMVIRATE_PEER_REVIEWERS"),
            }
        }
        let err = only_new.expect_err("no new seat may be assigned");
        assert!(
            err.contains("refused by the seat gate: kimi, qwen"),
            "{err}"
        );
        for r in mixed {
            assert_eq!(
                r.expect("assigned").reviewer_agent.as_deref(),
                Some("codex")
            );
        }
    }
}
