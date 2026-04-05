use serde_json::Value;

#[derive(Debug, Clone)]
pub enum ClaudeEventKind {
    Init,
    Message,
    ToolUse,
    Result,
    Error,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ClaudeEvent {
    pub kind: ClaudeEventKind,
    pub raw: Value,
}

impl ClaudeEvent {
    pub fn text_content(&self) -> Option<String> {
        extract_text(&self.raw)
    }

    pub fn error_message(&self) -> Option<String> {
        if let Some(message) = self.raw.get("message").and_then(Value::as_str) {
            return Some(message.to_string());
        }
        if let Some(err) = self.raw.get("error") {
            if let Some(message) = err.get("message").and_then(Value::as_str) {
                return Some(message.to_string());
            }
            if let Some(as_str) = err.as_str() {
                return Some(as_str.to_string());
            }
        }
        None
    }
}

pub fn parse_claude_event(line: &str) -> serde_json::Result<Option<ClaudeEvent>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let raw: Value = serde_json::from_str(trimmed)?;

    let event_name = raw
        .get("type")
        .and_then(Value::as_str)
        .or_else(|| raw.get("event").and_then(Value::as_str))
        .or_else(|| raw.get("kind").and_then(Value::as_str))
        .unwrap_or_default()
        .to_ascii_lowercase();

    let kind = if event_name.contains("init") {
        ClaudeEventKind::Init
    } else if event_name.contains("message") || event_name.contains("assistant") {
        ClaudeEventKind::Message
    } else if event_name.contains("tool_use") || event_name.contains("tool-use") || event_name.contains("tool") {
        ClaudeEventKind::ToolUse
    } else if event_name.contains("result") || event_name.contains("completion") || event_name.contains("done") {
        ClaudeEventKind::Result
    } else if event_name.contains("error") {
        ClaudeEventKind::Error
    } else {
        ClaudeEventKind::Unknown
    };

    Ok(Some(ClaudeEvent { kind, raw }))
}

fn extract_text(value: &Value) -> Option<String> {
    if let Some(content) = value.get("content").and_then(Value::as_str) {
        return Some(content.to_string());
    }
    if let Some(content_blocks) = value.get("content").and_then(Value::as_array)
        && let Some(text) = extract_text_block_array(content_blocks)
    {
        return Some(text);
    }
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }

    if let Some(message) = value.get("message") {
        if let Some(content) = message.get("content").and_then(Value::as_str) {
            return Some(content.to_string());
        }
        if let Some(content_blocks) = message.get("content").and_then(Value::as_array)
            && let Some(text) = extract_text_block_array(content_blocks)
        {
            return Some(text);
        }
        if let Some(text) = message.get("text").and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }

    if let Some(delta) = value.get("delta")
        && let Some(text) = delta.get("text").and_then(Value::as_str)
    {
        return Some(text.to_string());
    }

    if let Some(result) = value.get("result") {
        if let Some(text) = result.as_str() {
            return Some(text.to_string());
        }
        if let Some(content) = result.get("content").and_then(Value::as_str) {
            return Some(content.to_string());
        }
        if let Some(text) = result.get("text").and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }

    None
}

fn extract_text_block_array(content_blocks: &[Value]) -> Option<String> {
    for block in content_blocks {
        let is_text_block = block.get("type").and_then(Value::as_str) == Some("text");
        if !is_text_block {
            continue;
        }
        if let Some(text) = block.get("text").and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{parse_claude_event, ClaudeEventKind};

    #[test]
    fn parses_message_event_and_text() {
        let line = r#"{"type":"message","content":"hello"}"#;
        let parsed = parse_claude_event(line).expect("valid json").expect("non-empty");
        assert!(matches!(parsed.kind, ClaudeEventKind::Message));
        assert_eq!(parsed.text_content().as_deref(), Some("hello"));
    }

    #[test]
    fn parses_result_event_and_nested_text() {
        let line = r#"{"type":"result","result":{"text":"done"}}"#;
        let parsed = parse_claude_event(line).expect("valid json").expect("non-empty");
        assert!(matches!(parsed.kind, ClaudeEventKind::Result));
        assert_eq!(parsed.text_content().as_deref(), Some("done"));
    }

    #[test]
    fn parses_assistant_event_with_content_blocks() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hello from assistant"}]}}"#;
        let parsed = parse_claude_event(line).expect("valid json").expect("non-empty");
        assert!(matches!(parsed.kind, ClaudeEventKind::Message));
        assert_eq!(parsed.text_content().as_deref(), Some("hello from assistant"));
    }

    #[test]
    fn parses_result_event_with_plain_string() {
        let line = r#"{"type":"result","result":"done"}"#;
        let parsed = parse_claude_event(line).expect("valid json").expect("non-empty");
        assert!(matches!(parsed.kind, ClaudeEventKind::Result));
        assert_eq!(parsed.text_content().as_deref(), Some("done"));
    }

    #[test]
    fn parses_error_event_and_message() {
        let line = r#"{"type":"error","error":{"message":"boom"}}"#;
        let parsed = parse_claude_event(line).expect("valid json").expect("non-empty");
        assert!(matches!(parsed.kind, ClaudeEventKind::Error));
        assert_eq!(parsed.error_message().as_deref(), Some("boom"));
    }

    #[test]
    fn empty_line_is_ignored() {
        let parsed = parse_claude_event("   ").expect("empty line should parse");
        assert!(parsed.is_none());
    }
}
