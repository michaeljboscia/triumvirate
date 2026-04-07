//! Peer review orchestration primitives.

use shared_types::Lesson;

#[derive(Debug, Clone)]
pub struct PeerReviewEngine {
    pub max_inflight: usize,
}

impl PeerReviewEngine {
    pub fn new(max_inflight: usize) -> Self {
        Self { max_inflight }
    }

    pub fn is_reviewer_allowed(&self, author: &str, reviewer: &str) -> bool {
        !author.eq_ignore_ascii_case(reviewer)
    }

    pub fn summarize_lessons(&self, lessons: &[Lesson]) -> usize {
        lessons.len()
    }
}

#[cfg(test)]
mod tests {
    use super::PeerReviewEngine;

    #[test]
    fn rejects_self_review() {
        let engine = PeerReviewEngine::new(2);
        assert!(!engine.is_reviewer_allowed("codex", "codex"));
        assert!(engine.is_reviewer_allowed("codex", "gemini"));
    }
}
