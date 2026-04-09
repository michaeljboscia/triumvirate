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

    let response = ask_agent_executor(AskAgentRequest {
            agent: "gemini".to_string(),
            message: prompt,
            cwd: None,
            repo: None,
            branch: None,
        })
        .await
        .map_err(|e| format!("query_gemini_review failed: {e}"))?;
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
