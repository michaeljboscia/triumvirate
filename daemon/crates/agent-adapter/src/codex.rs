use crate::types::{
    ParsedAgentResult, TokenUsage, ToolCallRecord, ToolKind, WorkingState, WorkingStateEvent,
};
use shared_types::AgentStreamEvent;
use tokio::sync::mpsc;

#[derive(Debug, Default)]
pub struct CodexExecParser {
    thread_id: Option<String>,
    response_chunks: Vec<String>,
    events: Vec<WorkingStateEvent>,
    tool_calls: Vec<ToolCallRecord>,
    token_usage: Option<TokenUsage>,
    stream_tx: Option<mpsc::Sender<AgentStreamEvent>>,
    stream_seq: u64,
}

/// Strip codex's shell wrapper, if present.
///
/// codex does not report `cat /repo/a.rs`. It reports
/// `/bin/zsh -lc 'cat /repo/a.rs'`, verified live on 2026-09-01: the first version of the
/// classifier assumed the bare form, saw the program as `zsh`, and classified every read as
/// Bash. The offline tests all passed because they used the shape I imagined rather than the
/// shape codex emits. Only the live test caught it.
///
/// Returns the inner command when the outer one is a recognised shell invoked with `-c`
/// (including `-lc`, `-lic`), and the input unchanged otherwise. A shell whose script is not
/// a single quoted string is left alone, so anything unusual falls through to the caller's
/// fail-closed checks.
fn unwrap_shell_wrapper(cmd: &str) -> &str {
    const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];
    let mut it = cmd.split_whitespace();
    let Some(program) = it.next() else {
        return cmd;
    };
    let base = program.rsplit('/').next().unwrap_or(program);
    if !SHELLS.contains(&base) {
        return cmd;
    }
    // The flag bundle must be a -c form: -c, -lc, -lic, -ic.
    let Some(flag) = it.next() else {
        return cmd;
    };
    if !(flag.starts_with('-') && flag.ends_with('c')) {
        return cmd;
    }
    let rest = cmd[cmd.find(flag).map(|i| i + flag.len()).unwrap_or(cmd.len())..].trim();
    // Only a single fully-quoted script is unwrapped; anything else stays as-is and is then
    // rejected by the caller's compound/redirection checks.
    for q in ['\'', '"'] {
        if rest.starts_with(q) && rest.ends_with(q) && rest.len() >= 2 {
            let inner = &rest[1..rest.len() - 1];
            if !inner.contains(q) {
                return inner;
            }
        }
    }
    cmd
}

/// Does this shell command READ file contents, as opposed to merely naming a path?
///
/// `codex exec` reports every action as `command_execution` with `ToolKind::Bash`, so on that
/// backend "opened the file" and "ran a command mentioning the file" were the same record. That
/// made `required_sources` unenforceable for codex, the peer most likely to be reviewing code,
/// and the sight gate refused named sources there rather than fake them.
///
/// This narrows it honestly. A CONSERVATIVE ALLOWLIST of programs whose job is to emit file
/// contents. Anything unlisted stays `Bash` and still cannot satisfy a source, so an unknown
/// command fails closed.
///
/// Deliberately excluded, and worth stating: `ls`, `find`, `stat`, `file` and `mdfind` name
/// paths without reading them, which is the exact hole that let a search satisfy a source on
/// the agy backend. `grep` and `rg` are excluded TOO, for the pattern-position reason in the
/// body: `rg needle a.rs` reads a.rs while the boundary matcher counted `needle` as opened.
///
/// A compound or piped command is NOT classified, because the reader may not be the part that
/// touched the named path. Any redirection disqualifies, and `sed -i` writes.
/// Programs that put the WHOLE file in front of the model, as opposed to a slice of it.
///
/// FIND-REVIEW-07. Grok found the hole in round 3 and it is the same class it named in
/// FIND-REVIEW-06, moved to the other end of the file. Putting the proof-of-read nonce on the
/// LAST line turned `head -1` into `tail -1`, and `tail` sits in the reader list right next to
/// `head`. One command satisfies the sight gate and returns the nonce, and the work under
/// review never enters the model's context.
///
/// So a source that must be READ is now only satisfied by a program that emits all of it.
///
/// `head`, `tail` and `cut` are readers (they do put file contents in front of the model, which
/// is why they stay in READERS and still count as a read for the no-touch check) but they are
/// PARTIAL: a slice of lines or a slice of columns. `more` and `less` are pagers; non-interactively
/// they dump everything, but that depends on the terminal and on `$PAGER`, so they are treated as
/// partial rather than reasoned about. Fail closed: the cost is a false rejection of an unusual
/// full read, never a false pass on a one-line peek.
pub fn command_reads_whole_file(command: &str) -> bool {
    const WHOLE_FILE_READERS: &[&str] =
        &["cat", "nl", "bat", "od", "xxd", "strings", "pr", "zcat"];
    if !command_reads_file_contents(command) {
        return false;
    }
    let cmd = unwrap_shell_wrapper(command.trim());
    let Some(program) = cmd.split_whitespace().next() else {
        return false;
    };
    let base = program.rsplit('/').next().unwrap_or(program);
    WHOLE_FILE_READERS.contains(&base)
}

