use serde::{Deserialize, Serialize};

const OPEN_TAG: &str = "<triumvirate_tool";
const CLOSE_TAG: &str = "</triumvirate_tool>";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallRequest {
    pub name: String,
    pub params: serde_json::Value,
}

pub fn parse_tool_call_marker(stdout: &str) -> anyhow::Result<Option<ToolCallRequest>> {
    let Some(open_start) = stdout.find(OPEN_TAG) else {
        return Ok(None);
    };

    let tag_close_offset = stdout[open_start..]
        .find('>')
        .ok_or_else(|| anyhow::anyhow!("triumvirate_tool opening tag is not terminated"))?;
    let tag_close = open_start + tag_close_offset;
    let opening_tag = &stdout[open_start..=tag_close];
    let name = extract_name(opening_tag)?;

    let body_start = tag_close + 1;
    let close_offset = stdout[body_start..]
        .find(CLOSE_TAG)
        .ok_or_else(|| anyhow::anyhow!("triumvirate_tool closing tag is missing"))?;
    let body_end = body_start + close_offset;
    let body = stdout[body_start..body_end].trim();

    let params = if body.is_empty() {
        serde_json::json!({})
    } else {
        let value: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| anyhow::anyhow!("invalid triumvirate_tool params JSON: {e}"))?;
        if !value.is_object() {
            anyhow::bail!("triumvirate_tool params must decode to a JSON object");
        }
        value
    };

    Ok(Some(ToolCallRequest { name, params }))
}

fn extract_name(opening_tag: &str) -> anyhow::Result<String> {
    let key = "name=\"";
    let start = opening_tag
        .find(key)
        .ok_or_else(|| anyhow::anyhow!("triumvirate_tool is missing required name attribute"))?
        + key.len();
    let remainder = &opening_tag[start..];
    let end = remainder
        .find('"')
        .ok_or_else(|| anyhow::anyhow!("triumvirate_tool name attribute is malformed"))?;
    let name = remainder[..end].trim();
    if name.is_empty() {
        anyhow::bail!("triumvirate_tool name attribute cannot be empty");
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::{ToolCallRequest, parse_tool_call_marker};

    #[test]
    fn parses_valid_marker_with_object_params() {
        let input = r#"
            prep text
            <triumvirate_tool name="ledger_record">{"title":"Architecture decision","summary_type":"architecture_decision"}</triumvirate_tool>
            tail text
        "#;
        let parsed = parse_tool_call_marker(input).expect("parse marker");
        assert_eq!(
            parsed,
            Some(ToolCallRequest {
                name: "ledger_record".to_string(),
                params: serde_json::json!({
                    "title": "Architecture decision",
                    "summary_type": "architecture_decision"
                }),
            })
        );
    }

    #[test]
    fn malformed_marker_returns_error() {
        let input = r#"<triumvirate_tool name="ledger_record">{"title":"x"}"#;
        let err = parse_tool_call_marker(input).expect_err("must error");
        assert!(err.to_string().contains("closing tag"));
    }

    #[test]
    fn text_without_marker_returns_none() {
        let parsed = parse_tool_call_marker("normal assistant output").expect("parse");
        assert!(parsed.is_none());
    }
}
