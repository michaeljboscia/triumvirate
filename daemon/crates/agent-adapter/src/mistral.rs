//! Mistral stream parser (Seven Primes REQ-022, REQ-044). research/01..03 hold the stream shapes; fixtures are
//! transcribed from vendor source (`*-provisional-20261018.jsonl`) until T-302 commits live captures.
//!
//! T-003 scaffold: every public function returns `Err("not implemented: T-102")` until T-102 replaces it. The public
//! signatures and the shared types are final; Wave 1 tasks change bodies, not shapes.

use crate::{SeatParsed, WorkingStateEvent};

pub const PARSER_MODE: &str = "mistral-streaming";

/// Line-by-line parser for one dispatch.
#[derive(Debug, Default)]
pub struct MistralStreamParser {
    lines: Vec<String>,
}

impl MistralStreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one stdout line; returns a working-state event when the line carries one.
    pub fn parse_line(&mut self, line: &str) -> Option<WorkingStateEvent> {
        self.lines.push(line.to_string());
        None
    }

    /// Finish the dispatch given the exit code and the contents of the per-call export.json (`None` when missing).
    pub fn finish(
        self,
        exit_code: Option<i32>,
        export_json: Option<&str>,
    ) -> Result<SeatParsed, String> {
        let _ = (self.lines, exit_code, export_json);
        Err("not implemented: T-102".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RED IF: a stub stops being wired to its public signature, or is mistaken for an implementation.
    #[test]
    fn t003_stubs_are_wired_and_refuse() {
        let mut p = MistralStreamParser::new();
        assert!(p.parse_line("{}").is_none());
        assert_eq!(
            p.finish(Some(0), None).unwrap_err(),
            "not implemented: T-102"
        );
        assert_eq!(PARSER_MODE, "mistral-streaming");
    }
}
