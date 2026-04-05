use clap::{Parser, Subcommand};
use axum::{Json as AxumJson, Router, routing::get};
use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use std::{collections::HashMap, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::Mutex,
    time::{Duration, sleep, timeout},
};

#[derive(Debug, Parser)]
#[command(name = "triumvirate")]
#[command(about = "Triumvirate v2 daemon + MCP bridge binary")]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Run the MCP stdio bridge.
    Mcp,
    /// Run the long-lived daemon (stub in Increment 1a).
    Daemon,
}

#[derive(Debug, Clone)]
struct McpBridge {
    tool_router: ToolRouter<Self>,
    sessions: Arc<Mutex<HashMap<String, SessionState>>>,
}

impl McpBridge {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Debug, Clone)]
struct SessionState {
    agent: String,
    history: Vec<String>,
}

#[derive(Debug, Clone, serde::Deserialize, JsonSchema)]
struct AskAgentRequest {
    agent: String,
    message: String,
    cwd: Option<String>,
    repo: Option<String>,
    branch: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
struct LifecycleEvent {
    state: String,
    detail: String,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
struct AskAgentResponse {
    agent: String,
    response: String,
    lifecycle: Vec<LifecycleEvent>,
}

#[derive(Debug, Clone, serde::Deserialize, JsonSchema)]
struct AskTwinsRequest {
    message: String,
    cwd: Option<String>,
    repo: Option<String>,
    branch: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
struct AgentResult {
    agent: String,
    response: String,
    prompt_sent: String,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
struct AskTwinsResponse {
    results: Vec<AgentResult>,
    lifecycle: Vec<LifecycleEvent>,
}

#[derive(Debug, Clone, serde::Deserialize, JsonSchema)]
struct SpawnSessionRequest {
    agent: String,
    name: String,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
struct SessionInfo {
    name: String,
    agent: String,
    turns: usize,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
struct SessionListResponse {
    sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, serde::Serialize, JsonSchema)]
struct StatusResponse {
    daemon_mode: String,
    active_sessions: usize,
    supported_agents: Vec<String>,
    pending_fallbacks: usize,
}

#[derive(Debug, Clone, serde::Deserialize, JsonSchema)]
struct AskSessionRequest {
    name: String,
    message: String,
}

#[derive(Debug, Clone, serde::Deserialize, JsonSchema)]
struct DismissSessionRequest {
    name: String,
}

#[tool_router]
impl McpBridge {
    #[tool(description = "Health check tool for MCP connectivity")]
    async fn ping(&self) -> String {
        "pong".to_string()
    }

    #[tool(description = "Send a task to a specific agent (Increment 1b supports gemini mock path).")]
    async fn ask_agent(
        &self,
        Parameters(req): Parameters<AskAgentRequest>,
    ) -> Result<Json<AskAgentResponse>, String> {
        if req.agent.to_lowercase() != "gemini" {
            return Err("Increment 1b currently supports only agent='gemini'".to_string());
        }

        // Increment 1b emits lifecycle states in-band so Claude can render user-visible progress
        // before we wire native MCP progress notifications in a later increment.
        let mut lifecycle = vec![LifecycleEvent {
            state: "SPAWNED".to_string(),
            detail: format!(
                "Started Gemini connector{}{}{}",
                req.cwd
                    .as_ref()
                    .map(|v| format!(" cwd={v}"))
                    .unwrap_or_default(),
                req.repo
                    .as_ref()
                    .map(|v| format!(" repo={v}"))
                    .unwrap_or_default(),
                req.branch
                    .as_ref()
                    .map(|v| format!(" branch={v}"))
                    .unwrap_or_default()
            ),
        }];

        lifecycle.push(LifecycleEvent {
            state: "WORKING".to_string(),
            detail: "Gemini is processing request".to_string(),
        });

        let backoffs = [Duration::from_millis(250), Duration::from_secs(1), Duration::from_secs(2)];
        let mut last_err: Option<String> = None;

        for (idx, backoff) in backoffs.iter().enumerate() {
            match run_gemini_mock(&req.message).await {
                Ok(response) => {
                    lifecycle.push(LifecycleEvent {
                        state: "DONE".to_string(),
                        detail: format!("Gemini responded on attempt {}", idx + 1),
                    });
                    return Ok(Json(AskAgentResponse {
                        agent: "gemini".to_string(),
                        response,
                        lifecycle,
                    }));
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("timed out") {
                        lifecycle.push(LifecycleEvent {
                            state: "TIMEOUT".to_string(),
                            detail: format!("Gemini timed out on attempt {}", idx + 1),
                        });
                    }
                    lifecycle.push(LifecycleEvent {
                        state: "RETRY".to_string(),
                        detail: format!("Retrying Gemini ({}/{}) after {}", idx + 1, backoffs.len(), msg),
                    });
                    last_err = Some(msg);
                    sleep(*backoff).await;
                }
            }
        }

        lifecycle.push(LifecycleEvent {
            state: "FAILED".to_string(),
            detail: format!(
                "Gemini failed after {} attempts",
                backoffs.len()
            ),
        });
        Err(format!(
            "ask_agent failed after lifecycle {:?}: {}",
            lifecycle.iter().map(|e| e.state.as_str()).collect::<Vec<_>>(),
            last_err.unwrap_or_else(|| "unknown error".to_string())
        ))
    }

