#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureClass {
    WorkerError,
    ContractError,
    OrchestratorBriefingError,
    EnvironmentError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub class: FailureClass,
    pub reason: String,
}

pub fn classify_failure(log_text: &str) -> Classification {
    let lower = log_text.to_lowercase();

    if lower.contains("command not found") || lower.contains("sandbox") {
        return Classification {
            class: FailureClass::EnvironmentError,
            reason: "environment dependency or sandbox error".to_string(),
        };
    }
    if lower.contains("blocked: write to") || lower.contains("denied by contract") {
        return Classification {
            class: FailureClass::ContractError,
            reason: "contract blocked required change".to_string(),
        };
    }
    if lower.contains("orchestrator") || lower.contains("briefing") {
        return Classification {
            class: FailureClass::OrchestratorBriefingError,
            reason: "briefing quality issue".to_string(),
        };
    }
    Classification {
        class: FailureClass::OrchestratorBriefingError,
        reason: "unclassified failure — conservative default, send to Gemini".to_string(),
    }
}

pub fn can_retry(class: &FailureClass, class_attempts: u32, total_attempts: u32) -> bool {
    if total_attempts >= 5 {
        return false;
    }
    match class {
        FailureClass::WorkerError => class_attempts < 3,
        FailureClass::ContractError => class_attempts < 2,
        FailureClass::OrchestratorBriefingError => class_attempts < 2,
        FailureClass::EnvironmentError => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{can_retry, classify_failure, FailureClass};

    #[test]
    fn classification_paths_are_deterministic() {
        assert_eq!(
            classify_failure("BLOCKED: Write to src/cli.rs denied by contract").class,
            FailureClass::ContractError
        );
        assert_eq!(
            classify_failure("command not found: cargo").class,
            FailureClass::EnvironmentError
        );
        assert_eq!(
            classify_failure("orchestrator briefing omitted required file").class,
            FailureClass::OrchestratorBriefingError
        );
        assert_eq!(
            classify_failure("stub marker TODO found").class,
            FailureClass::OrchestratorBriefingError
        );
    }

    #[test]
    fn retry_caps_follow_spec() {
        assert!(can_retry(&FailureClass::WorkerError, 2, 2));
        assert!(!can_retry(&FailureClass::WorkerError, 3, 3));
        assert!(!can_retry(&FailureClass::EnvironmentError, 0, 0));
        assert!(!can_retry(&FailureClass::ContractError, 1, 5));
    }
}
