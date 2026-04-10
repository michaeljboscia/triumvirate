use tracing::instrument;
use daemon_core::metrics::DaemonMetrics;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
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

#[instrument(skip_all)]
pub fn classify_failure(log_text: &str) -> Classification {
    classify_failure_with_metrics(log_text, None)
}

pub fn classify_failure_with_metrics(
    log_text: &str,
    metrics: Option<&DaemonMetrics>,
) -> Classification {
    let lower = log_text.to_lowercase();
    let classification = if lower.contains("stub marker")
        || lower.contains("test command failed")
        || lower.contains("validation failed")
    {
        Classification {
            class: FailureClass::WorkerError,
            reason: "worker produced non-compliant output".to_string(),
        }
    } else if lower.contains("command not found") || lower.contains("sandbox") {
        Classification {
            class: FailureClass::EnvironmentError,
            reason: "environment dependency or sandbox error".to_string(),
        }
    } else if lower.contains("blocked: write to") || lower.contains("denied by contract") {
        Classification {
            class: FailureClass::ContractError,
            reason: "contract blocked required change".to_string(),
        }
    } else if lower.contains("orchestrator") || lower.contains("briefing") {
        Classification {
            class: FailureClass::OrchestratorBriefingError,
            reason: "briefing quality issue".to_string(),
        }
    } else {
        Classification {
            class: FailureClass::OrchestratorBriefingError,
            reason: "unclassified failure — conservative default, send to Gemini".to_string(),
        }
    };
    if let Some(metrics) = metrics {
        metrics
            .abe_failure_class_total
            .with_label_values(&[failure_class_label(&classification.class)])
            .inc();
    }
    classification
}

#[instrument(skip_all)]
pub fn can_retry(class: &FailureClass, class_attempts: u32, total_attempts: u32) -> bool {
    can_retry_with_metrics(class, class_attempts, total_attempts, None)
}

pub fn can_retry_with_metrics(
    class: &FailureClass,
    class_attempts: u32,
    total_attempts: u32,
    metrics: Option<&DaemonMetrics>,
) -> bool {
    let should_retry = if total_attempts >= 5 {
        false
    } else {
        match class {
        FailureClass::WorkerError => class_attempts < 3,
        FailureClass::ContractError => class_attempts < 2,
        FailureClass::OrchestratorBriefingError => class_attempts < 2,
        FailureClass::EnvironmentError => false,
        }
    };
    if should_retry {
        if let Some(metrics) = metrics {
            metrics
                .abe_retry_total
                .with_label_values(&[failure_class_label(class)])
                .inc();
        }
    }
    should_retry
}

fn failure_class_label(class: &FailureClass) -> &'static str {
    match class {
        FailureClass::WorkerError => "worker-error",
        FailureClass::ContractError => "contract-error",
        FailureClass::OrchestratorBriefingError => "orchestrator-briefing-error",
        FailureClass::EnvironmentError => "environment-error",
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
            FailureClass::WorkerError
        );
        assert_eq!(
            classify_failure("validation failed after retries").class,
            FailureClass::WorkerError
        );
        assert_eq!(
            classify_failure("test command failed: cargo test").class,
            FailureClass::WorkerError
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