fn command_reads_file_contents(command: &str) -> bool {
    // ONLY programs that emit FILE CONTENTS to the model.
    //
    // The first version included `grep`, `rg`, `wc`, `shasum`, `diff`, `cmp`, `awk`, `sed`,
    // `jq` and `yq`, and every one of them was a hole:
    //
    //   `rg /repo/required.rs /repo/other.rs`  reads other.rs; required.rs is the PATTERN, and
    //                                          the boundary matcher counted it as opened.
    //   `wc /repo/a.rs`                        shows the model a COUNT, not the file.
    //   `shasum`, `cmp`                        same: a summary, not contents.
    //   `yq -i`, `awk -i inplace`, `perl -i`   MUTATE while classified as reads. Only `sed -i`
    //                                          was checked, and `sed -ix` slipped past that.
    //
    // Codex found the pattern-position hole, Antigravity found the in-place-mutation bypass,
    // and Grok named the principle that fixes all of them: a read is a program that puts the
    // file's CONTENTS in front of the model. Anything else is a search or a summary, and
    // `sight_21` already forbids a search from satisfying a source on the other backends.
    //
    // The doc comment on this function used to say "`grep` and `rg` ARE included: they read the
    // file to match against it." That was FALSE: the array below does not contain them, and the
    // list above records why they were taken out. Grok caught the stale sentence in round 3.
    // In this repo a wrong comment is a defect, because it is what the next reader trusts.
    //
    // Unknown programs stay Bash and fail closed, so the cost of being strict here is a false
    // REJECTION of an unusual reader, never a false pass.
    const READERS: &[&str] = &[
        "cat", "head", "tail", "nl", "bat", "od", "xxd", "strings", "cut", "pr", "zcat", "more",
        "less",
    ];
    let cmd = unwrap_shell_wrapper(command.trim());
    if cmd.is_empty() {
        return false;
    }
    if cmd.contains("&&") || cmd.contains("||") || cmd.contains(';') || cmd.contains('|')
        || cmd.contains('&')
    {
        return false;
    }
    if cmd.contains('>') {
        return false;
    }
    let Some(program) = cmd.split_whitespace().next() else {
        return false;
    };
    let base = program.rsplit('/').next().unwrap_or(program);
    // Belt and braces: no in-place flag may ever ride a reader, whatever gets added later.
    // `sed -ix` and `sed --in-place` both slipped past the old `" -i"` substring check.
    for tok in cmd.split_whitespace().skip(1) {
        if tok == "-i" || tok.starts_with("-i") && tok.len() <= 4 || tok.starts_with("--in-place")
        {
            return false;
        }
    }
    READERS.contains(&base)
}