    #[tool(description = "Fan out a request to Gemini and Codex in parallel with role-adapted prompts.")]
    async fn ask_twins(
        &self,
        Parameters(req): Parameters<AskTwinsRequest>,
    ) -> Result<Json<AskTwinsResponse>, String> {
        let gemini_prompt = format!(
            "[Gemini role: research/analysis]\nQuestion: {}\nContext: cwd={:?} repo={:?} branch={:?}",
            req.message, req.cwd, req.repo, req.branch
        );
        let codex_prompt = format!(
            "[Codex role: implementation/testing]\nQuestion: {}\nContext: cwd={:?} repo={:?} branch={:?}",
            req.message, req.cwd, req.repo, req.branch
        );

        let mut lifecycle = vec![
            LifecycleEvent {
                state: "SPAWNED".to_string(),
                detail: "Gemini request sent".to_string(),
            },
            LifecycleEvent {
                state: "SPAWNED".to_string(),
                detail: "Codex request sent".to_string(),
            },
            LifecycleEvent {
                state: "WORKING".to_string(),
                detail: "Gemini and Codex processing in parallel".to_string(),
            },
        ];

        let gemini_fut = run_named_agent("gemini", &gemini_prompt);
        let codex_fut = run_named_agent("codex", &codex_prompt);
        let (gemini_out, codex_out) = tokio::join!(gemini_fut, codex_fut);

        let gemini_out = gemini_out.map_err(|e| format!("Gemini failed: {e}"))?;
        lifecycle.push(LifecycleEvent {
            state: "DONE".to_string(),
            detail: "Gemini responded".to_string(),
        });

        let codex_out = codex_out.map_err(|e| format!("Codex failed: {e}"))?;
        lifecycle.push(LifecycleEvent {
            state: "DONE".to_string(),
            detail: "Codex responded".to_string(),
        });

        Ok(Json(AskTwinsResponse {
            results: vec![
                AgentResult {
                    agent: "gemini".to_string(),
                    response: gemini_out,
                    prompt_sent: gemini_prompt,
                },
                AgentResult {
                    agent: "codex".to_string(),
                    response: codex_out,
                    prompt_sent: codex_prompt,
                },
            ],
            lifecycle,
        }))
    }

    #[tool(description = "Create a persistent named session for an agent.")]
    async fn spawn_session(
        &self,
        Parameters(req): Parameters<SpawnSessionRequest>,
    ) -> Result<String, String> {
        let agent = req.agent.to_lowercase();
        if agent != "gemini" && agent != "codex" {
            return Err("spawn_session supports only 'gemini' or 'codex'".to_string());
        }

        let mut sessions = self.sessions.lock().await;
        sessions.insert(
            req.name.clone(),
            SessionState {
                agent: agent.clone(),
                history: Vec::new(),
            },
        );
        Ok(format!("session '{}' spawned for {}", req.name, agent))
    }

    #[tool(description = "Ask within a named persistent session.")]
    async fn ask_session(
        &self,
        Parameters(req): Parameters<AskSessionRequest>,
    ) -> Result<String, String> {
        let (agent, prompt) = {
            let mut sessions = self.sessions.lock().await;
            let state = sessions
                .get_mut(&req.name)
                .ok_or_else(|| format!("session '{}' not found", req.name))?;

            let context = if state.history.is_empty() {
                String::new()
            } else {
                format!("Previous turns:\n{}\n\n", state.history.join("\n"))
            };
            let prompt = format!("{context}New user message:\n{}", req.message);
            state.history.push(req.message.clone());
            (state.agent.clone(), prompt)
        };

        let response = run_named_agent(&agent, &prompt)
            .await
            .map_err(|e| format!("ask_session failed: {e}"))?;

        let mut sessions = self.sessions.lock().await;
        if let Some(state) = sessions.get_mut(&req.name) {
            state.history.push(format!("assistant: {response}"));
        }

        Ok(response)
    }

