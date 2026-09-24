use shared_types::{
    AskAgentRequest, AskAgentResponse, GeminiReviewVerdict, QueryGeminiRequest,
    QueryGeminiResponse, QueryGeminiReviewRequest, QueryGeminiReviewResponse,
};
use std::future::Future;

pub async fn query_gemini<F, Fut>(
    req: QueryGeminiRequest,
    ask_agent_executor: F,
) -> Result<QueryGeminiResponse, String>
where
    F: Fn(AskAgentRequest) -> Fut,
    Fut: Future<Output = Result<AskAgentResponse, String>>,
{
    let query = if let Some(ctx) = req.context {
        format!("Context:\n{ctx}\n\nQuestion:\n{}", req.query)
    } else {
        req.query
    };
    let response = ask_agent_executor(AskAgentRequest {
            agent: "gemini".to_string(),
            message: query,
            cwd: None,
            repo: None,
            branch: None,
            ..Default::default()
        })
        .await
        .map_err(|e| format!("query_gemini failed: {e}"))?;
    Ok(QueryGeminiResponse {
        response: response.response,
    })
}

pub async fn query_gemini_review<F, Fut>(
    req: QueryGeminiReviewRequest,
    ask_agent_executor: F,
) -> Result<QueryGeminiReviewResponse, String>
where
    F: Fn(AskAgentRequest) -> Fut,
    Fut: Future<Output = Result<AskAgentResponse, String>>,
{
    let mut prompt = format!(
        "Review this diff and provide verdict clean/concerns/regression.\n\n{}",
        req.diff
    );
    if matches!(req.mode, shared_types::GeminiReviewMode::Failure) {
        if let Some(briefing) = req.briefing {
            prompt.push_str(&format!("\n\nBriefing:\n{briefing}"));
        }
        if let Some(contract) = req.contract {
            let serialized = serde_json::to_string_pretty(&contract)
                .map_err(|e| format!("contract serialization failed: {e}"))?;
            prompt.push_str(&format!("\n\nContract:\n{serialized}"));
        }
        if let Some(details) = req.failure_details {
            prompt.push_str(&format!("\n\nFailure details:\n{details}"));
        }
    }

    // D-030: this call used to leave `strict_agent` unset. With substitution opted in, an agy
    // failure meant CODEX answered, and the code below then threw away `answered_by_agent` and
    // keyword-scanned the text: a codex "Approved." came back as a CLEAN GEMINI REVIEW, in a
    // response type with no field able to say otherwise. A verdict attributed to a seat that
    // did not produce it is worse than no verdict, so this one is strict.
    let response = ask_agent_executor(AskAgentRequest {
            agent: "gemini".to_string(),
            message: prompt,
            cwd: None,
            repo: None,
            branch: None,
            strict_agent: Some(true),
            ..Default::default()
        })
        .await
        .map_err(|e| format!("query_gemini_review failed: {e}"))?;
    // Belt and braces: strict_agent should make this impossible, but this function's return
    // type cannot express "someone else answered", so it must not return at all if one did.
    if let Some(other) = response.answered_by_agent.as_deref()
        && !other.eq_ignore_ascii_case("gemini")
        && !other.eq_ignore_ascii_case("antigravity")
    {
        return Err(format!(
            "query_gemini_review was answered by `{other}`, not the gemini seat. A review verdict \
             carries the authority of the reviewer that produced it, and this response type \
             cannot record that the reviewer was substituted."
        ));
    }
    let lower = response.response.to_lowercase();
    let verdict = if lower.contains("regression") {
        GeminiReviewVerdict::Regression
    } else if lower.contains("concern") || lower.contains("issue") {
        GeminiReviewVerdict::Concerns
    } else {
        GeminiReviewVerdict::Clean
    };
    Ok(QueryGeminiReviewResponse {
        verdict,
        concerns: None,
        suggestions: None,
    })
}
