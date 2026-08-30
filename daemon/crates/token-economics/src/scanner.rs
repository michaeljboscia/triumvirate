use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::Path,
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde_json::{Map, Value};

use crate::{TokenDb, TokenRecord};

#[derive(Debug, Clone, Copy, Default)]
struct TokenFields {
    input_tokens: i64,
    output_tokens: i64,
    cached_tokens: i64,
    thinking_tokens: i64,
    total_tokens: i64,
    latency_ms: Option<i64>,
    tool_calls: Option<i64>,
    lines_added: Option<i64>,
    lines_removed: Option<i64>,
    rate_limit_pct: Option<f64>,
    context_window: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct ScanState {
    last_mtime: i64,
    last_offset: i64,
}


// ─────────────────────────────────────────────────────────────────────────────
// Grok has NO offline scanner, deliberately. REQ-GROK-012.
//
// The other agents leave token counts in files on disk, so `scan_codex_file` and friends can
// reconstruct spend after the fact. Grok does not: `~/.grok/sessions/<cwd>/prompt_history.jsonl`
// contains only `{timestamp, session_id, prompt, is_bash}`, with no usage block at all. Verified
// against a real profile on 2026-08-30.
//
// Grok reports usage and its own `total_cost_usd` LIVE, in the `end` event of every turn, which
// the parser captures. So grok spend is recorded through `direct::record_daemon_tokens` from the
// runner, where the numbers actually exist, rather than scraped afterwards from a file that never
// had them.
//
// If a future grok version starts writing usage to disk, add a scanner then. Adding one now would
// produce records with zeroed token counts, which is worse than no records because it looks like
// measured spend.
// ─────────────────────────────────────────────────────────────────────────────

pub fn scan_claude_file(db: &TokenDb, file_path: &Path) -> Result<Vec<TokenRecord>> {
    scan_jsonl_file(db, file_path, "claude")
}

pub fn scan_codex_file(db: &TokenDb, file_path: &Path) -> Result<Vec<TokenRecord>> {
    scan_jsonl_file(db, file_path, "codex")
}

pub fn scan_gemini_chat_file(db: &TokenDb, file_path: &Path) -> Result<Vec<TokenRecord>> {
    let conn = lock_conn(db)?;
    let mtime = file_mtime(file_path)?;

    if !should_scan_by_mtime(&conn, file_path, mtime)? {
        return Ok(Vec::new());
    }

    let value: Value = serde_json::from_reader(
        File::open(file_path)
            .with_context(|| format!("failed to open gemini chat file {}", file_path.display()))?,
    )
    .with_context(|| format!("failed to parse gemini chat JSON {}", file_path.display()))?;

    let mut records = Vec::new();
    let mut last_session_id: Option<String> = None;
    let mut last_model: Option<String> = None;
    collect_records_from_value(
        &value,
        "gemini",
        &mut last_session_id,
        &mut last_model,
        &mut records,
    );

    update_scan_state(&conn, file_path, mtime, 0)?;
    Ok(records)
}

pub fn scan_gemini_telemetry_file(db: &TokenDb, file_path: &Path) -> Result<Vec<TokenRecord>> {
    let conn = lock_conn(db)?;
    let mtime = file_mtime(file_path)?;
    let file = File::open(file_path)
        .with_context(|| format!("failed to open gemini telemetry file {}", file_path.display()))?;
    let file_len = file
        .metadata()
        .with_context(|| format!("failed to read metadata for {}", file_path.display()))?
        .len() as i64;

    let state = get_scan_state(&conn, file_path)?;
    let start_offset = match state {
        Some(state) if file_len >= state.last_offset => state.last_offset,
        Some(_) => 0, // file rotated/truncated
        None => 0,
    };

    if let Some(state) = state
        && mtime <= state.last_mtime
        && file_len <= state.last_offset
    {
        return Ok(Vec::new());
    }

    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(start_offset as u64))
        .with_context(|| format!("failed to seek telemetry file {}", file_path.display()))?;

