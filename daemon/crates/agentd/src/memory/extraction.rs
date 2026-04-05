/// Extract explicit decision candidates from agent output.
///
/// Syntax gate:
/// - `# DECISION: <text>`
/// - `DECISION: <text>`
pub fn extract_decisions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("# DECISION:") {
                let decision = rest.trim();
                if !decision.is_empty() {
                    return Some(decision.to_string());
                }
            }
            if let Some(rest) = trimmed.strip_prefix("DECISION:") {
                let decision = rest.trim();
                if !decision.is_empty() {
                    return Some(decision.to_string());
                }
            }
            None
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::extract_decisions;

    #[test]
    fn extracts_hash_prefixed_decisions() {
        let input = "# DECISION: Use JWT\n# DECISION: Add refresh tokens";
        let decisions = extract_decisions(input);
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0], "Use JWT");
    }

    #[test]
    fn ignores_non_decision_lines() {
        let input = "hello\nworld\n# NOTE: not a decision";
        let decisions = extract_decisions(input);
        assert!(decisions.is_empty());
    }
}
