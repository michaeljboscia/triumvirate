use serde_json::Value;

#[derive(Debug, Clone)]
pub enum CodexEventKind {
    Response,
    Error,
    Notification,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct CodexEvent {
    pub kind: CodexEventKind,
    pub raw: Value,
}

impl CodexEvent {
    pub fn text_content(&self) -> Option<String> {
        extract_text(&self.raw)
    }

    pub fn error_message(&self) -> Option<String> {
        if let Some(message) = self
            .raw
            .get("error")
            .and_then(|v| v.get("message"))
            .and_then(Value::as_str)
        {
            return Some(message.to_string());
        }

        self.raw
            .get("message")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
    }
}

pub fn parse_codex_event(line: &str) -> serde_json::Result<Option<CodexEvent>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let raw: Value = serde_json::from_str(trimmed)?;
    let kind = if raw.get("error").is_some() {
        CodexEventKind::Error
    } else if raw.get("result").is_some() {
        CodexEventKind::Response
    } else if raw.get("method").is_some() {
        CodexEventKind::Notification
    } else {
        CodexEventKind::Unknown
    };

    Ok(Some(CodexEvent { kind, raw }))
}

fn extract_text(value: &Value) -> Option<String> {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(content) = value.get("content").and_then(Value::as_str) {
        return Some(content.to_string());
    }

    if let Some(result) = value.get("result") {
        if let Some(text) = result.get("text").and_then(Value::as_str) {
            return Some(text.to_string());
        }
        if let Some(content) = result.get("content").and_then(Value::as_str) {
            return Some(content.to_string());
        }
    }

    if let Some(params) = value.get("params") {
        if let Some(text) = params.get("text").and_then(Value::as_str) {
            return Some(text.to_string());
        }
        if let Some(content) = params.get("content").and_then(Value::as_str) {
            return Some(content.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{CodexEventKind, parse_codex_event};

    #[test]
    fn parses_notification_text() {
        let line = r#"{"jsonrpc":"2.0","method":"event","params":{"text":"hello"}}"#;
        let event = parse_codex_event(line).expect("valid").expect("some");
        assert!(matches!(event.kind, CodexEventKind::Notification));
        assert_eq!(event.text_content().as_deref(), Some("hello"));
    }

    #[test]
    fn parses_error_message() {
        let line = r#"{"jsonrpc":"2.0","id":99,"error":{"message":"bad method"}}"#;
        let event = parse_codex_event(line).expect("valid").expect("some");
        assert!(matches!(event.kind, CodexEventKind::Error));
        assert_eq!(event.error_message().as_deref(), Some("bad method"));
    }

    #[test]
    fn ignores_empty_line() {
        let event = parse_codex_event("\n\n").expect("ok");
        assert!(event.is_none());
    }
}
