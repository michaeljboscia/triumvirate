use std::{
    collections::{BTreeMap, BTreeSet},
    sync::MutexGuard,
};

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{TokenDb, TokenSummaryRow, query_summary};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryQueryFilters {
    pub since: Option<String>,
    pub until: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeRange {
    pub first_timestamp: String,
    pub last_timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentTokenSummary {
    pub agent: String,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
    pub record_count: i64,
    pub session_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuildTaskCost {
    pub task_id: Option<String>,
    pub wave: Option<i64>,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
    pub record_count: i64,
    pub session_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BuildCostBreakdown {
    pub build_id: String,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
    pub record_count: i64,
    pub session_count: i64,
    pub time_range: Option<TimeRange>,
    pub tasks: Vec<BuildTaskCost>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTokenBreakdown {
    pub session_id: String,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
    pub record_count: i64,
    pub time_range: Option<TimeRange>,
    pub agents: Vec<AgentTokenSummary>,
    pub build_ids: Vec<String>,
    pub task_ids: Vec<String>,
    pub records: Vec<TokenSummaryRow>,
}

#[derive(Default)]
struct SummaryAccumulator {
    total_tokens: i64,
    total_cost_usd: f64,
    record_count: i64,
    sessions: BTreeSet<String>,
}

pub fn summary_query(db: &TokenDb, filters: &SummaryQueryFilters) -> anyhow::Result<serde_json::Value> {
    let rows = query_summary(
        db,
        filters.since.as_deref(),
        filters.until.as_deref(),
        filters.agent.as_deref(),
    )?;

    let mut per_agent: BTreeMap<String, SummaryAccumulator> = BTreeMap::new();
    let mut total_tokens: i64 = 0;
    let mut total_cost_usd: f64 = 0.0;
    let mut first_timestamp: Option<String> = None;
    let mut last_timestamp: Option<String> = None;

    for row in &rows {
        total_tokens += row.total_tokens;
        total_cost_usd += row.cost_usd.unwrap_or(0.0);
        first_timestamp = min_timestamp(first_timestamp, &row.timestamp);
        last_timestamp = max_timestamp(last_timestamp, &row.timestamp);

        let entry = per_agent.entry(row.agent.clone()).or_default();
        entry.total_tokens += row.total_tokens;
        entry.total_cost_usd += row.cost_usd.unwrap_or(0.0);
        entry.record_count += 1;
        entry.sessions.insert(row.session_id.clone());
    }

    let agents: Vec<AgentTokenSummary> = per_agent
        .into_iter()
        .map(|(agent, acc)| AgentTokenSummary {
            agent,
            total_tokens: acc.total_tokens,
            total_cost_usd: acc.total_cost_usd,
            record_count: acc.record_count,
            session_count: acc.sessions.len() as i64,
        })
        .collect();

    let time_range = match (first_timestamp, last_timestamp) {
        (Some(first_timestamp), Some(last_timestamp)) => Some(TimeRange {
            first_timestamp,
            last_timestamp,
        }),
        _ => None,
    };

    Ok(serde_json::json!({
        "filters": filters,
        "time_range": time_range,
        "record_count": rows.len(),
        "total_tokens": total_tokens,
        "total_cost_usd": total_cost_usd,
        "agents": agents
    }))
}

pub fn by_build_query(db: &TokenDb, build_id: &str) -> anyhow::Result<BuildCostBreakdown> {
    let conn = lock_conn(db)?;

    let (total_tokens, total_cost_usd, record_count, session_count) = conn.query_row(
        r#"
        SELECT
            COALESCE(SUM(total_tokens), 0),
            COALESCE(SUM(cost_usd), 0.0),
            COUNT(*),
            COUNT(DISTINCT session_id)
        FROM token_records
        WHERE build_id = ?1
        "#,
        params![build_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;

    let mut stmt = conn.prepare(
        r#"
        SELECT
            task_id,
            wave,
            COALESCE(SUM(total_tokens), 0),
            COALESCE(SUM(cost_usd), 0.0),
            COUNT(*),
            COUNT(DISTINCT session_id)
        FROM token_records
        WHERE build_id = ?1
        GROUP BY task_id, wave
        ORDER BY wave ASC, task_id ASC
        "#,
    )?;
    let task_rows = stmt.query_map(params![build_id], |row| {
        Ok(BuildTaskCost {
            task_id: row.get(0)?,
            wave: row.get(1)?,
            total_tokens: row.get(2)?,
            total_cost_usd: row.get(3)?,
            record_count: row.get(4)?,
            session_count: row.get(5)?,
        })
    })?;

    let mut tasks = Vec::new();
    for row in task_rows {
        tasks.push(row?);
    }

    let time_range = query_time_range_for_build(&conn, build_id)?;

    Ok(BuildCostBreakdown {
        build_id: build_id.to_string(),
        total_tokens,
        total_cost_usd,
        record_count,
        session_count,
        time_range,
        tasks,
    })
}

pub fn by_session_query(db: &TokenDb, session_id: &str) -> anyhow::Result<SessionTokenBreakdown> {
    let conn = lock_conn(db)?;
    let mut stmt = conn.prepare(
        r#"
        SELECT
            id,
            agent,
            session_id,
            timestamp,
            model,
            input_tokens,
            output_tokens,
            cached_tokens,
            thinking_tokens,
            total_tokens,
            cost_usd,
            latency_ms,
            tool_calls,
            lines_added,
            lines_removed,
            rate_limit_pct,
            context_window,
            build_id,
            task_id,
            wave
        FROM token_records
        WHERE session_id = ?1
        ORDER BY timestamp ASC, id ASC
        "#,
    )?;

    let rows_iter = stmt.query_map(params![session_id], |row| {
        Ok(TokenSummaryRow {
            id: row.get(0)?,
            agent: row.get(1)?,
            session_id: row.get(2)?,
            timestamp: row.get(3)?,
            model: row.get(4)?,
            input_tokens: row.get(5)?,
            output_tokens: row.get(6)?,
            cached_tokens: row.get(7)?,
            thinking_tokens: row.get(8)?,
            total_tokens: row.get(9)?,
            cost_usd: row.get(10)?,
            latency_ms: row.get(11)?,
            tool_calls: row.get(12)?,
            lines_added: row.get(13)?,
            lines_removed: row.get(14)?,
            rate_limit_pct: row.get(15)?,
            context_window: row.get(16)?,
            build_id: row.get(17)?,
            task_id: row.get(18)?,
            wave: row.get(19)?,
        })
    })?;

    let mut rows = Vec::new();
    for row in rows_iter {
        rows.push(row?);
    }

    let mut per_agent: BTreeMap<String, SummaryAccumulator> = BTreeMap::new();
    let mut total_tokens: i64 = 0;
    let mut total_cost_usd = 0.0;
    let mut first_timestamp: Option<String> = None;
    let mut last_timestamp: Option<String> = None;
    let mut build_ids: BTreeSet<String> = BTreeSet::new();
    let mut task_ids: BTreeSet<String> = BTreeSet::new();

    for row in &rows {
        total_tokens += row.total_tokens;
        total_cost_usd += row.cost_usd.unwrap_or(0.0);
        first_timestamp = min_timestamp(first_timestamp, &row.timestamp);
        last_timestamp = max_timestamp(last_timestamp, &row.timestamp);
        if let Some(build_id) = &row.build_id {
            build_ids.insert(build_id.clone());
        }
        if let Some(task_id) = &row.task_id {
            task_ids.insert(task_id.clone());
        }

        let entry = per_agent.entry(row.agent.clone()).or_default();
        entry.total_tokens += row.total_tokens;
        entry.total_cost_usd += row.cost_usd.unwrap_or(0.0);
        entry.record_count += 1;
        entry.sessions.insert(row.session_id.clone());
    }

    let agents = per_agent
        .into_iter()
        .map(|(agent, acc)| AgentTokenSummary {
            agent,
            total_tokens: acc.total_tokens,
            total_cost_usd: acc.total_cost_usd,
            record_count: acc.record_count,
            session_count: acc.sessions.len() as i64,
        })
        .collect();

    let time_range = match (first_timestamp, last_timestamp) {
        (Some(first_timestamp), Some(last_timestamp)) => Some(TimeRange {
            first_timestamp,
            last_timestamp,
        }),
        _ => None,
    };

    Ok(SessionTokenBreakdown {
        session_id: session_id.to_string(),
        total_tokens,
        total_cost_usd,
        record_count: rows.len() as i64,
        time_range,
        agents,
        build_ids: build_ids.into_iter().collect(),
        task_ids: task_ids.into_iter().collect(),
        records: rows,
    })
}

fn query_time_range_for_build(
    conn: &rusqlite::Connection,
    build_id: &str,
) -> anyhow::Result<Option<TimeRange>> {
    conn.query_row(
        r#"
        SELECT
            MIN(timestamp),
            MAX(timestamp)
        FROM token_records
        WHERE build_id = ?1
        "#,
        params![build_id],
        |row| {
            let first_timestamp: Option<String> = row.get(0)?;
            let last_timestamp: Option<String> = row.get(1)?;
            Ok(match (first_timestamp, last_timestamp) {
                (Some(first_timestamp), Some(last_timestamp)) => Some(TimeRange {
                    first_timestamp,
                    last_timestamp,
                }),
                _ => None,
            })
        },
    )
    .map_err(Into::into)
}

fn lock_conn(db: &TokenDb) -> anyhow::Result<MutexGuard<'_, rusqlite::Connection>> {
    db.conn
        .lock()
        .map_err(|_| anyhow::anyhow!("token DB connection mutex poisoned"))
}

fn min_timestamp(current: Option<String>, candidate: &str) -> Option<String> {
    match current {
        Some(value) if value.as_str() <= candidate => Some(value),
        _ => Some(candidate.to_string()),
    }
}

fn max_timestamp(current: Option<String>, candidate: &str) -> Option<String> {
    match current {
        Some(value) if value.as_str() >= candidate => Some(value),
        _ => Some(candidate.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use crate::{TokenRecord, insert_record, open};

    use super::{SummaryQueryFilters, by_build_query, by_session_query, summary_query};

    fn sample_record(agent: &str, session_id: &str, ts: &str) -> TokenRecord {
        TokenRecord {
            agent: agent.to_string(),
            session_id: session_id.to_string(),
            timestamp: ts.to_string(),
            model: Some("model-a".to_string()),
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 1,
            thinking_tokens: 0,
            total_tokens: 16,
            cost_usd: Some(0.01),
            latency_ms: None,
            tool_calls: None,
            lines_added: None,
            lines_removed: None,
            rate_limit_pct: None,
            context_window: None,
            build_id: Some("abe-v3-main".to_string()),
            task_id: Some("T-115".to_string()),
            wave: Some(4),
        }
    }

    #[test]
    fn summary_query_returns_totals_and_agent_breakdown() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open db");
        insert_record(&db, &sample_record("codex", "sess-1", "2026-04-10T01:00:00Z")).expect("insert row 1");
        insert_record(&db, &sample_record("gemini", "sess-2", "2026-04-10T02:00:00Z")).expect("insert row 2");

        let payload = summary_query(&db, &SummaryQueryFilters::default()).expect("summary query");
        assert_eq!(payload["record_count"], serde_json::json!(2));
        assert_eq!(payload["total_tokens"], serde_json::json!(32));
        assert_eq!(payload["agents"].as_array().map(|v| v.len()), Some(2));
    }

    #[test]
    fn by_build_query_returns_task_level_breakdown() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open db");
        insert_record(&db, &sample_record("codex", "sess-1", "2026-04-10T01:00:00Z")).expect("insert row 1");
        let mut second = sample_record("codex", "sess-2", "2026-04-10T02:00:00Z");
        second.task_id = Some("T-116".to_string());
        insert_record(&db, &second).expect("insert row 2");

        let breakdown = by_build_query(&db, "abe-v3-main").expect("build query");
        assert_eq!(breakdown.build_id, "abe-v3-main");
        assert_eq!(breakdown.record_count, 2);
        assert_eq!(breakdown.tasks.len(), 2);
    }

    #[test]
    fn by_session_query_returns_records_for_session() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open db");
        insert_record(&db, &sample_record("codex", "sess-target", "2026-04-10T01:00:00Z"))
            .expect("insert row 1");
        insert_record(&db, &sample_record("gemini", "sess-other", "2026-04-10T02:00:00Z"))
            .expect("insert row 2");

        let breakdown = by_session_query(&db, "sess-target").expect("session query");
        assert_eq!(breakdown.session_id, "sess-target");
        assert_eq!(breakdown.record_count, 1);
        assert_eq!(breakdown.total_tokens, 16);
        assert_eq!(breakdown.records.len(), 1);
    }

    #[test]
    fn empty_db_queries_return_empty_not_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open db");

        let summary = summary_query(&db, &SummaryQueryFilters::default()).expect("summary query");
        assert_eq!(summary["record_count"], serde_json::json!(0));
        assert_eq!(summary["total_tokens"], serde_json::json!(0));
        assert_eq!(summary["agents"], serde_json::json!([]));

        let build = by_build_query(&db, "missing-build").expect("build query");
        assert_eq!(build.build_id, "missing-build");
        assert_eq!(build.record_count, 0);
        assert_eq!(build.total_tokens, 0);
        assert!(build.tasks.is_empty());
        assert_eq!(build.time_range, None);

        let session = by_session_query(&db, "missing-session").expect("session query");
        assert_eq!(session.session_id, "missing-session");
        assert_eq!(session.record_count, 0);
        assert_eq!(session.total_tokens, 0);
        assert!(session.records.is_empty());
        assert!(session.agents.is_empty());
        assert!(session.build_ids.is_empty());
        assert!(session.task_ids.is_empty());
        assert_eq!(session.time_range, None);
    }
}