impl CodexExecParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_stream_channel(tx: mpsc::Sender<AgentStreamEvent>) -> Self {
        Self {
            stream_tx: Some(tx),
            ..Self::default()
        }
    }

    fn emit_stream_event(&mut self, event: AgentStreamEvent) {
        if let Some(tx) = &self.stream_tx {
            let _ = tx.try_send(event);
        }
    }

    fn next_seq(&mut self) -> u64 {
        self.stream_seq += 1;
        self.stream_seq
    }

    pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent> {
        let json: serde_json::Value = serde_json::from_str(line).ok()?;
        let event_type = json.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        match event_type {
            "thread.started" => {
                self.thread_id = json.get("thread_id").and_then(|v| v.as_str()).map(ToString::to_string);
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::TurnStarted,
                    detail: "thread started".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::TurnStarted {
                    agent: "codex".into(),
                    session_name: self.thread_id.clone().unwrap_or_default(),
                    seq,
                });
                Some(event)
            }
            "turn.started" => {
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::TurnStarted,
                    detail: "turn started".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::TurnStarted {
                    agent: "codex".into(),
                    session_name: self.thread_id.clone().unwrap_or_default(),
                    seq,
                });
                Some(event)
            }
            "item.started" => self.parse_item_event(&json, true),
            "item.completed" => self.parse_item_event(&json, false),
            "turn.completed" => {
                let usage = json.get("usage").cloned().unwrap_or_default();
                let token_usage = TokenUsage {
                    input: usage.get("input_tokens").and_then(|v| v.as_u64()),
                    output: usage.get("output_tokens").and_then(|v| v.as_u64()),
                    cached: usage.get("cached_input_tokens").and_then(|v| v.as_u64()),
                    // 0.145 reports reasoning tokens separately as `reasoning_output_tokens`;
                    // map to thinking_tokens (already emitted as tv_thinking_tokens). Previously
                    // dropped, so codex reasoning volume went uncounted.
                    thinking_tokens: usage.get("reasoning_output_tokens").and_then(|v| v.as_u64()),
                    latency_ms: None,
                    tool_calls: None,
                    total: None,
                };
                self.token_usage = Some(token_usage.clone());
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::TurnCompleted,
                    detail: "turn completed".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: Some(token_usage.clone()),
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::TurnCompleted {
                    agent: "codex".into(),
                    tokens_in: token_usage.input.unwrap_or(0) as i64,
                    tokens_out: token_usage.output.unwrap_or(0) as i64,
                    cached_tokens: token_usage.cached.map(|c| c as i64),
                    tool_count: self.tool_calls.len() as i64,
                    duration_ms: token_usage.latency_ms.unwrap_or(0),
                    seq,
                });
                Some(event)
            }
            "error" => {
                let detail = json
                    .get("message")
                    .or_else(|| json.get("error"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("codex error")
                    .to_string();
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::Error,
                    detail: detail.clone(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                let seq = self.next_seq();
                self.emit_stream_event(AgentStreamEvent::Error {
                    agent: "codex".into(),
                    message: detail,
                    seq,
                });
                Some(event)
            }
            _ => {
                tracing::debug!("unknown codex event type: {event_type}");
                None
            }
        }
    }

    fn parse_item_event(&mut self, json: &serde_json::Value, started: bool) -> Option<WorkingStateEvent> {
        let item = json.get("item")?;
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        match item_type {
            "agent_message" => {
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or_default();
                if !text.is_empty() {
                    self.response_chunks.push(text.to_string());
                }
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: WorkingState::MessageDelta,
                    detail: "assistant response chunk".to_string(),
                    tool_name: None,
                    tool_args_json: None,
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                Some(event)
            }
            "command_execution" => {
                let command = item.get("command").and_then(|v| v.as_str()).unwrap_or_default();
                if started {
                    self.tool_calls.push(ToolCallRecord {
                        id: item.get("id").and_then(|v| v.as_str()).map(ToString::to_string),
                        tool: "command_execution".to_string(),
                        // A pure content reader is classified as a READ so codex can satisfy
                        // `required_sources`. Everything else stays Bash and cannot.
                        kind: if command_reads_file_contents(command) {
                            ToolKind::ReadFile
                        } else {
                            ToolKind::Bash
                        },
                        success: None,
                        duration_ms: None,
                        args_json: Some(serde_json::json!({"command": command}).to_string()),
                    });
                } else {
                    let id = item.get("id").and_then(|v| v.as_str());
                    let exit_code = item.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
                    if let Some(id) = id
                        && let Some(existing) = self.tool_calls.iter_mut().find(|r| r.id.as_deref() == Some(id))
                    {
                        existing.success = Some(exit_code == 0);
                    }
                }
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state: if started {
                        WorkingState::CommandStarted
                    } else {
                        WorkingState::CommandCompleted
                    },
                    detail: if started {
                        "running command".to_string()
                    } else {
                        "command completed".to_string()
                    },
                    tool_name: Some("command_execution".to_string()),
                    tool_args_json: Some(serde_json::json!({"command": command}).to_string()),
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                if started {
                    let seq = self.next_seq();
                    self.emit_stream_event(AgentStreamEvent::ToolCall {
                        agent: "codex".into(),
                        tool_name: "bash".into(),
                        args_summary: command.to_string(),
                        seq,
                    });
                }
                Some(event)
            }
            _ => {
                let state = if started {
                    WorkingState::ToolCallStarted
                } else {
                    WorkingState::ToolCallCompleted
                };
                let event = WorkingStateEvent {
                    agent: "codex".to_string(),
                    state,
                    detail: format!("{} {}", if started { "started" } else { "completed" }, item_type),
                    tool_name: Some(item_type.to_string()),
                    tool_args_json: Some(item.to_string()),
                    token_usage: None,
                    ts_ms: None,
                };
                self.events.push(event.clone());
                Some(event)
            }
        }
    }

    pub fn finish(self) -> ParsedAgentResult {
        ParsedAgentResult {
            response_text: self.response_chunks.join("\n"),
            session_id: self.thread_id,
            events: self.events,
            tool_calls: self.tool_calls,
            token_usage: self.token_usage,
            self_reported_cost_usd: None,
            cli_version: None,
            parser_mode: "codex-exec-json".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_golden_trace() {
        let mut parser = CodexExecParser::new();
        let raw = include_str!("../../../tests/fixtures/codex-exec-trace.jsonl");
        let mut parsed = 0;
        for line in raw.lines() {
            if parser.parse_line(line).is_some() {
                parsed += 1;
            }
        }
        let result = parser.finish();
        assert!(parsed >= 5);
        assert_eq!(
            result.session_id.as_deref(),
            Some("019d626f-c562-7a43-b388-f48c1d9b8dc8")
        );
        assert!(result.response_text.contains("8 crates"));
        assert_eq!(result.token_usage.as_ref().and_then(|t| t.output), Some(157));
    }
}

#[cfg(test)]
mod command_classification_tests {
    use super::*;

    /// Programs that emit file contents are reads, so codex can satisfy `required_sources`.
    /// RED IF: the reader allowlist is emptied, which returns codex to being unable to
    /// source-gate at all.
    #[test]
    fn codex_01_content_readers_are_reads() {
        for c in [
            "cat crates/foo/src/lib.rs",
            "head -n 50 /repo/a.rs",
            "/usr/bin/cat /repo/a.rs",
            "bat /repo/a.rs",
            "cut -d, -f1 /repo/a.rs",
        ] {
            assert!(command_reads_file_contents(c), "should be a read: {c}");
        }
    }

    /// THE SHAPE CODEX ACTUALLY EMITS, captured live: a shell wrapper around the real command.
    ///
    /// The first classifier assumed a bare `cat /repo/a.rs`. codex emits
    /// `/bin/zsh -lc 'cat /repo/a.rs'`, so the program looked like `zsh` and every read was
    /// classified Bash. Every offline test passed because they all used the shape I imagined.
    /// Only the live test caught it.
    ///
    /// RED IF: the shell unwrapper is removed, which silently returns codex to being
    /// unable to satisfy a named source.
    #[test]
    fn codex_05_the_shell_wrapper_codex_actually_emits_is_unwrapped() {
        assert!(
            command_reads_file_contents("/bin/zsh -lc 'cat /repo/a.rs'"),
            "this is the literal shape from a live codex turn"
        );
        assert!(command_reads_file_contents("bash -c \"head -n 5 /repo/a.rs\""));
        // A search inside the wrapper is still not a read.
        assert!(!command_reads_file_contents("/bin/zsh -lc 'rg needle /repo/a.rs'"));
        assert!(command_reads_file_contents("/bin/sh -lic 'tail -n 5 /repo/a.rs'"));
        // The wrapper must not launder a non-reader.
        assert!(!command_reads_file_contents("/bin/zsh -lc 'ls /repo'"));
        // Nor a compound script: the reader may not be what touched the named path.
        assert!(!command_reads_file_contents("/bin/zsh -lc 'ls /repo && cat /repo/a.rs'"));
        // A shell without a -c form is left alone and fails closed.
        assert!(!command_reads_file_contents("/bin/zsh script.sh"));
    }

    /// Naming a path is not reading it. This is the hole that let a search satisfy a source on
    /// the agy backend, and it must not reopen on codex.
    /// RED IF: ls, find, stat or mdfind are added to the reader allowlist.
    #[test]
    fn codex_02_naming_a_path_is_not_reading_it() {
        for c in [
            "ls -la /repo/a.rs",
            "find . -name a.rs",
            "stat /repo/a.rs",
            "file /repo/a.rs",
            "mdfind a.rs",
            "test -f /repo/a.rs",
            // SEARCHES and SUMMARIES. None of these put the file's contents in front of the
            // model, and the first two let the PATH be the search pattern rather than the file
            // operand: `rg /repo/required.rs /repo/other.rs` reads other.rs.
            "rg needle /repo/a.rs",
            "grep -n fn /repo/a.rs",
            "rg /repo/required.rs /repo/other.rs",
            "wc /repo/a.rs",
            "shasum /repo/a.rs",
            "diff /repo/a.rs /repo/b.rs",
            // PROGRAMMABLE, and all have in-place flags.
            "sed -n '1,80p' /repo/a.rs",
            "awk '{print}' /repo/a.rs",
            "jq . /repo/a.json",
        ] {
            assert!(!command_reads_file_contents(c), "must NOT be a read: {c}");
        }
    }

    /// Fail closed on anything the classifier cannot reason about.
    /// RED IF: compound commands, pipes or redirections start being classified, where the
    /// reader may not be the part that touched the named path, or the command writes.
    #[test]
    fn codex_03_compound_and_writing_commands_fail_closed() {
        for c in [
            "ls /repo && cat /repo/a.rs",
            "cat /repo/a.rs | grep x",
            "cat /repo/a.rs > /tmp/copy",
            "sed -i '' 's/a/b/' /repo/a.rs",
            "cat -i /repo/a.rs",
            "cat --in-place /repo/a.rs",
            "cat /repo/a.rs & rm /repo/b.rs",
            "python3 -c 'open(\"/repo/a.rs\")'",
            "",
        ] {
            assert!(!command_reads_file_contents(c), "must fail closed: {c}");
        }
    }

    /// The parser must actually apply the classification, not just define it.
    /// RED IF: command_execution goes back to hardcoding ToolKind::Bash.
    #[test]
    fn codex_04_the_parser_applies_the_classification() {
        let mut p = CodexExecParser::new();
        let read = r#"{"type":"item.started","item":{"type":"command_execution","id":"c1","command":"cat /repo/a.rs"}}"#;
        let list = r#"{"type":"item.started","item":{"type":"command_execution","id":"c2","command":"ls /repo"}}"#;
        let _ = p.parse_line(read);
        let _ = p.parse_line(list);
        let r = p.finish();
        let by_id = |id: &str| {
            r.tool_calls
                .iter()
                .find(|c| c.id.as_deref() == Some(id))
                .unwrap_or_else(|| panic!("missing {id}"))
                .kind
                .clone()
        };
        assert_eq!(by_id("c1"), ToolKind::ReadFile, "`cat` reads the file");
        assert_eq!(by_id("c2"), ToolKind::Bash, "`ls` does not");
    }
}
