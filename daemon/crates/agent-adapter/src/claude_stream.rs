//! claude `--output-format stream-json --verbose` parser.
//!
//! WHY THIS EXISTS. `claude` is a DEFAULT peer reviewer (`peer-review::default_reviewers`), and
//! until now its runner built `ParsedAgentResult` from `Default::default()`: empty
//! `parser_mode`, empty `tool_calls`. So the `claude` seat on the panel was structurally
//! incapable of producing a receipt. Once the reviewer dispatch is sight-gated, that seat
//! rejects 100% of the time however carefully it looked, which is the exact "instrument is
//! blind, blame the agent" failure this repo already hit with agy and wrote a rule about.
//!
//! Same shape of fix as `agy_stream.rs`, same reason.
//!
//! THE WIRE SHAPE, captured live on 2026-09-02 from the installed CLI, not from documentation:
//!
//! ```text
//! {"type":"system","subtype":"init","cwd":"...","session_id":"...","tools":[...]}
//! {"type":"system","subtype":"thinking_tokens","estimated_tokens":45,...}
//! {"type":"assistant","message":{"id":"msg_...","role":"assistant","content":[
//!    {"type":"text","text":"..."},
//!    {"type":"tool_use","id":"toolu_...","name":"Read","input":{"file_path":"/etc/hosts"}}]}}
//! {"type":"user","message":{"role":"user","content":[
//!    {"type":"tool_result","tool_use_id":"toolu_...","content":"1\t##"}]}}
//! {"type":"rate_limit_event","rate_limit_info":{...}}
//! {"type":"result","subtype":"success","is_error":false,"num_turns":2,
//!    "session_id":"...","result":"The first line ...","usage":{...}}
//! ```
//!
//! TWO TRAPS, both hit while capturing the fixture, both recorded so nobody re-derives them:
//!
//! 1. `--allowedTools` is VARIADIC and will swallow the prompt. `claude --allowedTools
//!    "Read,Grep,Glob" "the prompt"` dies with "Input must be provided either through stdin or
//!    as a prompt argument when using --print", because the prompt was consumed as another tool
//!    name. The prompt must arrive behind an explicit `-p`. This is the same class of trap as
//!    agy's `-p`, which silently swallows the flag that follows it.
//! 2. Without an allow-list, a headless claude AUTO-DENIES the read and emits
//!    `{"type":"system","subtype":"permission_denied"}` plus a `tool_result` with
//!    `is_error: true`. The turn then looks exactly like a reviewer that chose not to look. A
//!    zero from a blocked instrument is not evidence about the agent.
//!
//! SUCCESS CLASSIFICATION. `tool_result` carries `is_error` only when it failed; a successful
//! result OMITS the field. So absent means success, which is the opposite of the defensive
//! reading, and getting it backwards would mark every successful read as failed and reject
//! every review. Verified against the capture: the allowed read has no `is_error`, the denied
//! one has `is_error: true`.

use std::collections::HashMap;

use serde_json::Value;

use crate::types::{
    ParsedAgentResult, TokenUsage, ToolCallRecord, ToolKind, WorkingState, WorkingStateEvent,
};

/// The parser mode this emits. Belongs on the sight gate's allowlist; the empty string the
/// plain-text runner produced does not, and fails closed there.
pub const PARSER_MODE_STREAM: &str = "claude-stream-json";

