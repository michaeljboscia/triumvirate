#![allow(dead_code)]

use std::collections::HashMap;

use anyhow::Context;
use async_trait::async_trait;
use triumvirate_proto::AgentId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Cli,
    Api,
}

#[derive(Debug, Clone)]
pub struct ApiBackendConfig {
    pub endpoint: String,
    pub api_key_env: String,
}

#[derive(Debug, Clone, Default)]
pub struct BackendRegistry {
    kinds: HashMap<AgentId, BackendKind>,
    api: HashMap<AgentId, ApiBackendConfig>,
}

impl BackendRegistry {
    pub fn with_defaults() -> Self {
        let mut kinds = HashMap::new();
        kinds.insert(AgentId::Claude, BackendKind::Cli);
        kinds.insert(AgentId::Gemini, BackendKind::Cli);
        kinds.insert(AgentId::Codex, BackendKind::Cli);
        Self {
            kinds,
            api: HashMap::new(),
        }
    }

    #[allow(dead_code)]
    pub fn set_backend(&mut self, agent: AgentId, kind: BackendKind) {
        self.kinds.insert(agent, kind);
    }

    #[allow(dead_code)]
    pub fn set_api_config(&mut self, agent: AgentId, config: ApiBackendConfig) {
        self.api.insert(agent, config);
    }

    #[allow(dead_code)]
    pub fn backend_kind(&self, agent: AgentId) -> BackendKind {
        self.kinds.get(&agent).copied().unwrap_or(BackendKind::Cli)
    }
}

#[async_trait]
pub trait AgentBackend: Send + Sync {
    async fn send_prompt(&self, agent: AgentId, prompt: &str) -> anyhow::Result<String>;
}

/// HTTP API backend placeholder.
///
/// This compiles as an abstraction boundary while CLI backends remain default.
pub struct ApiBackend;

#[async_trait]
impl AgentBackend for ApiBackend {
    async fn send_prompt(&self, _agent: AgentId, _prompt: &str) -> anyhow::Result<String> {
        Err(anyhow::anyhow!(
            "api backend not enabled yet; configure provider endpoint + key and implement transport"
        ))
        .context("api backend stub")
    }
}
