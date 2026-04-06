use crate::types::{WorkingState, WorkingStateEvent};
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StuckReason {
    IdleTimeout,
    Frozen,
    RepeatedToolCall,
    InputLoop,
}

#[derive(Debug)]
pub struct StuckDetector {
    started_at: Option<Instant>,
    last_event_at: Option<Instant>,
    repeat_counts: HashMap<String, u32>,
    input_denials: u32,
    idle_timeout: Duration,
    frozen_timeout: Duration,
    max_repeats: u32,
}

impl Default for StuckDetector {
    fn default() -> Self {
        Self {
            started_at: None,
            last_event_at: None,
            repeat_counts: HashMap::new(),
            input_denials: 0,
            idle_timeout: Duration::from_secs(60),
            frozen_timeout: Duration::from_secs(90),
            max_repeats: 5,
        }
    }
}

impl StuckDetector {
    pub fn observe(&mut self, event: &WorkingStateEvent) -> Option<StuckReason> {
        let now = Instant::now();
        if self.started_at.is_none() {
            self.started_at = Some(now);
        }
        self.last_event_at = Some(now);

        if matches!(event.state, WorkingState::InputRequested)
            && event.detail.to_lowercase().contains("denied")
        {
            self.input_denials += 1;
            if self.input_denials >= 3 {
                return Some(StuckReason::InputLoop);
            }
        }

        if matches!(event.state, WorkingState::ToolCallStarted)
            && let Some(name) = &event.tool_name
        {
            let key = format!("{}::{}", name, event.tool_args_json.as_deref().unwrap_or(""));
            let count = self.repeat_counts.entry(key).or_insert(0);
            *count += 1;
            if *count > self.max_repeats {
                return Some(StuckReason::RepeatedToolCall);
            }
        }

        None
    }

    pub fn check_timeouts(&self) -> Option<StuckReason> {
        let now = Instant::now();
        if let Some(started) = self.started_at
            && now.duration_since(started) > self.idle_timeout
            && self.last_event_at == Some(started)
        {
            return Some(StuckReason::IdleTimeout);
        }
        if let Some(last) = self.last_event_at
            && now.duration_since(last) > self.frozen_timeout
        {
            return Some(StuckReason::Frozen);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(state: WorkingState, detail: &str, tool_name: Option<&str>, args: Option<&str>) -> WorkingStateEvent {
        WorkingStateEvent {
            agent: "codex".to_string(),
            state,
            detail: detail.to_string(),
            tool_name: tool_name.map(ToString::to_string),
            tool_args_json: args.map(ToString::to_string),
            token_usage: None,
            ts_ms: None,
        }
    }

    #[test]
    fn repeated_tool_call_detected() {
        let mut detector = StuckDetector::default();
        let mut reason = None;
        for _ in 0..6 {
            reason = detector.observe(&event(
                WorkingState::ToolCallStarted,
                "calling ReadFile",
                Some("ReadFile"),
                Some("{\"file_path\":\"a\"}"),
            ));
        }
        assert_eq!(reason, Some(StuckReason::RepeatedToolCall));
    }

    #[test]
    fn input_loop_detected() {
        let mut detector = StuckDetector::default();
        let mut reason = None;
        for _ in 0..3 {
            reason = detector.observe(&event(
                WorkingState::InputRequested,
                "user input denied",
                Some("requestUserInput"),
                None,
            ));
        }
        assert_eq!(reason, Some(StuckReason::InputLoop));
    }
}