/// Map a claude tool name to the kind the sight gate reasons about.
///
/// Names are claude's own, taken from the `system.init` event's `tools` array in the capture.
fn tool_kind(name: &str) -> ToolKind {
    match name {
        "Read" | "NotebookRead" => ToolKind::ReadFile,
        "Write" => ToolKind::WriteFile,
        "Edit" | "MultiEdit" | "NotebookEdit" => ToolKind::EditFile,
        "Bash" | "BashOutput" | "KillShell" => ToolKind::Bash,
        "Grep" | "WebSearch" | "WebFetch" => ToolKind::Grep,
        "Glob" | "LS" => ToolKind::Glob,
        "AskUserQuestion" => ToolKind::RequestUserInput,
        // OPAQUE EFFECT, classified EditFile on purpose, matching the ruling already made for
        // agy's `invoke_subagent` and `call_mcp_tool`.
        //
        // These delegate to something whose actions this stream never records. A subagent or an
        // MCP server can write, and the delegating turn shows only the delegation. Treating
        // them as a mutation REJECTS a review that used one, rather than accepting it as clean.
        // The false-rejection risk is accepted: a reviewer has no business delegating, and the
        // alternative is a write the gate cannot see.
        "Task" | "Agent" | "Artifact" | "SlashCommand" | "Skill" | "SendMessage"
        | "RemoteTrigger" | "CronCreate" | "CronDelete" | "Workflow" => ToolKind::EditFile,
        // An MCP tool. Same reasoning, and matched by prefix because the server half of the
        // name is arbitrary.
        n if n.starts_with("mcp__") => ToolKind::EditFile,
        _ => ToolKind::Unknown,
    }
}

fn parse_usage(u: &Value) -> TokenUsage {
    let g = |k: &str| u.get(k).and_then(Value::as_u64);
    TokenUsage {
        input: g("input_tokens"),
        output: g("output_tokens"),
        // Cache reads are kept OUT of `input` so a cached turn is not double counted, matching
        // how the agy and grok parsers treat their own cache fields.
        cached: g("cache_read_input_tokens"),
        thinking_tokens: u
            .get("output_tokens_details")
            .and_then(|d| d.get("thinking_tokens"))
            .and_then(Value::as_u64),
        latency_ms: None,
        tool_calls: None,
        total: None,
    }
}

fn event(
    state: WorkingState,
    detail: impl Into<String>,
    tool_name: Option<String>,
    tool_args_json: Option<String>,
    token_usage: Option<TokenUsage>,
) -> WorkingStateEvent {
    WorkingStateEvent {
        agent: "claude".to_string(),
        state,
        detail: detail.into(),
        tool_name,
        tool_args_json,
        token_usage,
        ts_ms: None,
    }
}

#[derive(Debug, Default)]
pub struct ClaudeStreamParser {
    text_chunks: Vec<String>,
    final_response: Option<String>,
    session_id: Option<String>,
    events: Vec<WorkingStateEvent>,
    tool_calls: Vec<ToolCallRecord>,
    usage: Option<TokenUsage>,
    /// `tool_use.id` to its index in `tool_calls`, so the later `tool_result` can set success
    /// on the right record. Claude interleaves turns, so position is not a safe pairing key.
    pending: HashMap<String, usize>,
    /// True when the CLI reported the whole turn as an error.
    is_error: bool,
}

