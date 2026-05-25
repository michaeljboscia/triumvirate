use anyhow::{Context, ensure};

use crate::{TokenDb, TokenRecord, storage};

/// Direct-write fast path for daemon-mediated sessions.
///
/// This path bypasses scanning because the daemon has exact session/task context at call time.
pub fn record_daemon_tokens(db: &TokenDb, record: TokenRecord) -> anyhow::Result<()> {
    validate_record(&record)?;
    storage::insert_record(db, &record).with_context(|| {
        format!(
            "failed to persist daemon token record for agent={} session_id={}",
            record.agent, record.session_id
        )
    })
}

fn validate_record(record: &TokenRecord) -> anyhow::Result<()> {
    ensure!(
        !record.agent.trim().is_empty(),
        "token record agent must be non-empty"
    );
    ensure!(
        !record.session_id.trim().is_empty(),
        "token record session_id must be non-empty"
    );
    ensure!(
        !record.timestamp.trim().is_empty(),
        "token record timestamp must be non-empty"
    );
    ensure!(record.input_tokens >= 0, "input_tokens must be non-negative");
    ensure!(record.output_tokens >= 0, "output_tokens must be non-negative");
    ensure!(record.cached_tokens >= 0, "cached_tokens must be non-negative");
    ensure!(
        record.thinking_tokens >= 0,
        "thinking_tokens must be non-negative"
    );
    ensure!(record.total_tokens >= 0, "total_tokens must be non-negative");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{TokenRecord, open, query_summary};

    use super::record_daemon_tokens;

    #[test]
    fn record_daemon_tokens_writes_record() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("token-economics.db");
        let db = open(&db_path).expect("open db");

        let record = TokenRecord {
            agent: "gemini".to_string(),
            session_id: "session-direct-1".to_string(),
            timestamp: "2026-04-10T12:00:00Z".to_string(),
            model: Some("gemini-2.5-pro".to_string()),
            input_tokens: 100,
            output_tokens: 50,
            cached_tokens: 10,
            thinking_tokens: 0,
            total_tokens: 160,
            cost_usd: None,
            latency_ms: None,
            tool_calls: Some(1),
            lines_added: None,
            lines_removed: None,
            rate_limit_pct: None,
            context_window: None,
            build_id: Some("abe-v3-main".to_string()),
            task_id: Some("T-114".to_string()),
            wave: Some(3),
            usage_source: "exact".to_string(),
        };

        record_daemon_tokens(&db, record).expect("record daemon tokens");
        let rows = query_summary(&db, None, None, Some("gemini")).expect("query summary");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, "session-direct-1");
        assert_eq!(rows[0].task_id.as_deref(), Some("T-114"));
    }

    #[test]
    fn record_daemon_tokens_rejects_empty_session_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("token-economics.db");
        let db = open(&db_path).expect("open db");

        let record = TokenRecord {
            agent: "codex".to_string(),
            session_id: "".to_string(),
            timestamp: "2026-04-10T12:00:00Z".to_string(),
            model: Some("gpt-5.3-codex".to_string()),
            input_tokens: 1,
            output_tokens: 2,
            cached_tokens: 0,
            thinking_tokens: 0,
            total_tokens: 3,
            cost_usd: None,
            latency_ms: None,
            tool_calls: None,
            lines_added: None,
            lines_removed: None,
            rate_limit_pct: None,
            context_window: None,
            build_id: None,
            task_id: None,
            wave: None,
            usage_source: "exact".to_string(),
        };

        let err = record_daemon_tokens(&db, record).expect_err("should reject empty session_id");
        assert!(err.to_string().contains("session_id"));
    }
}
