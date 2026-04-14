use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DispatchSandbox {
    WorkspaceWrite,
    ReadOnly,
    DangerFullAccess,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DispatchCodexRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<DispatchSandbox>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DispatchCodexResponse {
    pub task_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum FilePolicy {
    DefaultDeny,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ContractFields {
    pub task_id: String,
    pub req_ids: Vec<String>,
    pub wave: u32,
    pub file_policy: FilePolicy,
    pub allowed_files: Vec<String>,
    pub forbidden_files: Vec<String>,
    pub allowed_commands: Vec<Vec<String>>,
    pub forbidden_commands: Vec<Vec<String>>,
    pub commit_format: String,
    pub test_command: String,
    pub task_timeout_sec: u64,
    pub done_when: String,
    pub reality_test: String,
    /// Optional codex-exec `-c sandbox_permissions=[...]` extensions.
    /// Known values: "network-full-access", "disk-full-read-access", "disk-write-cwd".
    /// Omitted/empty = default sandbox (no extensions) — preserves prior behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_permissions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DispatchCodexWorktreeRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    pub sha: String,
    pub briefing_content: String,
    pub contract_fields: ContractFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_failed_worktree: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DispatchCodexWorktreeResponse {
    pub task_id: String,
    pub worktree_path: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TaskCompleteRequest {
    pub task_id: String,
    pub commit_sha: String,
    pub result: String,
    pub timestamp: String,
    pub commit_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DispatchErrorCode {
    DaemonUnavailable,
    SetupFailed,
    InvalidSha,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct DispatchErrorResponse {
    pub error: DispatchErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct QueryGeminiRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct QueryGeminiResponse {
    pub response: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GeminiReviewMode {
    Pass,
    Failure,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct QueryGeminiReviewRequest {
    pub diff: String,
    pub mode: GeminiReviewMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub briefing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<ContractFields>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GeminiReviewVerdict {
    Clean,
    Concerns,
    Regression,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct QueryGeminiReviewResponse {
    pub verdict: GeminiReviewVerdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concerns: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct GetTaskStatusRequest {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Working,
    Completed,
    Stuck,
    Failed,
    Timeout,
    SetupFailed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct GetTaskStatusResponse {
    pub task_id: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct GetTaskOutputRequest {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct GetTaskOutputResponse {
    pub task_id: String,
    pub commit_sha: String,
    pub modified_files: Vec<String>,
    pub stdout: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_log: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CancelTaskRequest {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct CancelTaskResponse {
    pub task_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractValidationIssue {
    pub field: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractValidationError {
    pub issues: Vec<ContractValidationIssue>,
}

impl std::fmt::Display for ContractValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = self
            .issues
            .iter()
            .map(|issue| format!("{}: {}", issue.field, issue.message))
            .collect::<Vec<_>>()
            .join("; ");
        write!(f, "invalid contract fields: {msg}")
    }
}

impl std::error::Error for ContractValidationError {}

pub fn validate_contract(fields: &ContractFields) -> Result<(), ContractValidationError> {
    let mut issues = Vec::new();

    if fields.task_id.trim().is_empty() {
        issues.push(ContractValidationIssue {
            field: "task_id",
            message: "is required".to_string(),
        });
    }
    if fields.req_ids.is_empty() {
        issues.push(ContractValidationIssue {
            field: "req_ids",
            message: "must contain at least one requirement id".to_string(),
        });
    }
    if fields.allowed_files.is_empty() {
        issues.push(ContractValidationIssue {
            field: "allowed_files",
            message: "must include at least one writeable file".to_string(),
        });
    }
    if fields.allowed_commands.is_empty() {
        issues.push(ContractValidationIssue {
            field: "allowed_commands",
            message: "must include at least one allowed command prefix".to_string(),
        });
    }
    if fields.commit_format.trim().is_empty() {
        issues.push(ContractValidationIssue {
            field: "commit_format",
            message: "is required".to_string(),
        });
    }
    if fields.test_command.trim().is_empty() {
        issues.push(ContractValidationIssue {
            field: "test_command",
            message: "is required".to_string(),
        });
    }
    if fields.task_timeout_sec == 0 {
        issues.push(ContractValidationIssue {
            field: "task_timeout_sec",
            message: "must be > 0".to_string(),
        });
    }
    if fields.done_when.trim().is_empty() {
        issues.push(ContractValidationIssue {
            field: "done_when",
            message: "is required".to_string(),
        });
    }
    if fields.reality_test.trim().is_empty() {
        issues.push(ContractValidationIssue {
            field: "reality_test",
            message: "is required".to_string(),
        });
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(ContractValidationError { issues })
    }
}

pub fn parse_and_validate_contract(raw: &str) -> Result<ContractFields, ContractValidationError> {
    let fields: ContractFields = match serde_json::from_str(raw) {
        Ok(parsed) => parsed,
        Err(err) => {
            return Err(ContractValidationError {
                issues: vec![ContractValidationIssue {
                    field: "contract",
                    message: format!("invalid json: {err}"),
                }],
            })
        }
    };
    validate_contract(&fields)?;
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::{
        parse_and_validate_contract, validate_contract, ContractFields, DispatchCodexWorktreeRequest,
        FilePolicy,
    };

    fn valid_contract() -> ContractFields {
        ContractFields {
            task_id: "T-003".to_string(),
            req_ids: vec!["REQ-002".to_string()],
            wave: 1,
            file_policy: FilePolicy::DefaultDeny,
            allowed_files: vec!["src/engine.rs".to_string()],
            forbidden_files: vec!["src/cli.rs".to_string()],
            allowed_commands: vec![vec!["cargo".to_string(), "test".to_string()]],
            forbidden_commands: vec![vec!["rm".to_string(), "-rf".to_string()]],
            commit_format: "^T-003:".to_string(),
            test_command: "cargo test".to_string(),
            task_timeout_sec: 600,
            done_when: "Behavior implemented".to_string(),
            reality_test: "End-to-end test passes".to_string(),
            sandbox_permissions: None,
        }
    }

    #[test]
    fn contract_validation_accepts_valid_contract() {
        let contract = valid_contract();
        assert!(validate_contract(&contract).is_ok());
    }

    #[test]
    fn contract_validation_rejects_missing_task_id_with_field_error() {
        let mut contract = valid_contract();
        contract.task_id = "".to_string();

        let err = validate_contract(&contract).expect_err("expected validation failure");
        assert!(err.issues.iter().any(|i| i.field == "task_id"));
    }

    #[test]
    fn parse_and_validate_contract_reports_json_error() {
        let err = parse_and_validate_contract("{not-json}").expect_err("expected json error");
        assert!(err.issues.iter().any(|i| i.field == "contract"));
    }

    #[test]
    fn contract_without_sandbox_permissions_deserializes() {
        let json = r#"{
            "task_id": "T-003",
            "req_ids": ["REQ-002"],
            "wave": 1,
            "file_policy": "default-deny",
            "allowed_files": ["src/engine.rs"],
            "forbidden_files": [],
            "allowed_commands": [["cargo","test"]],
            "forbidden_commands": [],
            "commit_format": "^T-003:",
            "test_command": "cargo test",
            "task_timeout_sec": 600,
            "done_when": "ok",
            "reality_test": "ok"
        }"#;
        let parsed: ContractFields = serde_json::from_str(json).expect("backward-compat parse");
        assert!(parsed.sandbox_permissions.is_none());
    }

    #[test]
    fn contract_with_sandbox_permissions_deserializes() {
        let json = r#"{
            "task_id": "T-003",
            "req_ids": ["REQ-002"],
            "wave": 1,
            "file_policy": "default-deny",
            "allowed_files": ["src/engine.rs"],
            "forbidden_files": [],
            "allowed_commands": [["cargo","test"]],
            "forbidden_commands": [],
            "commit_format": "^T-003:",
            "test_command": "cargo test",
            "task_timeout_sec": 600,
            "done_when": "ok",
            "reality_test": "ok",
            "sandbox_permissions": ["network-full-access"]
        }"#;
        let parsed: ContractFields = serde_json::from_str(json).expect("sandbox perms parse");
        assert_eq!(
            parsed.sandbox_permissions.as_deref(),
            Some(&["network-full-access".to_string()][..])
        );
    }

    #[test]
    fn dispatch_worktree_request_accepts_complete_contract_fields() {
        let req = DispatchCodexWorktreeRequest {
            project_root: Some("/tmp/project".to_string()),
            sha: "abc123".to_string(),
            briefing_content: "# briefing".to_string(),
            contract_fields: valid_contract(),
            keep_failed_worktree: Some(false),
        };

        assert_eq!(req.contract_fields.task_id, "T-003");
    }
}