    let mut records = Vec::new();
    let mut last_session_id: Option<String> = None;
    let mut last_model: Option<String> = None;
    let mut consumed: i64 = 0;
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        consumed += n as i64;
        if let Some(record) = build_record_from_line("gemini", &line, &mut last_session_id, &mut last_model)
        {
            records.push(record);
        }
    }

    update_scan_state(&conn, file_path, mtime, start_offset + consumed)?;
    Ok(records)
}

fn scan_jsonl_file(db: &TokenDb, file_path: &Path, agent: &str) -> Result<Vec<TokenRecord>> {
    let conn = lock_conn(db)?;
    let mtime = file_mtime(file_path)?;

    if !should_scan_by_mtime(&conn, file_path, mtime)? {
        return Ok(Vec::new());
    }

    let file = File::open(file_path)
        .with_context(|| format!("failed to open {} session file {}", agent, file_path.display()))?;
    let reader = BufReader::new(file);

    let mut records = Vec::new();
    let mut last_session_id: Option<String> = None;
    let mut last_model: Option<String> = None;
    for line in reader.lines() {
        let line = line?;
        if let Some(record) = build_record_from_line(agent, &line, &mut last_session_id, &mut last_model) {
            records.push(record);
        }
    }

    update_scan_state(&conn, file_path, mtime, 0)?;
    Ok(records)
}

fn build_record_from_line(
    agent: &str,
    line: &str,
    last_session_id: &mut Option<String>,
    last_model: &mut Option<String>,
) -> Option<TokenRecord> {
    let value: Value = serde_json::from_str(line).ok()?;
    update_session_model_hints(&value, last_session_id, last_model);
    let fields = extract_token_fields(&value)?;

    Some(TokenRecord {
        agent: agent.to_string(),
        session_id: current_session_id(&value, last_session_id),
        timestamp: extract_timestamp(&value),
        model: current_model(&value, last_model),
        input_tokens: fields.input_tokens,
        output_tokens: fields.output_tokens,
        cached_tokens: fields.cached_tokens,
        thinking_tokens: fields.thinking_tokens,
        total_tokens: fields.total_tokens,
        cost_usd: None,
        latency_ms: fields.latency_ms,
        tool_calls: fields.tool_calls,
        lines_added: fields.lines_added,
        lines_removed: fields.lines_removed,
        rate_limit_pct: fields.rate_limit_pct,
        context_window: fields.context_window,
        build_id: None,
        task_id: None,
        wave: None,
        usage_source: crate::USAGE_SOURCE_EXACT.to_string(),
    })
}

fn collect_records_from_value(
    value: &Value,
    agent: &str,
    last_session_id: &mut Option<String>,
    last_model: &mut Option<String>,
    records: &mut Vec<TokenRecord>,
) {
    update_session_model_hints(value, last_session_id, last_model);

    if let Some(fields) = extract_token_fields_shallow(value) {
        records.push(TokenRecord {
            agent: agent.to_string(),
            session_id: current_session_id(value, last_session_id),
            timestamp: extract_timestamp(value),
            model: current_model(value, last_model),
            input_tokens: fields.input_tokens,
            output_tokens: fields.output_tokens,
            cached_tokens: fields.cached_tokens,
            thinking_tokens: fields.thinking_tokens,
            total_tokens: fields.total_tokens,
            cost_usd: None,
            latency_ms: fields.latency_ms,
            tool_calls: fields.tool_calls,
            lines_added: fields.lines_added,
            lines_removed: fields.lines_removed,
            rate_limit_pct: fields.rate_limit_pct,
            context_window: fields.context_window,
            build_id: None,
            task_id: None,
            wave: None,
            usage_source: crate::USAGE_SOURCE_EXACT.to_string(),
        });
    }

    match value {
        Value::Array(items) => {
            for item in items {
                collect_records_from_value(item, agent, last_session_id, last_model, records);
            }
        }
        Value::Object(map) => {
            for nested in map.values() {
                collect_records_from_value(nested, agent, last_session_id, last_model, records);
            }
        }
        _ => {}
    }
}

