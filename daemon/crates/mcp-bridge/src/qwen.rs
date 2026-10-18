//! Qwen invocation builder (Seven Primes REQ-023). BACKEND_STRUCTURE.md section 6 holds the argv contract.
//!
//! T-003 scaffold: every public function returns `Err("not implemented: T-103")` until T-103 replaces it. The public
//! signatures and the shared types are final; Wave 1 tasks change bodies, not shapes.

use crate::containment::{SeatInvocation, SeatRequest};

/// The read-only argv for one dispatch. Errors carry a code first, for example `prompt_too_large_for_qwen`.
pub fn build_qwen_invocation(req: &SeatRequest<'_>) -> Result<SeatInvocation, String> {
    let _ = req;
    Err("not implemented: T-103".to_string())
}

/// The served-model check (REQ-043): the Qwen seat must be answered by a Qwen model, else
/// `qwen_seat_served_non_qwen_model: <model>`.
pub fn check_served_model(model: &str) -> Result<(), String> {
    let _ = model;
    Err("not implemented: T-103".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Duration;

    /// RED IF: a stub stops being wired to its public signature, or is mistaken for an implementation.
    #[test]
    fn t003_stubs_are_wired_and_refuse() {
        let p = Path::new("/nonexistent");
        let req = SeatRequest {
            prompt: "p",
            cwd: p,
            seat_home: p,
            output_dir: p,
            model: None,
            resume_session_id: None,
            extra_args: &[],
            max_turns: 1,
            time_limit: Duration::from_secs(1),
        };
        assert_eq!(
            build_qwen_invocation(&req).unwrap_err(),
            "not implemented: T-103"
        );
        assert_eq!(
            check_served_model("qwen3-coder-plus").unwrap_err(),
            "not implemented: T-103"
        );
    }
}
