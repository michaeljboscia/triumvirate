use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use token_economics::{TokenSummaryRow, open, query_summary};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GetTokenSummaryRequest {
    pub since: Option<String>,
    pub until: Option<String>,
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct TokenTotals {
    pub session_count: usize,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub thinking_tokens: i64,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct TokenAgentSummary {
    pub agent: String,
    pub session_count: usize,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub thinking_tokens: i64,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GetTokenSummaryResponse {
    pub since: Option<String>,
    pub until: Option<String>,
    pub agent: Option<String>,
    pub totals: TokenTotals,
    pub by_agent: Vec<TokenAgentSummary>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GetBuildCostRequest {
    pub build_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct BuildTaskCost {
    pub task_id: String,
    pub wave: Option<i64>,
    pub session_count: usize,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cached_tokens: i64,
    pub thinking_tokens: i64,
    pub total_tokens: i64,
    pub total_cost_usd: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GetBuildCostResponse {
    pub build_id: String,
    pub total_cost_usd: f64,
    pub total_tokens: i64,
    pub tasks: Vec<BuildTaskCost>,
}

#[derive(Default)]
struct Aggregate {
    sessions: HashSet<String>,
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
    thinking_tokens: i64,
    total_tokens: i64,
    total_cost_usd: f64,
}

impl Aggregate {
    fn add_row(&mut self, session_id: &str, row: &TokenSummaryRow) {
        self.sessions.insert(session_id.to_string());
        self.input_tokens += row.input_tokens;
        self.output_tokens += row.output_tokens;
        self.cached_tokens += row.cached_tokens;
        self.thinking_tokens += row.thinking_tokens;
        self.total_tokens += row.total_tokens;
        self.total_cost_usd += row.cost_usd.unwrap_or(0.0);
    }

    fn to_totals(&self) -> TokenTotals {
        TokenTotals {
            session_count: self.sessions.len(),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cached_tokens: self.cached_tokens,
            thinking_tokens: self.thinking_tokens,
            total_tokens: self.total_tokens,
            total_cost_usd: self.total_cost_usd,
        }
    }
}

pub fn get_token_summary(
    db_path: &Path,
    req: GetTokenSummaryRequest,
) -> Result<GetTokenSummaryResponse, String> {
    let token_db = open(db_path).map_err(|e| format!("get_token_summary db open failed: {e}"))?;
    let rows = query_summary(
        &token_db,
        req.since.as_deref(),
        req.until.as_deref(),
        req.agent.as_deref(),
    )
    .map_err(|e| format!("get_token_summary query failed: {e}"))?;

    let mut total = Aggregate::default();
    let mut by_agent: BTreeMap<String, Aggregate> = BTreeMap::new();

    for row in rows {
        total.add_row(&row.session_id, &row);
        by_agent
            .entry(row.agent.clone())
            .or_default()
            .add_row(&row.session_id, &row);
    }

    let by_agent = by_agent
        .into_iter()
        .map(|(agent, aggregate)| TokenAgentSummary {
            agent,
            session_count: aggregate.sessions.len(),
            input_tokens: aggregate.input_tokens,
            output_tokens: aggregate.output_tokens,
            cached_tokens: aggregate.cached_tokens,
            thinking_tokens: aggregate.thinking_tokens,
            total_tokens: aggregate.total_tokens,
            total_cost_usd: aggregate.total_cost_usd,
        })
        .collect::<Vec<_>>();

    Ok(GetTokenSummaryResponse {
        since: req.since,
        until: req.until,
        agent: req.agent,
        totals: total.to_totals(),
        by_agent,
    })
}

#[derive(Default)]
struct BuildTaskAggregate {
    wave: Option<i64>,
    metrics: Aggregate,
}

pub fn get_build_cost(db_path: &Path, req: GetBuildCostRequest) -> Result<GetBuildCostResponse, String> {
    if req.build_id.trim().is_empty() {
        return Err("get_build_cost requires a non-empty build_id".to_string());
    }

    let token_db = open(db_path).map_err(|e| format!("get_build_cost db open failed: {e}"))?;
    let rows = query_summary(&token_db, None, None, None)
        .map_err(|e| format!("get_build_cost query failed: {e}"))?;

    let mut by_task: BTreeMap<String, BuildTaskAggregate> = BTreeMap::new();
    let mut total_cost_usd = 0.0;
    let mut total_tokens = 0i64;

    for row in rows {
        if row.build_id.as_deref() != Some(req.build_id.as_str()) {
            continue;
        }

        total_cost_usd += row.cost_usd.unwrap_or(0.0);
        total_tokens += row.total_tokens;

        let task_key = row
            .task_id
            .clone()
            .unwrap_or_else(|| "unattributed".to_string());
        let entry = by_task.entry(task_key).or_default();
        if entry.wave.is_none() {
            entry.wave = row.wave;
        }
        entry.metrics.add_row(&row.session_id, &row);
    }

    let mut tasks = by_task
        .into_iter()
        .map(|(task_id, aggregate)| BuildTaskCost {
            task_id,
            wave: aggregate.wave,
            session_count: aggregate.metrics.sessions.len(),
            input_tokens: aggregate.metrics.input_tokens,
            output_tokens: aggregate.metrics.output_tokens,
            cached_tokens: aggregate.metrics.cached_tokens,
            thinking_tokens: aggregate.metrics.thinking_tokens,
            total_tokens: aggregate.metrics.total_tokens,
            total_cost_usd: aggregate.metrics.total_cost_usd,
        })
        .collect::<Vec<_>>();

    tasks.sort_by(|a, b| {
        b.total_cost_usd
            .partial_cmp(&a.total_cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.task_id.cmp(&b.task_id))
    });

    Ok(GetBuildCostResponse {
        build_id: req.build_id,
        total_cost_usd,
        total_tokens,
        tasks,
    })
}