fn extract_token_fields(value: &Value) -> Option<TokenFields> {
    let objects = candidate_objects(value);
    for obj in objects {
        let input_tokens = first_i64(
            obj,
            &[
                "input_tokens",
                "input",
                "promptTokenCount",
                "prompt_tokens",
                "usageMetadata.promptTokenCount",
            ],
        );
        let output_tokens = first_i64(
            obj,
            &[
                "output_tokens",
                "output",
                "candidatesTokenCount",
                "completion_tokens",
                "usageMetadata.candidatesTokenCount",
            ],
        );
        let cached_tokens = first_i64(
            obj,
            &[
                "cached_input_tokens",
                "cached",
                "cachedContentTokenCount",
                "usageMetadata.cachedContentTokenCount",
            ],
        )
        .unwrap_or(0);
        let thinking_tokens = first_i64(
            obj,
            &[
                "thinking_tokens",
                "thoughtsTokenCount",
                "usageMetadata.thoughtsTokenCount",
            ],
        )
        .unwrap_or(0);
        let total_tokens = first_i64(
            obj,
            &[
                "total_tokens",
                "total",
                "totalTokenCount",
                "usageMetadata.totalTokenCount",
            ],
        )
        .unwrap_or(input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0) + cached_tokens + thinking_tokens);

        if input_tokens.is_none() && output_tokens.is_none() && total_tokens == 0 {
            continue;
        }

        return Some(TokenFields {
            input_tokens: input_tokens.unwrap_or(0),
            output_tokens: output_tokens.unwrap_or(0),
            cached_tokens,
            thinking_tokens,
            total_tokens,
            latency_ms: first_i64(obj, &["duration_ms", "latency_ms", "totalLatencyMs", "stats.duration_ms"]),
            tool_calls: first_i64(obj, &["tool_calls", "totalCalls", "tools.totalCalls", "stats.tool_calls"]),
            lines_added: first_i64(obj, &["lines_added", "linesAdded", "files.linesAdded"]),
            lines_removed: first_i64(obj, &["lines_removed", "linesRemoved", "files.linesRemoved"]),
            rate_limit_pct: first_f64(obj, &["rate_limit_pct"]),
            context_window: first_i64(obj, &["context_window"]),
        });
    }
    None
}

fn extract_token_fields_shallow(value: &Value) -> Option<TokenFields> {
    let obj = value.as_object()?;

    let input_tokens = first_i64(
        obj,
        &[
            "input_tokens",
            "input",
            "promptTokenCount",
            "prompt_tokens",
            "usageMetadata.promptTokenCount",
        ],
    );
    let output_tokens = first_i64(
        obj,
        &[
            "output_tokens",
            "output",
            "candidatesTokenCount",
            "completion_tokens",
            "usageMetadata.candidatesTokenCount",
        ],
    );
    let cached_tokens = first_i64(
        obj,
        &[
            "cached_input_tokens",
            "cached",
            "cachedContentTokenCount",
            "usageMetadata.cachedContentTokenCount",
        ],
    )
    .unwrap_or(0);
    let thinking_tokens = first_i64(
        obj,
        &[
            "thinking_tokens",
            "thoughtsTokenCount",
            "usageMetadata.thoughtsTokenCount",
        ],
    )
    .unwrap_or(0);
    let total_tokens = first_i64(
        obj,
        &[
            "total_tokens",
            "total",
            "totalTokenCount",
            "usageMetadata.totalTokenCount",
        ],
    )
    .unwrap_or(input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0) + cached_tokens + thinking_tokens);

    if input_tokens.is_none() && output_tokens.is_none() && total_tokens == 0 {
        return None;
    }

    Some(TokenFields {
        input_tokens: input_tokens.unwrap_or(0),
        output_tokens: output_tokens.unwrap_or(0),
        cached_tokens,
        thinking_tokens,
        total_tokens,
        latency_ms: first_i64(obj, &["duration_ms", "latency_ms", "totalLatencyMs", "stats.duration_ms"]),
        tool_calls: first_i64(obj, &["tool_calls", "totalCalls", "tools.totalCalls", "stats.tool_calls"]),
        lines_added: first_i64(obj, &["lines_added", "linesAdded", "files.linesAdded"]),
        lines_removed: first_i64(obj, &["lines_removed", "linesRemoved", "files.linesRemoved"]),
        rate_limit_pct: first_f64(obj, &["rate_limit_pct"]),
        context_window: first_i64(obj, &["context_window"]),
    })
}

