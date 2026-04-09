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
    let record = engine
        .request_review(PersistedReviewRequest {
            fleet_id: req.fleet_id,
            author_agent: req.author_agent,
            artifact: req.artifact,
            review_type: req.review_type,
        })
        .map_err(|e| format!("review_request failed: {e}"))?;
    Ok(ReviewRequestResponse {
        review_id: record.review_id,
        reviewer_agent: record.reviewer_agent,
        state: record.state,
    })
}

pub fn review_submit(req: ReviewSubmitRequest) -> Result<String, String> {
    let project_root = req
        .project_root
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "failed to resolve project root".to_string())?;
    let engine = PeerReviewEngine::new(project_root)
        .map_err(|e| format!("review_submit engine init failed: {e}"))?;
    let _ = engine
        .submit_review(&req.review_id, &req.verdict, req.comments.as_deref())
        .map_err(|e| format!("review_submit failed: {e}"))?;
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