    #[tool(description = "Dismiss a named session.")]
    async fn dismiss_session(
        &self,
        Parameters(req): Parameters<DismissSessionRequest>,
    ) -> Result<String, String> {
        let mut sessions = self.sessions.lock().await;
        match sessions.remove(&req.name) {
            Some(_) => Ok(format!("session '{}' dismissed", req.name)),
            None => Err(format!("session '{}' not found", req.name)),
        }
    }

    #[tool(description = "List active sessions.")]
    async fn list_sessions(&self) -> Json<SessionListResponse> {
        let sessions = self.sessions.lock().await;
        let mut out = sessions
            .iter()
            .map(|(name, s)| SessionInfo {
                name: name.clone(),
                agent: s.agent.clone(),
                turns: s.history.len(),
            })
            .collect::<Vec<_>>();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Json(SessionListResponse { sessions: out })
    }

    #[tool(description = "Get current system status snapshot.")]
    async fn get_status(&self) -> Json<StatusResponse> {
        let sessions = self.sessions.lock().await;
        Json(StatusResponse {
            daemon_mode: "incremental-dev".to_string(),
            active_sessions: sessions.len(),
            supported_agents: vec!["gemini".to_string(), "codex".to_string()],
            pending_fallbacks: 0,
        })
    }
}

fn gemini_command() -> (String, Vec<String>) {
    let bin = std::env::var("TRIUMVIRATE_GEMINI_BIN").unwrap_or_else(|_| "mock-gemini".to_string());
    let args = std::env::var("TRIUMVIRATE_GEMINI_ARGS")
        .map(|v| v.split_whitespace().map(ToString::to_string).collect())
        .unwrap_or_else(|_| Vec::new());
    (bin, args)
}

async fn run_gemini_mock(message: &str) -> anyhow::Result<String> {
    let (bin, args) = gemini_command();
    run_agent_process(&bin, &args, message).await
}

fn codex_command() -> (String, Vec<String>) {
    let bin = std::env::var("TRIUMVIRATE_CODEX_BIN").unwrap_or_else(|_| "mock-codex".to_string());
    let args = std::env::var("TRIUMVIRATE_CODEX_ARGS")
        .map(|v| v.split_whitespace().map(ToString::to_string).collect())
        .unwrap_or_else(|_| Vec::new());
    (bin, args)
}

async fn run_named_agent(agent: &str, message: &str) -> anyhow::Result<String> {
    match agent {
        "gemini" => {
            let (bin, args) = gemini_command();
            run_agent_process(&bin, &args, message).await
        }
        "codex" => {
            let (bin, args) = codex_command();
            run_agent_process(&bin, &args, message).await
        }
        _ => anyhow::bail!("unsupported agent: {agent}"),
    }
}

async fn run_agent_process(bin: &str, args: &[String], message: &str) -> anyhow::Result<String> {
    let mut child = Command::new(&bin)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await?;
        stdin.flush().await?;
    }

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("gemini stdout missing"))?;
    let mut lines = BufReader::new(stdout).lines();

    // The mock connector may emit readiness notifications before the final result; scan until we
    // find a JSON-RPC payload with result.text.
    let read_result = timeout(Duration::from_secs(5), async {
        while let Some(line) = lines.next_line().await? {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(text) = json
                    .get("result")
                    .and_then(|r| r.get("text"))
                    .and_then(|t| t.as_str())
                {
                    return Ok(text.to_string());
                }
            }
        }
        Err(anyhow::anyhow!("no result.text message from gemini connector"))
    })
    .await;

    let response = match read_result {
        Ok(result) => result?,
        Err(_) => anyhow::bail!("gemini connector timed out"),
    };

    let _ = child.kill().await;
    let _ = child.wait().await;

    Ok(response)
}

#[tool_handler]
impl ServerHandler for McpBridge {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Triumvirate MCP bridge. Use `ping` to verify connectivity.")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "triumvirate=info".into()),
        )
        .with_target(false)
        .init();

    match Cli::parse().command {
        CliCommand::Mcp => {
            McpBridge::new().serve(stdio()).await?.waiting().await?;
        }
        CliCommand::Daemon => {
            run_daemon().await?;
        }
    }