fn candidate_objects(value: &Value) -> Vec<&Map<String, Value>> {
    let mut out = Vec::new();
    let mut stack = vec![value];

    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                out.push(map);
                for key in ["usage", "stats", "usageMetadata", "tools", "files"] {
                    if let Some(v) = map.get(key) {
                        stack.push(v);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    stack.push(item);
                }
            }
            _ => {}
        }
    }

    out
}

fn first_i64(obj: &Map<String, Value>, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(value) = get_obj_path(obj, key) {
            if let Some(n) = value.as_i64() {
                return Some(n);
            }
            if let Some(n) = value.as_u64() {
                return Some(n as i64);
            }
            if let Some(n) = value.as_f64() {
                return Some(n as i64);
            }
            if let Some(s) = value.as_str()
                && let Ok(n) = s.parse::<i64>()
            {
                return Some(n);
            }
        }
    }
    None
}

fn first_f64(obj: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    for key in keys {
        if let Some(value) = get_obj_path(obj, key) {
            if let Some(n) = value.as_f64() {
                return Some(n);
            }
            if let Some(n) = value.as_i64() {
                return Some(n as f64);
            }
            if let Some(n) = value.as_u64() {
                return Some(n as f64);
            }
            if let Some(s) = value.as_str()
                && let Ok(n) = s.parse::<f64>()
            {
                return Some(n);
            }
        }
    }
    None
}

fn get_obj_path<'a>(obj: &'a Map<String, Value>, key_path: &str) -> Option<&'a Value> {
    let mut current: Option<&Value> = None;
    for (idx, part) in key_path.split('.').enumerate() {
        if idx == 0 {
            current = obj.get(part);
        } else {
            current = current.and_then(|v| v.get(part));
        }
    }
    current
}

fn update_session_model_hints(
    value: &Value,
    last_session_id: &mut Option<String>,
    last_model: &mut Option<String>,
) {
    if let Some(session_id) = extract_session_id(value) {
        *last_session_id = Some(session_id);
    }
    if let Some(model) = extract_model(value) {
        *last_model = Some(model);
    }
}

fn current_session_id(value: &Value, fallback: &Option<String>) -> String {
    extract_session_id(value)
        .or_else(|| fallback.clone())
        .unwrap_or_else(|| "unknown-session".to_string())
}

fn current_model(value: &Value, fallback: &Option<String>) -> Option<String> {
    extract_model(value).or_else(|| fallback.clone())
}

fn extract_session_id(value: &Value) -> Option<String> {
    for key in ["session_id", "thread_id", "conversation_id", "sessionUuid", "chat_id"] {
        if let Some(s) = find_string(value, key) {
            return Some(s);
        }
    }
    None
}

fn extract_model(value: &Value) -> Option<String> {
    for key in ["model", "model_name", "modelId"] {
        if let Some(s) = find_string(value, key) {
            return Some(s);
        }
    }
    None
}

fn extract_timestamp(value: &Value) -> String {
    for key in ["timestamp", "created_at", "time"] {
        if let Some(ts) = find_string(value, key) {
            return ts;
        }
    }
    "1970-01-01T00:00:00Z".to_string()
}

fn find_string(value: &Value, key: &str) -> Option<String> {
    let mut stack = vec![value];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if let Some(v) = map.get(key).and_then(|v| v.as_str()) {
                    return Some(v.to_string());
                }
                for nested in map.values() {
                    stack.push(nested);
                }
            }
            Value::Array(items) => {
                for item in items {
                    stack.push(item);
                }
            }
            _ => {}
        }
    }
    None
}

