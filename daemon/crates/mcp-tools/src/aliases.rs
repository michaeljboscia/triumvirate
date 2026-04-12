//! Backwards-compatibility alias parameter mappings for legacy TS inter-agent tool names.
//!
//! This module defines TS-side input shapes and conversion functions into Rust-side
//! request-like structs for Wave 0 contract work. Tool router registration and invocation
//! wiring are handled later in Wave 3 (T-011).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasMappingError {
    InvalidTarget(String),
    MissingRequired(&'static str),
    InvalidDaemonId(String),
}

impl std::fmt::Display for AliasMappingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTarget(t) => {
                write!(f, "invalid target '{t}' — expected 'gemini' or 'codex'")
            }
            Self::MissingRequired(field) => write!(f, "missing required field '{field}'"),
            Self::InvalidDaemonId(id) => write!(
                f,
                "invalid daemon_id '{id}' — must be a non-empty session name"
            ),
        }
    }
}

impl std::error::Error for AliasMappingError {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpawnDaemonParams {
    pub target: String,
    pub session_name: Option<String>,
    pub cwd: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskDaemonParams {
    pub daemon_id: String,
    pub question: String,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DismissDaemonParams {
    pub daemon_id: String,
    pub hard: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListDaemonsParams {
    pub target: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SendMessageParams {
    pub target: String,
    pub request_type: String,
    pub question: String,
    pub context: Option<String>,
    pub cwd: Option<String>,
    pub session_log: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetResponseParams {
    pub job_id: String,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListJobsParams {
    pub target: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriteScratchpadParams {
    pub topic: String,
    pub content: String,
    pub cwd: Option<String>,
    pub owner: Option<String>,
    pub daemon_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListScratchpadParams {
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CodeReviewParams {
    pub cwd: Option<String>,
    pub uncommitted: Option<bool>,
    pub base_branch: Option<String>,
    pub commit_sha: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSessionRequestLike {
    pub agent: String,
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskSessionRequestLike {
    pub name: String,
    pub message: String,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DismissSessionRequestLike {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListSessionsRequestLike {
    pub target: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetResponseDeprecationShim {
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetStatusRequestLike {
    pub target: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchpadWriteRequestLike {
    pub filename_stem: String,
    pub content: String,
    pub cwd: Option<String>,
    pub owner: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchpadListRequestLike {
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewRequestLike {
    pub cwd: Option<String>,
    pub uncommitted: Option<bool>,
    pub base_branch: Option<String>,
    pub commit_sha: Option<String>,
    pub timeout_ms: Option<u64>,
}

fn map_target_to_agent(target: String) -> Result<String, AliasMappingError> {
    match target.as_str() {
        "gemini" => Ok("gemini".to_string()),
        "codex" => Ok("codex".to_string()),
        _ => Err(AliasMappingError::InvalidTarget(target)),
    }
}

fn validate_optional_target(target: Option<String>) -> Result<Option<String>, AliasMappingError> {
    match target {
        Some(value) => map_target_to_agent(value).map(Some),
        None => Ok(None),
    }
}

fn validate_daemon_id(daemon_id: String) -> Result<String, AliasMappingError> {
    if daemon_id.starts_with("gd_") || daemon_id.starts_with("cd_") {
        Ok(daemon_id)
    } else {
        Err(AliasMappingError::InvalidDaemonId(daemon_id))
    }
}

pub fn map_spawn_daemon_params(
    p: SpawnDaemonParams,
) -> Result<SpawnSessionRequestLike, AliasMappingError> {
    Ok(SpawnSessionRequestLike {
        agent: map_target_to_agent(p.target)?,
        name: p.session_name,
        cwd: p.cwd,
        timeout_ms: p.timeout_ms,
    })
}

pub fn map_ask_daemon_params(p: AskDaemonParams) -> Result<AskSessionRequestLike, AliasMappingError> {
    Ok(AskSessionRequestLike {
        name: validate_daemon_id(p.daemon_id)?,
        message: p.question,
        timeout_ms: p.timeout_ms,
    })
}

pub fn map_dismiss_daemon_params(
    p: DismissDaemonParams,
) -> Result<DismissSessionRequestLike, AliasMappingError> {
    if p.hard.is_some() {
        tracing::warn!(
            "alias dismiss_daemon: 'hard' param dropped — Rust does not support it"
        );
    }

    Ok(DismissSessionRequestLike {
        name: validate_daemon_id(p.daemon_id)?,
    })
}

pub fn map_list_daemons_params(
    p: ListDaemonsParams,
) -> Result<ListSessionsRequestLike, AliasMappingError> {
    Ok(ListSessionsRequestLike {
        target: validate_optional_target(p.target)?,
        cwd: p.cwd,
    })
}

pub fn map_send_message_params(
    p: SendMessageParams,
) -> Result<AskSessionRequestLike, AliasMappingError> {
    // Dropped in Wave 0 mapping: request_type, context, cwd, session_log.
    let _ = (&p.request_type, &p.context, &p.cwd, &p.session_log);

    Ok(AskSessionRequestLike {
        name: map_target_to_agent(p.target)?,
        message: p.question,
        timeout_ms: p.timeout_ms,
    })
}

pub fn map_get_response_params(
    p: GetResponseParams,
) -> Result<GetResponseDeprecationShim, AliasMappingError> {
    // Deprecated alias shim: legacy job-based fields are intentionally ignored.
    let _ = (&p.job_id, &p.timeout_ms);

    Ok(GetResponseDeprecationShim {
        message: "get_response is deprecated in 3.1.0 — use ask_session directly. The async job queue was removed.".to_string(),
    })
}

pub fn map_list_jobs_params(p: ListJobsParams) -> Result<GetStatusRequestLike, AliasMappingError> {
    Ok(GetStatusRequestLike {
        target: validate_optional_target(p.target)?,
        cwd: p.cwd,
    })
}

pub fn map_write_scratchpad_params(
    p: WriteScratchpadParams,
) -> Result<ScratchpadWriteRequestLike, AliasMappingError> {
    if p.topic.trim().is_empty() {
        return Err(AliasMappingError::MissingRequired("topic"));
    }
    if p.content.trim().is_empty() {
        return Err(AliasMappingError::MissingRequired("content"));
    }

    let owner = if let Some(ref daemon_id) = p.daemon_id {
        if let Some(suffix) = daemon_id.strip_prefix("gd_") {
            format!("gemini-{suffix}")
        } else if let Some(suffix) = daemon_id.strip_prefix("cd_") {
            format!("codex-{suffix}")
        } else if let Some(explicit_owner) = p.owner.clone() {
            explicit_owner
        } else {
            "inter-agent".to_string()
        }
    } else if let Some(explicit_owner) = p.owner {
        explicit_owner
    } else {
        "inter-agent".to_string()
    };

    Ok(ScratchpadWriteRequestLike {
        filename_stem: p.topic,
        content: p.content,
        cwd: p.cwd,
        owner,
    })
}

pub fn map_list_scratchpad_params(
    p: ListScratchpadParams,
) -> Result<ScratchpadListRequestLike, AliasMappingError> {
    Ok(ScratchpadListRequestLike { cwd: p.cwd })
}

pub fn map_code_review_params(p: CodeReviewParams) -> Result<ReviewRequestLike, AliasMappingError> {
    Ok(ReviewRequestLike {
        cwd: p.cwd,
        uncommitted: p.uncommitted,
        base_branch: p.base_branch,
        commit_sha: p.commit_sha,
        timeout_ms: p.timeout_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u_al_01_spawn_daemon_gemini_maps_to_agent_gemini() {
        let p = SpawnDaemonParams {
            target: "gemini".into(),
            session_name: Some("x".into()),
            cwd: None,
            timeout_ms: None,
        };
        let out = map_spawn_daemon_params(p).unwrap();
        assert_eq!(out.agent, "gemini");
        assert_eq!(out.name, Some("x".into()));
    }

    #[test]
    fn u_al_02_spawn_daemon_codex_maps_to_agent_codex() {
        let p = SpawnDaemonParams {
            target: "codex".into(),
            session_name: None,
            cwd: None,
            timeout_ms: None,
        };
        let out = map_spawn_daemon_params(p).unwrap();
        assert_eq!(out.agent, "codex");
    }

    #[test]
    fn u_al_03_spawn_daemon_claude_rejected() {
        let p = SpawnDaemonParams {
            target: "claude".into(),
            session_name: None,
            cwd: None,
            timeout_ms: None,
        };
        let err = map_spawn_daemon_params(p).unwrap_err();
        assert!(matches!(err, AliasMappingError::InvalidTarget(_)));
    }

    #[test]
    fn u_al_04_spawn_daemon_empty_target_errors() {
        let p = SpawnDaemonParams {
            target: "".into(),
            session_name: None,
            cwd: None,
            timeout_ms: None,
        };
        let err = map_spawn_daemon_params(p).unwrap_err();
        assert!(matches!(err, AliasMappingError::InvalidTarget(t) if t.is_empty()));
    }

    #[test]
    fn u_al_05_ask_daemon_preserves_gd_prefix() {
        let p = AskDaemonParams {
            daemon_id: "gd_session_abc".into(),
            question: "hi".into(),
            timeout_ms: None,
        };
        let out = map_ask_daemon_params(p).unwrap();
        assert_eq!(out.name, "gd_session_abc");
        assert_eq!(out.message, "hi");
    }

    #[test]
    fn u_al_06_ask_daemon_preserves_cd_prefix() {
        let p = AskDaemonParams {
            daemon_id: "cd_session_xyz".into(),
            question: "hello".into(),
            timeout_ms: Some(50),
        };
        let out = map_ask_daemon_params(p).unwrap();
        assert_eq!(out.name, "cd_session_xyz");
        assert_eq!(out.message, "hello");
        assert_eq!(out.timeout_ms, Some(50));
    }

    #[test]
    fn u_al_07_ask_daemon_invalid_prefix_errors() {
        let p = AskDaemonParams {
            daemon_id: "xx_bad".into(),
            question: "hello".into(),
            timeout_ms: None,
        };
        let err = map_ask_daemon_params(p).unwrap_err();
        assert!(matches!(err, AliasMappingError::InvalidDaemonId(id) if id == "xx_bad"));
    }

    #[test]
    fn u_al_08_dismiss_daemon_drops_hard_param() {
        let p = DismissDaemonParams {
            daemon_id: "gd_session_42".into(),
            hard: Some(true),
        };
        let out = map_dismiss_daemon_params(p).unwrap();
        assert_eq!(out.name, "gd_session_42");
    }

    #[test]
    fn u_al_09_send_message_is_synchronous_mapping() {
        let p = SendMessageParams {
            target: "codex".into(),
            request_type: "question".into(),
            question: "echo test".into(),
            context: Some("ctx".into()),
            cwd: Some("/tmp/repo".into()),
            session_log: Some("session.log".into()),
            timeout_ms: Some(1234),
        };
        let out = map_send_message_params(p).unwrap();
        assert_eq!(out.name, "codex");
        assert_eq!(out.message, "echo test");
        assert_eq!(out.timeout_ms, Some(1234));
    }

    #[test]
    fn u_al_10_get_response_returns_deprecation_shim() {
        let p = GetResponseParams {
            job_id: "job_123".into(),
            timeout_ms: Some(1),
        };
        let out = map_get_response_params(p).unwrap();
        assert!(
            out.message.starts_with("get_response is deprecated in 3.1.0"),
            "unexpected deprecation message: {}",
            out.message
        );
    }

    #[test]
    fn u_al_11_list_jobs_target_filter_validation() {
        let valid = ListJobsParams {
            target: Some("gemini".into()),
            cwd: Some("/tmp".into()),
        };
        let out = map_list_jobs_params(valid).unwrap();
        assert_eq!(out.target, Some("gemini".into()));
        assert_eq!(out.cwd, Some("/tmp".into()));

        let invalid = ListJobsParams {
            target: Some("invalid".into()),
            cwd: None,
        };
        let err = map_list_jobs_params(invalid).unwrap_err();
        assert!(matches!(err, AliasMappingError::InvalidTarget(t) if t == "invalid"));
    }

    #[test]
    fn u_al_12_write_scratchpad_owner_from_gd_prefix() {
        let p = WriteScratchpadParams {
            topic: "notes".into(),
            content: "hello".into(),
            cwd: None,
            owner: None,
            daemon_id: Some("gd_session_xyz".into()),
        };
        let out = map_write_scratchpad_params(p).unwrap();
        assert_eq!(out.owner, "gemini-session_xyz");
    }

    #[test]
    fn u_al_13_write_scratchpad_owner_from_cd_prefix() {
        let p = WriteScratchpadParams {
            topic: "notes".into(),
            content: "hello".into(),
            cwd: None,
            owner: None,
            daemon_id: Some("cd_session_abc".into()),
        };
        let out = map_write_scratchpad_params(p).unwrap();
        assert_eq!(out.owner, "codex-session_abc");
    }

    #[test]
    fn u_al_14_write_scratchpad_explicit_owner() {
        let p = WriteScratchpadParams {
            topic: "notes".into(),
            content: "hello".into(),
            cwd: Some("/tmp/repo".into()),
            owner: Some("custom".into()),
            daemon_id: None,
        };
        let out = map_write_scratchpad_params(p).unwrap();
        assert_eq!(out.owner, "custom");
        assert_eq!(out.cwd, Some("/tmp/repo".into()));
    }

    #[test]
    fn u_al_15_write_scratchpad_default_owner() {
        let p = WriteScratchpadParams {
            topic: "notes".into(),
            content: "hello".into(),
            cwd: None,
            owner: None,
            daemon_id: None,
        };
        let out = map_write_scratchpad_params(p).unwrap();
        assert_eq!(out.owner, "inter-agent");
    }

    #[test]
    fn u_al_16_write_scratchpad_cwd_optional() {
        let p = WriteScratchpadParams {
            topic: "notes".into(),
            content: "hello".into(),
            cwd: None,
            owner: Some("custom".into()),
            daemon_id: None,
        };
        let out = map_write_scratchpad_params(p).unwrap();
        assert_eq!(out.cwd, None);
    }

    #[test]
    fn u_al_17_code_review_all_fields_passthrough() {
        let p = CodeReviewParams {
            cwd: Some("/tmp/repo".into()),
            uncommitted: Some(true),
            base_branch: Some("main".into()),
            commit_sha: Some("abc123".into()),
            timeout_ms: Some(5000),
        };
        let out = map_code_review_params(p).unwrap();
        assert_eq!(out.cwd, Some("/tmp/repo".into()));
        assert_eq!(out.uncommitted, Some(true));
        assert_eq!(out.base_branch, Some("main".into()));
        assert_eq!(out.commit_sha, Some("abc123".into()));
        assert_eq!(out.timeout_ms, Some(5000));
    }

    #[test]
    fn u_al_18_code_review_has_no_diff_or_context_fields() {
        let p = CodeReviewParams {
            cwd: None,
            uncommitted: None,
            base_branch: None,
            commit_sha: None,
            timeout_ms: None,
        };
        let out = map_code_review_params(p).unwrap();
        let ReviewRequestLike {
            cwd,
            uncommitted,
            base_branch,
            commit_sha,
            timeout_ms,
        } = out;
        assert_eq!(cwd, None);
        assert_eq!(uncommitted, None);
        assert_eq!(base_branch, None);
        assert_eq!(commit_sha, None);
        assert_eq!(timeout_ms, None);
    }

    #[test]
    fn u_al_19_list_scratchpad_cwd_passthrough() {
        let p = ListScratchpadParams {
            cwd: Some("/tmp/work".into()),
        };
        let out = map_list_scratchpad_params(p).unwrap();
        assert_eq!(out.cwd, Some("/tmp/work".into()));
    }

    #[test]
    fn u_al_20_all_alias_functions_return_expected_types() {
        fn assert_spawn_type(_: Result<SpawnSessionRequestLike, AliasMappingError>) {}
        fn assert_ask_type(_: Result<AskSessionRequestLike, AliasMappingError>) {}
        fn assert_dismiss_type(_: Result<DismissSessionRequestLike, AliasMappingError>) {}
        fn assert_list_sessions_type(_: Result<ListSessionsRequestLike, AliasMappingError>) {}
        fn assert_get_response_type(_: Result<GetResponseDeprecationShim, AliasMappingError>) {}
        fn assert_get_status_type(_: Result<GetStatusRequestLike, AliasMappingError>) {}
        fn assert_scratchpad_write_type(_: Result<ScratchpadWriteRequestLike, AliasMappingError>) {}
        fn assert_scratchpad_list_type(_: Result<ScratchpadListRequestLike, AliasMappingError>) {}
        fn assert_review_type(_: Result<ReviewRequestLike, AliasMappingError>) {}

        assert_spawn_type(map_spawn_daemon_params(SpawnDaemonParams {
            target: "gemini".into(),
            session_name: None,
            cwd: None,
            timeout_ms: None,
        }));
        assert_ask_type(map_ask_daemon_params(AskDaemonParams {
            daemon_id: "gd_ok".into(),
            question: "q".into(),
            timeout_ms: None,
        }));
        assert_dismiss_type(map_dismiss_daemon_params(DismissDaemonParams {
            daemon_id: "cd_ok".into(),
            hard: Some(false),
        }));
        assert_list_sessions_type(map_list_daemons_params(ListDaemonsParams {
            target: Some("codex".into()),
            cwd: None,
        }));
        assert_ask_type(map_send_message_params(SendMessageParams {
            target: "gemini".into(),
            request_type: "question".into(),
            question: "q".into(),
            context: None,
            cwd: None,
            session_log: None,
            timeout_ms: None,
        }));
        assert_get_response_type(map_get_response_params(GetResponseParams {
            job_id: "job".into(),
            timeout_ms: None,
        }));
        assert_get_status_type(map_list_jobs_params(ListJobsParams {
            target: None,
            cwd: None,
        }));
        assert_scratchpad_write_type(map_write_scratchpad_params(WriteScratchpadParams {
            topic: "topic".into(),
            content: "content".into(),
            cwd: None,
            owner: None,
            daemon_id: None,
        }));
        assert_scratchpad_list_type(map_list_scratchpad_params(ListScratchpadParams { cwd: None }));
        assert_review_type(map_code_review_params(CodeReviewParams {
            cwd: None,
            uncommitted: None,
            base_branch: None,
            commit_sha: None,
            timeout_ms: None,
        }));
    }
}
