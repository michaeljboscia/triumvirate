use std::collections::HashMap;

use anyhow::Context;
use rusqlite::params;

use crate::{TokenDb, TokenRecord};

const UNATTRIBUTED_BUILD_ID: &str = "unattributed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxAttributionEvent {
    pub session_id: String,
    pub build_id: String,
    pub task_id: Option<String>,
    pub wave: Option<i64>,
}

pub fn attribute_records(
    db: &TokenDb,
    records: Vec<TokenRecord>,
    outbox_events: &[OutboxAttributionEvent],
) -> anyhow::Result<Vec<TokenRecord>> {
    let by_session: HashMap<&str, &OutboxAttributionEvent> = outbox_events
        .iter()
        .map(|event| (event.session_id.as_str(), event))
        .collect();

    records
        .into_iter()
        .map(|mut record| {
            if let Some(event) = by_session.get(record.session_id.as_str()) {
                record.build_id = Some(event.build_id.clone());
                record.task_id = event.task_id.clone();
                record.wave = event.wave;
            } else {
                record.build_id = Some(UNATTRIBUTED_BUILD_ID.to_string());
                record.task_id = None;
                record.wave = None;
            }

            record.cost_usd = calculate_cost_usd(db, &record)?;
            Ok(record)
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// T-003 (REQ-DS-009): seed the DeepSeek price rows in production.
//
// The existing INSERT pattern in this file (~line 156) is inside `#[cfg(test)]` and
// is therefore test-only. Production needs an explicit seed mechanism that runs at
// daemon startup. ensure_deepseek_prices is idempotent (only inserts if a row for
// the given model doesn't already exist), so it's safe to call on every daemon boot.
//
// Prices (per million tokens) — locked in by the spec's verification pass on
// 2026-05-25, with v4-pro's 75% promo becoming permanent 2026-05-31 15:59 UTC.
//
//   model              input-miss    input-hit (cached)    output
//   deepseek-v4-pro    $0.435        $0.003625             $0.87
//   deepseek-v4-flash  $0.14         $0.0028               $0.28
//
// The DeepSeek runner's usage mapping (REQ-DS-009 / A-04b):
//   output_tokens ← completion_tokens (already INCLUDES reasoning_tokens — no double-add)
//   input_tokens  ← prompt_cache_miss_tokens
//   cached_tokens ← prompt_cache_hit_tokens
// ─────────────────────────────────────────────────────────────────────────────

pub fn ensure_deepseek_prices(db: &TokenDb) -> anyhow::Result<()> {
    ensure_price_row(db, "deepseek-v4-pro", 0.435, 0.87, 0.003625)?;
    ensure_price_row(db, "deepseek-v4-flash", 0.14, 0.28, 0.0028)?;
    Ok(())
}

/// Idempotent: inserts the price row for `model` only if no row for that model
/// already exists in the table. (We could add a UNIQUE constraint and use
/// INSERT OR IGNORE, but that would be a schema change; the SELECT-then-INSERT
/// check is sufficient for the seed-once-at-startup case.)
fn ensure_price_row(
    db: &TokenDb,
    model: &str,
    input_per_mtok: f64,
    output_per_mtok: f64,
    cached_per_mtok: f64,
) -> anyhow::Result<()> {
    let conn = db
        .conn
        .lock()
        .map_err(|_| anyhow::anyhow!("token DB connection mutex poisoned"))?;

    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM price_table WHERE model = ?1)",
            params![model],
            |row| row.get(0),
        )
        .with_context(|| format!("failed to check existence of price row for model '{model}'"))?;

    if exists {
        return Ok(());
    }

    conn.execute(
        r#"
        INSERT INTO price_table (
            model,
            input_per_mtok,
            output_per_mtok,
            cached_per_mtok,
            effective_date,
            end_date
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            model,
            input_per_mtok,
            output_per_mtok,
            cached_per_mtok,
            "2026-01-01T00:00:00Z",
            Option::<String>::None,
        ],
    )
    .with_context(|| format!("failed to insert price row for model '{model}'"))?;

    Ok(())
}

// T-003 (REQ-DS-009 / REQ-DS-021): pub so the DeepSeek runner can compute the per-consult
// cost synchronously after the usage chunk arrives, instead of waiting for the deferred
// attribution sweep. Returns None for unmetered records and for any model with no
// price_table row at the record's timestamp.
pub fn calculate_cost_usd(db: &TokenDb, record: &TokenRecord) -> anyhow::Result<Option<f64>> {
    // REQ-057: unmetered rows (agy) have no honest token count → never cost them.
    if record.usage_source == crate::USAGE_SOURCE_UNMETERED {
        return Ok(None);
    }
    let Some(model) = record.model.as_deref() else {
        return Ok(None);
    };

    let Some((input_per_mtok, output_per_mtok, cached_per_mtok)) =
        query_active_price_components(db, model, &record.timestamp)?
    else {
        return Ok(None);
    };

    let cost = ((record.input_tokens as f64 * input_per_mtok)
        + (record.output_tokens as f64 * output_per_mtok)
        + (record.cached_tokens as f64 * cached_per_mtok))
        / 1_000_000.0;

    Ok(Some(cost))
}

fn query_active_price_components(
    db: &TokenDb,
    model: &str,
    timestamp: &str,
) -> anyhow::Result<Option<(f64, f64, f64)>> {
    let conn = db
        .conn
        .lock()
        .map_err(|_| anyhow::anyhow!("token DB connection mutex poisoned"))?;

    let mut stmt = conn
        .prepare(
            r#"
            SELECT
                input_per_mtok,
                output_per_mtok,
                cached_per_mtok
            FROM price_table
            WHERE model = ?1
                AND effective_date <= ?2
                AND (end_date IS NULL OR end_date > ?2)
            ORDER BY effective_date DESC
            LIMIT 1
            "#,
        )
        .with_context(|| {
            format!(
                "failed to prepare price lookup query for model '{}' at timestamp '{}'",
                model, timestamp
            )
        })?;

    let mut rows = stmt
        .query(params![model, timestamp])
        .with_context(|| format!("failed to execute price lookup for model '{}'", model))?;

    let Some(row) = rows.next()? else {
        return Ok(None);
    };

    Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?)))
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    use crate::{TokenRecord, open};

    use super::{OutboxAttributionEvent, attribute_records};

    fn sample_record(session_id: &str, model: &str) -> TokenRecord {
        TokenRecord {
            agent: "codex".to_string(),
            session_id: session_id.to_string(),
            timestamp: "2026-04-10T10:00:00Z".to_string(),
            model: Some(model.to_string()),
            input_tokens: 1_000,
            output_tokens: 500,
            cached_tokens: 250,
            thinking_tokens: 0,
            total_tokens: 1_750,
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
        }
    }

    fn seed_price(
        db: &crate::TokenDb,
        model: &str,
        input_per_mtok: f64,
        output_per_mtok: f64,
        cached_per_mtok: f64,
    ) {
        let conn = db.conn.lock().expect("lock db");
        conn.execute(
            r#"
            INSERT INTO price_table (
                model,
                input_per_mtok,
                output_per_mtok,
                cached_per_mtok,
                effective_date,
                end_date
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![
                model,
                input_per_mtok,
                output_per_mtok,
                cached_per_mtok,
                "2026-01-01T00:00:00Z",
                Option::<String>::None
            ],
        )
        .expect("insert price point");
    }

    #[test]
    fn attribute_records_matches_session_ids_and_calculates_cost() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open token db");

        seed_price(&db, "gpt-5.3-codex", 10.0, 20.0, 5.0);

        let input = vec![sample_record("sess-1", "gpt-5.3-codex")];
        let outbox = vec![OutboxAttributionEvent {
            session_id: "sess-1".to_string(),
            build_id: "abe-v3-main".to_string(),
            task_id: Some("T-113".to_string()),
            wave: Some(3),
        }];

        let attributed = attribute_records(&db, input, &outbox).expect("attribute records");
        assert_eq!(attributed.len(), 1);
        let record = &attributed[0];

        assert_eq!(record.build_id.as_deref(), Some("abe-v3-main"));
        assert_eq!(record.task_id.as_deref(), Some("T-113"));
        assert_eq!(record.wave, Some(3));

        let expected_cost = ((1_000.0 * 10.0) + (500.0 * 20.0) + (250.0 * 5.0)) / 1_000_000.0;
        let actual_cost = record.cost_usd.expect("cost should be present");
        assert!((actual_cost - expected_cost).abs() < 1e-12);
    }

    #[test]
    fn unmatched_sessions_go_to_unattributed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open token db");

        seed_price(&db, "gpt-5.3-codex", 1.0, 1.0, 0.0);

        let input = vec![sample_record("sess-unmatched", "gpt-5.3-codex")];
        let outbox = vec![OutboxAttributionEvent {
            session_id: "sess-different".to_string(),
            build_id: "abe-v3-main".to_string(),
            task_id: Some("T-999".to_string()),
            wave: Some(9),
        }];

        let attributed = attribute_records(&db, input, &outbox).expect("attribute records");
        let record = &attributed[0];
        assert_eq!(record.build_id.as_deref(), Some("unattributed"));
        assert_eq!(record.task_id, None);
        assert_eq!(record.wave, None);
        assert!(record.cost_usd.is_some());
    }

    #[test]
    fn price_table_temporal_lookup_uses_active_price_for_timestamp() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open token db");
        {
            let conn = db.conn.lock().expect("lock db");
            conn.execute(
                r#"
                INSERT INTO price_table (
                    model,
                    input_per_mtok,
                    output_per_mtok,
                    cached_per_mtok,
                    effective_date,
                    end_date
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    "gpt-5.3-codex",
                    10.0_f64,
                    20.0_f64,
                    0.0_f64,
                    "2026-01-01T00:00:00Z",
                    Some("2026-06-01T00:00:00Z")
                ],
            )
            .expect("insert first price point");
            conn.execute(
                r#"
                INSERT INTO price_table (
                    model,
                    input_per_mtok,
                    output_per_mtok,
                    cached_per_mtok,
                    effective_date,
                    end_date
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
                params![
                    "gpt-5.3-codex",
                    30.0_f64,
                    40.0_f64,
                    0.0_f64,
                    "2026-06-01T00:00:00Z",
                    Option::<String>::None
                ],
            )
            .expect("insert second price point");
        }

        let mut record = sample_record("sess-temporal", "gpt-5.3-codex");
        record.timestamp = "2026-06-15T10:00:00Z".to_string();
        record.input_tokens = 1_000;
        record.output_tokens = 500;
        record.cached_tokens = 0;

        let attributed = attribute_records(&db, vec![record], &[]).expect("attribute records");
        let cost = attributed[0].cost_usd.expect("cost should be present");

        let expected = ((1_000.0 * 30.0) + (500.0 * 40.0)) / 1_000_000.0;
        assert!((cost - expected).abs() < 1e-12);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // T-003 (REQ-DS-009): ensure_deepseek_prices seeds production rows.
    //
    // Stub guard: a no-op helper (one that returns Ok(()) without inserting)
    // would fail the "both rows present" assertion. A wrong-numbers helper
    // would fail the calculate_cost_usd math.
    // ─────────────────────────────────────────────────────────────────────────
    use super::{calculate_cost_usd, ensure_deepseek_prices};

    fn row_count_for_model(db: &crate::TokenDb, model: &str) -> i64 {
        let conn = db.conn.lock().expect("lock db");
        conn.query_row(
            "SELECT COUNT(*) FROM price_table WHERE model = ?1",
            params![model],
            |row| row.get(0),
        )
        .expect("count rows")
    }

    fn record_for_cost(model: &str, input: i64, output: i64, cached: i64) -> TokenRecord {
        let mut r = sample_record("sess-ds-cost", model);
        r.timestamp = "2026-06-01T00:00:00Z".to_string(); // after effective_date
        r.input_tokens = input;
        r.output_tokens = output;
        r.cached_tokens = cached;
        r
    }

    #[test]
    fn ensure_deepseek_prices_seeds_both_v4_pro_and_v4_flash() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open token db");

        // Fresh DB: no price rows exist for either model.
        assert_eq!(row_count_for_model(&db, "deepseek-v4-pro"), 0);
        assert_eq!(row_count_for_model(&db, "deepseek-v4-flash"), 0);

        ensure_deepseek_prices(&db).expect("seed runs");

        assert_eq!(row_count_for_model(&db, "deepseek-v4-pro"), 1);
        assert_eq!(row_count_for_model(&db, "deepseek-v4-flash"), 1);
    }

    #[test]
    fn ensure_deepseek_prices_is_idempotent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open token db");

        for _ in 0..5 {
            ensure_deepseek_prices(&db).expect("seed runs repeatedly without error");
        }
        // Still exactly 1 row per model — not 5, not 10.
        assert_eq!(row_count_for_model(&db, "deepseek-v4-pro"), 1);
        assert_eq!(row_count_for_model(&db, "deepseek-v4-flash"), 1);
    }

    #[test]
    fn calculate_cost_usd_matches_spec_pricing_v4_pro() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open token db");
        ensure_deepseek_prices(&db).expect("seed");

        // 1M input-miss tokens + 1M output tokens, no cached → 0.435 + 0.87 = $1.305
        let r = record_for_cost("deepseek-v4-pro", 1_000_000, 1_000_000, 0);
        let cost = calculate_cost_usd(&db, &r).expect("cost calc").expect("cost is Some");
        assert!(
            (cost - 1.305).abs() < 1e-9,
            "v4-pro cost expected ~$1.305, got ${cost}"
        );

        // 1M cached + 1M output, no miss → 0.003625 + 0.87 = $0.873625
        let r = record_for_cost("deepseek-v4-pro", 0, 1_000_000, 1_000_000);
        let cost = calculate_cost_usd(&db, &r).expect("cost calc").expect("cost is Some");
        assert!(
            (cost - 0.873625).abs() < 1e-9,
            "v4-pro cached-only cost expected ~$0.873625, got ${cost}"
        );
    }

    #[test]
    fn calculate_cost_usd_matches_spec_pricing_v4_flash() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open token db");
        ensure_deepseek_prices(&db).expect("seed");

        // 1M input-miss + 1M output → 0.14 + 0.28 = $0.42
        let r = record_for_cost("deepseek-v4-flash", 1_000_000, 1_000_000, 0);
        let cost = calculate_cost_usd(&db, &r).expect("cost calc").expect("cost is Some");
        assert!(
            (cost - 0.42).abs() < 1e-9,
            "v4-flash cost expected ~$0.42, got ${cost}"
        );

        // Cache discount: 1M cached + 1M output → 0.0028 + 0.28 = $0.2828
        let r = record_for_cost("deepseek-v4-flash", 0, 1_000_000, 1_000_000);
        let cost = calculate_cost_usd(&db, &r).expect("cost calc").expect("cost is Some");
        assert!(
            (cost - 0.2828).abs() < 1e-9,
            "v4-flash cached-only cost expected ~$0.2828, got ${cost}"
        );
    }

    // Realistic small-call check anchored to the live PROBE-04 numbers from Wave 0:
    //   prompt_cache_miss=18, completion (incl reasoning)=174, cached=0, v4-pro.
    //   cost = 174e-6 * 0.87 + 18e-6 * 0.435 ≈ $0.0001592
    // A stub that adds reasoning_tokens to output (the A-04b anti-pattern) would
    // overstate cost and fail. A stub returning a fixed value also fails.
    #[test]
    fn calculate_cost_usd_matches_wave0_probe_04_numbers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open token db");
        ensure_deepseek_prices(&db).expect("seed");

        let r = record_for_cost("deepseek-v4-pro", 18, 174, 0);
        let cost = calculate_cost_usd(&db, &r).expect("cost calc").expect("cost is Some");

        let expected = (174.0 * 0.87 + 18.0 * 0.435) / 1_000_000.0;
        assert!(
            (cost - expected).abs() < 1e-12,
            "expected ~${expected}, got ${cost}"
        );
        // Sanity: well under a cent for a single small consult.
        assert!(cost < 0.001, "small consult cost should be sub-cent; got ${cost}");
    }
}
