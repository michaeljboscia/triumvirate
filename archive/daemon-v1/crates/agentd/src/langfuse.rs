use serde::Deserialize;
use serde_json::json;
use tracing::warn;
use triumvirate_proto::AgentId;

#[derive(Debug, Clone, Deserialize)]
pub struct LangfuseConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default)]
    pub public_key: String,
    #[serde(default)]
    pub secret_key: String,
}

impl Default for LangfuseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_host(),
            public_key: String::new(),
            secret_key: String::new(),
        }
    }
}

#[derive(Clone)]
pub struct LangfuseClient {
    cfg: LangfuseConfig,
    http: reqwest::Client,
}

impl LangfuseClient {
    pub fn new(cfg: LangfuseConfig) -> Self {
        Self {
            cfg,
            http: reqwest::Client::new(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.cfg.enabled && !self.cfg.public_key.is_empty() && !self.cfg.secret_key.is_empty()
    }

    pub fn record_turn(
        &self,
        session_id: &str,
        agent: AgentId,
        input_tokens: u64,
        output_tokens: u64,
        estimated_cost_usd: f64,
        latency_ms: u64,
    ) {
        if !self.is_enabled() {
            return;
        }

        let url = format!("{}/api/public/ingestion", self.cfg.host.trim_end_matches('/'));
        let auth = format!("{}:{}", self.cfg.public_key, self.cfg.secret_key);
        let auth_b64 = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(auth)
        };

        let payload = json!({
            "batch": [
                {
                    "id": format!("triumvirate-{}-{}", agent, uuid::Uuid::new_v4()),
                    "type": "generation-create",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "body": {
                        "name": format!("{} turn", agent),
                        "model": format!("{}", agent),
                        "sessionId": session_id,
                        "usage": {
                            "input": input_tokens,
                            "output": output_tokens,
                            "total": input_tokens + output_tokens
                        },
                        "metadata": {
                            "agent": format!("{}", agent),
                            "estimated_cost_usd": estimated_cost_usd,
                            "latency_ms": latency_ms
                        }
                    }
                }
            ]
        });

        let client = self.http.clone();
        tokio::spawn(async move {
            let result = client
                .post(url)
                .header("Authorization", format!("Basic {auth_b64}"))
                .json(&payload)
                .send()
                .await;
            if let Err(e) = result {
                warn!(error = %e, "langfuse ingestion failed");
            }
        });
    }
}

fn default_host() -> String {
    "https://langfuse.e5btools.com".to_string()
}