fn should_scan_by_mtime(conn: &Connection, file_path: &Path, current_mtime: i64) -> Result<bool> {
    let state = get_scan_state(conn, file_path)?;
    Ok(match state {
        Some(state) => current_mtime > state.last_mtime,
        None => true,
    })
}

fn file_mtime(file_path: &Path) -> Result<i64> {
    let metadata = std::fs::metadata(file_path)
        .with_context(|| format!("failed to read metadata for {}", file_path.display()))?;
    let modified = metadata
        .modified()
        .with_context(|| format!("failed to read mtime for {}", file_path.display()))?;
    let secs = modified
        .duration_since(UNIX_EPOCH)
        .with_context(|| format!("mtime before epoch for {}", file_path.display()))?
        .as_secs();
    Ok(secs as i64)
}

fn lock_conn(db: &TokenDb) -> Result<std::sync::MutexGuard<'_, Connection>> {
    db.conn
        .lock()
        .map_err(|_| anyhow::anyhow!("token DB connection mutex poisoned"))
}

fn get_scan_state(conn: &Connection, file_path: &Path) -> Result<Option<ScanState>> {
    let mut stmt =
        conn.prepare("SELECT last_mtime, last_offset FROM scan_state WHERE file_path = ?1")?;
    let mut rows = stmt.query(params![file_path.to_string_lossy()])?;
    if let Some(row) = rows.next()? {
        Ok(Some(ScanState {
            last_mtime: row.get(0)?,
            last_offset: row.get(1)?,
        }))
    } else {
        Ok(None)
    }
}