    Ok(())
}

async fn run_daemon() -> anyhow::Result<()> {
    async fn health() -> AxumJson<serde_json::Value> {
        AxumJson(serde_json::json!({
            "status": "ok",
            "service": "triumvirate-daemon-v2",
            "mode": "incremental-dev"
        }))
    }

    let app = Router::new().route("/health", get(health));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{ClientHandler, model::ClientInfo};
    use rmcp::model::CallToolRequestParams;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::{Mutex, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[derive(Debug, Clone, Default)]
    struct NoopClient;

    impl ClientHandler for NoopClient {
        fn get_info(&self) -> ClientInfo {
            ClientInfo::default()
        }
    }

    #[tokio::test]
    async fn ping_tool_returns_pong() -> anyhow::Result<()> {
        let (server_transport, client_transport) = tokio::io::duplex(4096);

        let server_handle = tokio::spawn(async move {
            McpBridge::new().serve(server_transport).await?.waiting().await?;
            anyhow::Ok(())
        });

        let client = NoopClient.serve(client_transport).await?;
        let result = client.call_tool(CallToolRequestParams::new("ping")).await?;
        let text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("");

        assert_eq!(text, "pong");

        client.cancel().await?;
        server_handle.await??;
        Ok(())
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn write_mock_gemini_script() -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mock-gemini-{now}.sh"));
        let script = r#"#!/bin/sh
echo '{"jsonrpc":"2.0","method":"session/ready","params":{"text":"mock ready"}}'
IFS= read -r _line
echo '{"jsonrpc":"2.0","id":1,"result":{"text":"mock-gemini received: test message"}}'
"#;
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_mock_agent_script(name: &str, delay_s: f32) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mock-{name}-{now}.sh"));
        let script = format!(
            "#!/bin/sh\n\
echo '{{\"jsonrpc\":\"2.0\",\"method\":\"session/ready\",\"params\":{{\"text\":\"{name} ready\"}}}}'\n\
IFS= read -r _line\n\
sleep {delay}\n\
echo '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"text\":\"{name} done\"}}}}'\n",
            name = name,
            delay = delay_s
        );
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    fn write_retry_agent_script(name: &str) -> anyhow::Result<PathBuf> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("mock-{name}-retry-{now}.sh"));
        let state_path = std::env::temp_dir().join(format!("mock-{name}-retry-state-{now}.txt"));
        let script = format!(
            "#!/bin/sh\n\
state_file=\"{state_file}\"\n\
count=0\n\
if [ -f \"$state_file\" ]; then count=$(cat \"$state_file\"); fi\n\
count=$((count+1))\n\
echo \"$count\" > \"$state_file\"\n\
IFS= read -r _line\n\
if [ \"$count\" -eq 1 ]; then\n\
  echo '{{\"jsonrpc\":\"2.0\",\"method\":\"session/ready\",\"params\":{{\"text\":\"{name} attempt1 no result\"}}}}'\n\
  exit 0\n\
fi\n\
echo '{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"text\":\"{name} recovered on retry\"}}}}'\n",
            state_file = state_path.display(),
            name = name
        );
        fs::write(&path, script)?;
        let mut perms = fs::metadata(&path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms)?;
        Ok(path)
    }

    #[tokio::test]
    async fn ask_agent_gemini_happy_path_returns_lifecycle() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let script_path = write_mock_gemini_script()?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            McpBridge::new().serve(server_transport).await?.waiting().await?;
            anyhow::Ok(())
        });

        let client = NoopClient.serve(client_transport).await?;

        let args = serde_json::json!({
            "agent": "gemini",
            "message": "test message",
            "cwd": "/tmp/project",
            "repo": "triumvirate",
            "branch": "feat/mcp-first"
        });
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_agent")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let raw_text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();

        assert!(raw_text.contains("mock-gemini received: test message"));
        assert!(raw_text.contains("SPAWNED"));
        assert!(raw_text.contains("WORKING"));
        assert!(raw_text.contains("DONE"));

        client.cancel().await?;
        server_handle.await??;

        let _ = fs::remove_file(script_path);
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        Ok(())
    }

    #[tokio::test]
    async fn ask_twins_parallel_and_role_adapted() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let gemini_script = write_mock_agent_script("gemini", 1.0)?;
        let codex_script = write_mock_agent_script("codex", 0.2)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", gemini_script.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::set_var("TRIUMVIRATE_CODEX_BIN", codex_script.as_os_str());
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
        }

        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            McpBridge::new().serve(server_transport).await?.waiting().await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let args = serde_json::json!({
            "message": "Add auth module",
            "cwd": "/tmp/project",
            "repo": "triumvirate",
            "branch": "feat/mcp-first"
        });

        let start = std::time::Instant::now();
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_twins")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await?;
        let elapsed = start.elapsed();

        let raw_text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();

        assert!(elapsed < Duration::from_secs(2));
        assert!(raw_text.contains("gemini done"));
        assert!(raw_text.contains("codex done"));
        assert!(raw_text.contains("[Gemini role: research/analysis]"));
        assert!(raw_text.contains("[Codex role: implementation/testing]"));

        client.cancel().await?;
        server_handle.await??;

        let _ = fs::remove_file(gemini_script);
        let _ = fs::remove_file(codex_script);
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
            std::env::remove_var("TRIUMVIRATE_CODEX_BIN");
            std::env::remove_var("TRIUMVIRATE_CODEX_ARGS");
        }
        Ok(())
    }

    #[tokio::test]
    async fn ask_agent_retries_and_recovers() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let script_path = write_retry_agent_script("gemini")?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", script_path.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            McpBridge::new().serve(server_transport).await?.waiting().await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let args = serde_json::json!({
            "agent": "gemini",
            "message": "test retry",
        });
        let result = client
            .call_tool(
                CallToolRequestParams::new("ask_agent")
                    .with_arguments(args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let raw_text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(raw_text.contains("gemini recovered on retry"));
        assert!(raw_text.contains("RETRY"));
        assert!(raw_text.contains("DONE"));

        client.cancel().await?;
        server_handle.await??;
        let _ = fs::remove_file(script_path);
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        Ok(())
    }

    #[tokio::test]
    async fn session_lifecycle_spawn_ask_list_dismiss() -> anyhow::Result<()> {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let gemini_script = write_mock_agent_script("gemini", 0.0)?;
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::set_var("TRIUMVIRATE_GEMINI_BIN", gemini_script.as_os_str());
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }

        let (server_transport, client_transport) = tokio::io::duplex(8192);
        let server_handle = tokio::spawn(async move {
            McpBridge::new().serve(server_transport).await?.waiting().await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let spawn_args = serde_json::json!({
            "agent": "gemini",
            "name": "my-research"
        });
        let _spawn = client
            .call_tool(
                CallToolRequestParams::new("spawn_session")
                    .with_arguments(spawn_args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let ask_args = serde_json::json!({
            "name": "my-research",
            "message": "what is jwt?"
        });
        let ask = client
            .call_tool(
                CallToolRequestParams::new("ask_session")
                    .with_arguments(ask_args.as_object().cloned().unwrap_or_default()),
            )
            .await?;
        let ask_text = ask
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(ask_text.contains("gemini done"));

        let list = client
            .call_tool(CallToolRequestParams::new("list_sessions"))
            .await?;
        let list_text = list
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(list_text.contains("my-research"));

        let dismiss_args = serde_json::json!({
            "name": "my-research"
        });
        let _dismiss = client
            .call_tool(
                CallToolRequestParams::new("dismiss_session")
                    .with_arguments(dismiss_args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        client.cancel().await?;
        server_handle.await??;

        let _ = fs::remove_file(gemini_script);
        // SAFETY: test controls env var lifecycle under lock.
        unsafe {
            std::env::remove_var("TRIUMVIRATE_GEMINI_BIN");
            std::env::remove_var("TRIUMVIRATE_GEMINI_ARGS");
        }
        Ok(())
    }

    #[tokio::test]
    async fn get_status_reports_active_sessions() -> anyhow::Result<()> {
        let (server_transport, client_transport) = tokio::io::duplex(4096);
        let server_handle = tokio::spawn(async move {
            McpBridge::new().serve(server_transport).await?.waiting().await?;
            anyhow::Ok(())
        });
        let client = NoopClient.serve(client_transport).await?;

        let spawn_args = serde_json::json!({
            "agent": "gemini",
            "name": "status-session"
        });
        let _ = client
            .call_tool(
                CallToolRequestParams::new("spawn_session")
                    .with_arguments(spawn_args.as_object().cloned().unwrap_or_default()),
            )
            .await?;

        let status = client
            .call_tool(CallToolRequestParams::new("get_status"))
            .await?;
        let status_text = status
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        assert!(status_text.contains("\"active_sessions\":1"));
        assert!(status_text.contains("\"supported_agents\":[\"gemini\",\"codex\"]"));

        client.cancel().await?;
        server_handle.await??;
        Ok(())
    }
}
