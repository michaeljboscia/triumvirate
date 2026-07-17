use daemon_core::metrics::DaemonMetrics;
use peer_review::{PeerReviewEngine, ReviewRequest as PersistedReviewRequest};
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
    let result = engine.submit_review(&req.review_id, &req.verdict, req.comments.as_deref());
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