fn update_scan_state(conn: &Connection, file_path: &Path, last_mtime: i64, last_offset: i64) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO scan_state (file_path, last_mtime, last_offset)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(file_path) DO UPDATE SET
            last_mtime = excluded.last_mtime,
            last_offset = excluded.last_offset
        "#,
        params![file_path.to_string_lossy(), last_mtime, last_offset],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, OpenOptions},
        io::Write,
        time::Duration,
    };

    use crate::open;

    use super::{
        scan_claude_file, scan_codex_file, scan_gemini_chat_file, scan_gemini_telemetry_file,
    };

    #[test]
    fn scans_claude_jsonl_and_skips_unchanged_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open db");
        let session_file = temp.path().join("claude-session.jsonl");
        fs::write(
            &session_file,
            r#"{"type":"session_start","session_id":"claude-s1","model":"claude-sonnet-4"}"#.to_string()
                + "\n"
                + r#"{"timestamp":"2026-04-10T12:00:00Z","usage":{"input_tokens":100,"output_tokens":25,"cached_input_tokens":10,"total_tokens":135}}"#
                + "\n",
        )
        .expect("write session file");

        let first = scan_claude_file(&db, &session_file).expect("first scan");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].agent, "claude");
        assert_eq!(first[0].session_id, "claude-s1");
        assert_eq!(first[0].input_tokens, 100);
        assert_eq!(first[0].output_tokens, 25);
        assert_eq!(first[0].cached_tokens, 10);
        assert_eq!(first[0].total_tokens, 135);

        let second = scan_claude_file(&db, &session_file).expect("second scan");
        assert!(second.is_empty());
    }

    #[test]
    fn scans_codex_jsonl_turn_completed_usage() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open db");
        let session_file = temp.path().join("codex-session.jsonl");
        fs::write(
            &session_file,
            r#"{"type":"thread.started","thread_id":"codex-thread-1"}"#.to_string()
                + "\n"
                + r#"{"type":"turn.completed","usage":{"input_tokens":87291,"cached_input_tokens":49920,"output_tokens":157}}"#
                + "\n",
        )
        .expect("write session file");

        let records = scan_codex_file(&db, &session_file).expect("scan codex file");
        assert_eq!(records.len(), 1);
        let rec = &records[0];
        assert_eq!(rec.agent, "codex");
        assert_eq!(rec.session_id, "codex-thread-1");
        assert_eq!(rec.input_tokens, 87291);
        assert_eq!(rec.cached_tokens, 49920);
        assert_eq!(rec.output_tokens, 157);
        assert_eq!(rec.total_tokens, 137368);
    }

    #[test]
    fn scans_gemini_chat_json_structure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open db");
        let chat_file = temp.path().join("chat.json");
        fs::write(
            &chat_file,
            r#"{
              "session_id":"gemini-chat-1",
              "model":"gemini-2.5-pro",
              "events":[
                {"type":"message","role":"assistant","content":"done"},
                {"type":"result","timestamp":"2026-04-10T12:30:00Z","stats":{"input_tokens":43617,"output_tokens":42,"cached":18444,"total_tokens":43893,"duration_ms":19516,"tool_calls":1}}
              ]
            }"#,
        )
        .expect("write chat file");

        let records = scan_gemini_chat_file(&db, &chat_file).expect("scan gemini chat");
        assert_eq!(records.len(), 1);
        let rec = &records[0];
        assert_eq!(rec.agent, "gemini");
        assert_eq!(rec.session_id, "gemini-chat-1");
        assert_eq!(rec.model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(rec.input_tokens, 43617);
        assert_eq!(rec.output_tokens, 42);
        assert_eq!(rec.cached_tokens, 18444);
        assert_eq!(rec.total_tokens, 43893);
        assert_eq!(rec.latency_ms, Some(19516));
        assert_eq!(rec.tool_calls, Some(1));
    }

    #[test]
    fn scans_gemini_telemetry_incrementally_by_offset() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open db");
        let telemetry_file = temp.path().join("telemetry.jsonl");
        fs::write(
            &telemetry_file,
            r#"{"timestamp":"2026-04-10T13:00:00Z","session_id":"gemini-telemetry-1","model":"gemini-2.5-pro","usageMetadata":{"promptTokenCount":200,"candidatesTokenCount":50,"cachedContentTokenCount":30,"thoughtsTokenCount":12,"totalTokenCount":292},"tools":{"totalCalls":2}}"#
                .to_string()
                + "\n",
        )
        .expect("write initial telemetry");

        let first = scan_gemini_telemetry_file(&db, &telemetry_file).expect("first telemetry scan");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].session_id, "gemini-telemetry-1");
        assert_eq!(first[0].thinking_tokens, 12);
        assert_eq!(first[0].tool_calls, Some(2));

        std::thread::sleep(Duration::from_secs(1));
        let mut file = OpenOptions::new()
            .append(true)
            .open(&telemetry_file)
            .expect("open telemetry for append");
        writeln!(
            file,
            r#"{{"timestamp":"2026-04-10T13:01:00Z","session_id":"gemini-telemetry-1","usageMetadata":{{"promptTokenCount":10,"candidatesTokenCount":5,"totalTokenCount":15}}}}"#
        )
        .expect("append telemetry");

        let second = scan_gemini_telemetry_file(&db, &telemetry_file).expect("second telemetry scan");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].input_tokens, 10);
        assert_eq!(second[0].output_tokens, 5);
        assert_eq!(second[0].total_tokens, 15);
    }

    #[test]
    fn codex_mtime_incremental_skip_then_rescan() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = open(&temp.path().join("token-economics.db")).expect("open db");
        let session_file = temp.path().join("codex-incremental.jsonl");
        fs::write(
            &session_file,
            r#"{"type":"thread.started","thread_id":"codex-inc"}"#.to_string()
                + "\n"
                + r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":2}}"#
                + "\n",
        )
        .expect("write codex file");

        let first = scan_codex_file(&db, &session_file).expect("first scan");
        assert_eq!(first.len(), 1);

        let second = scan_codex_file(&db, &session_file).expect("second scan");
        assert!(second.is_empty());

        std::thread::sleep(Duration::from_secs(1));
        let mut file = OpenOptions::new()
            .append(true)
            .open(&session_file)
            .expect("open codex file");
        writeln!(
            file,
            r#"{{"type":"turn.completed","usage":{{"input_tokens":20,"output_tokens":4}}}}"#
        )
        .expect("append codex line");

        let third = scan_codex_file(&db, &session_file).expect("third scan");
        assert_eq!(third.len(), 2);
    }
}