impl ClaudeStreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one NDJSON line. Unparseable lines are ignored rather than fatal: the CLI prints
    /// hook output and occasional plain warnings to stdout, and one stray line must not lose an
    /// entire turn.
    pub fn parse_line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("system") => self.on_system(&v),
            Some("assistant") => self.on_assistant(&v),
            Some("user") => self.on_user(&v),
            Some("result") => self.on_result(&v),
            _ => {}
        }
    }

    fn on_system(&mut self, v: &Value) {
        match v.get("subtype").and_then(Value::as_str) {
            Some("init") => {
                if let Some(id) = v.get("session_id").and_then(Value::as_str) {
                    self.session_id = Some(id.to_string());
                }
                self.events.push(event(
                    WorkingState::TurnStarted,
                    "claude session started",
                    None,
                    None,
                    None,
                ));
            }
            // The blocked-instrument signal, surfaced rather than swallowed. A turn that hit
            // this looks identical to a reviewer that chose not to look, and the difference
            // matters more than anything else this parser records.
            Some("permission_denied") => {
                self.events.push(event(
                    WorkingState::Error,
                    "claude was DENIED a tool it asked for: it could not look, which is not the \
                     same as choosing not to. Dispatch with an explicit --allowedTools list.",
                    v.get("tool_name").and_then(Value::as_str).map(str::to_string),
                    None,
                    None,
                ));
            }
            _ => {}
        }
    }

    fn on_assistant(&mut self, v: &Value) {
        let Some(content) = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for block in content {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(Value::as_str)
                        && !t.is_empty()
                    {
                        self.text_chunks.push(t.to_string());
                        self.events.push(event(
                            WorkingState::MessageDelta,
                            t.chars().take(120).collect::<String>(),
                            None,
                            None,
                            None,
                        ));
                    }
                }
                Some("tool_use") => {
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    let id = block.get("id").and_then(Value::as_str).map(str::to_string);
                    let args_json = block.get("input").map(|i| i.to_string());
                    if let Some(ref id) = id {
                        self.pending.insert(id.clone(), self.tool_calls.len());
                    }
                    self.tool_calls.push(ToolCallRecord {
                        id: id.clone(),
                        kind: tool_kind(&name),
                        tool: name.clone(),
                        // Unknown until the matching tool_result arrives. A turn cut off before
                        // that leaves it None, which the sight gate treats as "not a proven
                        // successful read" rather than as a success.
                        success: None,
                        duration_ms: None,
                        args_json: args_json.clone(),
                    });
                    self.events.push(event(
                        WorkingState::ToolCallStarted,
                        format!("claude called {name}"),
                        Some(name),
                        args_json,
                        None,
                    ));
                }
                _ => {}
            }
        }
    }

    fn on_user(&mut self, v: &Value) {
        let Some(content) = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            // ABSENT MEANS SUCCESS. The CLI only emits `is_error` when the call failed.
            // Defaulting to failure here would mark every successful read as failed and reject
            // every review; defaulting to success would mark a denied read as a look. Read the
            // field, and treat only an explicit `true` as failure.
            let ok = !block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let idx = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .and_then(|id| self.pending.remove(id));
            if let Some(idx) = idx
                && let Some(rec) = self.tool_calls.get_mut(idx)
            {
                rec.success = Some(ok);
                let detail = format!(
                    "claude {} {}",
                    rec.tool,
                    if ok { "completed" } else { "FAILED" }
                );
                let tool = rec.tool.clone();
                self.events.push(event(
                    if ok {
                        WorkingState::ToolCallCompleted
                    } else {
                        WorkingState::Error
                    },
                    detail,
                    Some(tool),
                    None,
                    None,
                ));
            }
        }
    }

    fn on_result(&mut self, v: &Value) {
        if let Some(id) = v.get("session_id").and_then(Value::as_str) {
            self.session_id = Some(id.to_string());
        }
        if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
            self.is_error = true;
        }
        if let Some(text) = v.get("result").and_then(Value::as_str) {
            self.final_response = Some(text.to_string());
        }
        if let Some(u) = v.get("usage") {
            self.usage = Some(parse_usage(u));
        }
        self.events.push(event(
            if self.is_error {
                WorkingState::Error
            } else {
                WorkingState::TurnCompleted
            },
            format!(
                "claude turn finished ({})",
                v.get("subtype").and_then(Value::as_str).unwrap_or("unknown")
            ),
            None,
            None,
            self.usage.clone(),
        ));
    }

    /// True when the CLI reported the turn itself as failed, so a caller can mark the answer
    /// rather than presenting it as clean.
    pub fn turn_failed(&self) -> bool {
        self.is_error
    }

    /// True when a `result` event actually arrived, which is the only proof this stream really
    /// was stream-json.
    ///
    /// The runner needs this to decide the parser mode HONESTLY. A claude invoked in plain text
    /// (a mock binary in tests, an operator override in TRIUMVIRATE_CLAUDE_ARGS, an older CLI)
    /// produces no events at all, and stamping `claude-stream-json` on that turn would put a
    /// blind parser on the sight gate's allowlist. That is the exact fig leaf the allowlist was
    /// built to remove, so the mode is only claimed when the receipt mechanism demonstrably ran.
    pub fn saw_result(&self) -> bool {
        self.final_response.is_some()
    }

    pub fn finish(self) -> ParsedAgentResult {
        // `result.result` is the whole answer and wins. The accumulated text blocks are the
        // fallback for a stream that ended without a result event, so a truncated turn still
        // yields whatever text arrived rather than nothing.
        let response_text = self
            .final_response
            .clone()
            .unwrap_or_else(|| self.text_chunks.join(""))
            .trim()
            .to_string();
        ParsedAgentResult {
            response_text,
            session_id: self.session_id,
            events: self.events,
            tool_calls: self.tool_calls,
            token_usage: self.usage,
            self_reported_cost_usd: None,
            cli_version: None,
            parser_mode: PARSER_MODE_STREAM.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REAL captures from `claude 2.x` on 2026-09-02, not hand-written.
    ///
    /// Two edits were made and both are disclosed: `session_id` is replaced with a constant, and
    /// the `thinking` block's base64 `signature` is truncated to a marker (it was 2KB of noise
    /// that no code path reads). Event shapes, field names and the presence or absence of
    /// `is_error` are exactly as captured.
    const FIXTURE_ALLOWED: &str =
        include_str!("../tests/fixtures/claude-stream-read-2026-09-02.jsonl");
    const FIXTURE_DENIED: &str =
        include_str!("../tests/fixtures/claude-stream-denied-2026-09-02.jsonl");

    fn replay(fixture: &str) -> (ParsedAgentResult, bool) {
        let mut p = ClaudeStreamParser::new();
        for line in fixture.lines() {
            p.parse_line(line);
        }
        let failed = p.turn_failed();
        (p.finish(), failed)
    }

    /// THE POINT OF THE WHOLE FILE. Before this parser the claude runner returned
    /// `tool_calls: vec![]` and an empty `parser_mode`, so a claude reviewer could never satisfy
    /// the sight gate however carefully it looked.
    /// RED IF: tool calls stop being recorded, or the parser mode changes without the sight
    /// gate's allowlist changing with it.
    #[test]
    fn u_cs_01_a_read_is_recorded_as_a_successful_read() {
        let (parsed, failed) = replay(FIXTURE_ALLOWED);
        assert!(!failed, "the captured turn succeeded");
        assert_eq!(parsed.parser_mode, "claude-stream-json");
        assert_eq!(parsed.tool_calls.len(), 1, "one Read in the capture");

        let call = &parsed.tool_calls[0];
        assert_eq!(call.tool, "Read");
        assert_eq!(call.kind, ToolKind::ReadFile);
        assert_eq!(
            call.success,
            Some(true),
            "a tool_result with NO is_error field is a SUCCESS; reading it the other way would \
             mark every successful read as failed and reject every review"
        );
        assert!(
            call.args_json.as_deref().unwrap_or("").contains("/etc/hosts"),
            "the path must survive into args_json or required_sources can never match it"
        );
    }

    /// The blocked-instrument case, from the capture where claude was NOT given an allow-list.
    ///
    /// It asked to read, was auto-denied, and produced a turn that looks exactly like a reviewer
    /// which chose not to look. The distinction has to survive into the record, because the
    /// first read of the equivalent agy failure blamed the model and it was the harness.
    /// RED IF: a denied call is recorded as a successful one, or permission_denied is dropped.
    #[test]
    fn u_cs_02_a_denied_read_is_a_failure_not_a_look() {
        let (parsed, _) = replay(FIXTURE_DENIED);
        assert_eq!(parsed.tool_calls.len(), 1);
        let call = &parsed.tool_calls[0];
        assert_eq!(call.tool, "Read");
        assert_eq!(
            call.success,
            Some(false),
            "an explicit is_error:true must not be recorded as a look"
        );
        assert!(
            parsed
                .events
                .iter()
                .any(|e| e.state == WorkingState::Error
                    && e.detail.contains("DENIED")),
            "the denial must be visible in the record, not inferred from a zero"
        );
    }

    /// The answer must come from `result.result` and must not be contaminated by thinking
    /// blocks, tool arguments or tool output.
    /// RED IF: thinking text starts leaking into the response.
    #[test]
    fn u_cs_03_the_answer_is_clean() {
        let (parsed, _) = replay(FIXTURE_ALLOWED);
        assert!(
            parsed.response_text.contains("/etc/hosts"),
            "got: {}",
            parsed.response_text
        );
        assert!(
            !parsed.response_text.contains("TRUNCATED-FOR-FIXTURE"),
            "a thinking block's signature must never reach the answer"
        );
        assert!(
            !parsed.response_text.contains("Let me use the Read tool"),
            "thinking must not contaminate the answer"
        );
    }

    /// Token usage comes from the stream. Cache reads stay OUT of `input`, matching the ruling
    /// already made for the agy and grok parsers, or a cached turn is double counted.
    /// RED IF: cache_read_input_tokens is folded into input.
    #[test]
    fn u_cs_04_usage_is_parsed_without_double_counting() {
        let (parsed, _) = replay(FIXTURE_ALLOWED);
        let u = parsed.token_usage.expect("the result event carries usage");
        assert_eq!(u.input, Some(18));
        assert_eq!(u.output, Some(226));
        assert_eq!(u.cached, Some(49878));
        assert_eq!(u.thinking_tokens, Some(128));
    }

    /// A delegating call is classified as a mutation, the same ruling already made for agy's
    /// `invoke_subagent` and `call_mcp_tool`. A subagent or MCP server can write, and the
    /// delegating turn records only the delegation.
    /// RED IF: Task or an mcp__ tool falls back to Unknown, which the no-touch check ignores.
    #[test]
    fn u_cs_05_delegation_counts_as_a_mutation() {
        for name in ["Task", "Agent", "mcp__github__create_issue", "Workflow"] {
            assert_eq!(
                tool_kind(name),
                ToolKind::EditFile,
                "{name} delegates to something this stream cannot see"
            );
        }
        assert_eq!(tool_kind("Read"), ToolKind::ReadFile);
        assert_eq!(tool_kind("Write"), ToolKind::WriteFile);
        assert_eq!(tool_kind("Bash"), ToolKind::Bash);
    }

    /// A tool_use whose result never arrives (turn cut off) must stay `None`, not become a
    /// success. The sight gate only counts a read with `success == Some(true)`.
    /// RED IF: unpaired calls default to true.
    #[test]
    fn u_cs_06_an_unfinished_call_is_not_a_success() {
        let mut p = ClaudeStreamParser::new();
        p.parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"/a"}}]}}"#,
        );
        let parsed = p.finish();
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].success, None);
    }

    /// Results are paired by `tool_use_id`, not by position. Claude interleaves calls, so a
    /// positional pairing would attribute one call's failure to another call.
    /// RED IF: pairing goes back to order of arrival.
    #[test]
    fn u_cs_07_results_pair_by_id_not_by_position() {
        let mut p = ClaudeStreamParser::new();
        p.parse_line(
            r#"{"type":"assistant","message":{"content":[
               {"type":"tool_use","id":"toolu_a","name":"Read","input":{"file_path":"/a"}},
               {"type":"tool_use","id":"toolu_b","name":"Read","input":{"file_path":"/b"}}]}}"#,
        );
        // Deliberately out of order.
        p.parse_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_b","is_error":true,"content":"nope"}]}}"#,
        );
        p.parse_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_a","content":"ok"}]}}"#,
        );
        let parsed = p.finish();
        assert_eq!(parsed.tool_calls[0].success, Some(true), "/a succeeded");
        assert_eq!(parsed.tool_calls[1].success, Some(false), "/b failed");
    }

    /// Junk on stdout must not lose the turn. Hook output and plain warnings share this stream.
    /// RED IF: an unparseable line becomes fatal.
    #[test]
    fn u_cs_08_stray_output_does_not_lose_the_turn() {
        let mut p = ClaudeStreamParser::new();
        p.parse_line("not json at all");
        p.parse_line("");
        p.parse_line(r#"{"type":"result","subtype":"success","is_error":false,"result":"the answer"}"#);
        assert_eq!(p.finish().response_text, "the answer");
    }
}
