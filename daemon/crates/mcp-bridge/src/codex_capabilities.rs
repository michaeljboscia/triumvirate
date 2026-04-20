//! Runtime probe of the `codex` CLI surface.
//!
//! The daemon historically hard-coded the shape of the codex CLI (subcommands,
//! flag combinations). Each upstream codex release that renamed or relocated a
//! subcommand or made two flags mutually exclusive silently broke the daemon
//! with a status-1 exit inside the agent subprocess. The symptom was always
//! the same: `agent.outcome: "failure"`, `tokens: 0`, dead-drop ticket.
//!
//! This module probes the actual binary at daemon startup and exposes the
//! capabilities that `agent_exec` needs in order to make safe flag-injection
//! decisions. The probe is best-effort: if it fails, we fall back to the old
//! behaviour (no worse than before).

use std::sync::OnceLock;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;
use tracing::{info, warn};

/// Everything the daemon needs to know about the codex binary in this process.
#[derive(Debug, Clone)]
pub struct CodexCapabilities {
    /// Reported version string from `codex --version`, e.g. `"codex-cli 0.121.0"`.
    pub version: String,
    /// Whether `codex exec --help` succeeded (subcommand exists).
    pub has_exec: bool,
    /// Whether `codex app-server` still starts a JSON-RPC server over stdio
    /// (old shape). `false` in 0.121+ where `app-server` became a tooling
    /// namespace holding only `generate-ts` / `generate-json-schema`.
    pub has_app_server_protocol_server: bool,
    /// Whether `codex exec-server` exists (new home for a WebSocket-based
    /// standalone server, post-0.121).
    pub has_exec_server: bool,
    /// Whether `codex mcp-server` exists (MCP-protocol stdio server).
    pub has_mcp_server: bool,
    /// Flags that `codex exec` treats as equivalent approval/sandbox policy
    /// setters. If the user has already supplied one of these via
    /// `TRIUMVIRATE_CODEX_ARGS`, the daemon must not auto-inject `--full-auto`
    /// (0.121+ rejects the combination).
    pub approval_policy_flags: Vec<&'static str>,
}

impl CodexCapabilities {
    /// Fallback when the probe fails entirely — assume a modern codex with
    /// the most restrictive policy. This is the safest default: worst case
    /// we under-inject, never mis-inject.
    pub fn unknown() -> Self {
        Self {
            version: "unknown".to_string(),
            has_exec: true,
            has_app_server_protocol_server: false,
            has_exec_server: false,
            has_mcp_server: false,
            approval_policy_flags: approval_policy_flag_defaults(),
        }
    }

    /// Returns true iff `existing_args` already contains any flag that the
    /// codex binary treats as an approval/sandbox policy setter. The daemon
    /// must skip `--full-auto` injection when this is true.
    pub fn args_include_explicit_policy(&self, existing_args: &[String]) -> bool {
        self.approval_policy_flags.iter().any(|flag| {
            existing_args
                .iter()
                .any(|arg| arg == flag || arg.starts_with(&format!("{flag}=")))
        })
    }
}

fn approval_policy_flag_defaults() -> Vec<&'static str> {
    vec![
        "--dangerously-bypass-approvals-and-sandbox",
        "-s",
        "--sandbox",
        "--ask-for-approval",
        "-a",
        "--full-auto",
    ]
}

static CODEX_CAPABILITIES: OnceLock<CodexCapabilities> = OnceLock::new();

/// Return the probed codex capabilities, or the `unknown()` fallback if the
/// probe hasn't run yet (e.g. unit tests, or probe failure at boot).
pub fn codex_capabilities() -> &'static CodexCapabilities {
    CODEX_CAPABILITIES
        .get()
        .unwrap_or_else(|| Box::leak(Box::new(CodexCapabilities::unknown())))
}

/// Probe the codex binary at `bin` and cache the result. Safe to call once at
/// daemon startup. Subsequent calls are no-ops because `OnceLock::set` fails
/// silently if already initialised.
pub async fn probe_and_cache_codex_capabilities(bin: &str) {
    let caps = probe_codex_capabilities(bin).await;
    info!(
        version = %caps.version,
        has_exec = caps.has_exec,
        has_app_server_protocol_server = caps.has_app_server_protocol_server,
        has_exec_server = caps.has_exec_server,
        has_mcp_server = caps.has_mcp_server,
        "codex capabilities probed"
    );
    if !caps.has_exec {
        warn!(
            bin = %bin,
            "codex `exec` subcommand missing — daemon will not be able to call codex. \
             Check TRIUMVIRATE_CODEX_BIN and the installed codex-cli version."
        );
    }
    let _ = CODEX_CAPABILITIES.set(caps);
}

async fn probe_codex_capabilities(bin: &str) -> CodexCapabilities {
    let version = run_probe(bin, &["--version"]).await.unwrap_or_default();
    let exec_help = run_probe(bin, &["exec", "--help"]).await;
    let app_server_help = run_probe(bin, &["app-server", "--help"]).await;
    let exec_server_help = run_probe(bin, &["exec-server", "--help"]).await;
    let mcp_server_help = run_probe(bin, &["mcp-server", "--help"]).await;

    let version = version.lines().next().unwrap_or("unknown").trim().to_string();

    CodexCapabilities {
        version,
        has_exec: exec_help.is_some(),
        has_app_server_protocol_server: looks_like_protocol_server(&app_server_help),
        has_exec_server: exec_server_help.is_some(),
        has_mcp_server: mcp_server_help.is_some(),
        approval_policy_flags: approval_policy_flag_defaults(),
    }
}

/// Run a probe command with a hard 5s timeout. Returns `None` on failure,
/// timeout, or non-zero exit. Returns `Some(combined_stdout_stderr)` on
/// success.
async fn run_probe(bin: &str, args: &[&str]) -> Option<String> {
    let fut = async {
        let output = Command::new(bin)
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let mut combined = String::from_utf8_lossy(&output.stdout).to_string();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        Some(combined)
    };
    timeout(Duration::from_secs(5), fut).await.ok().flatten()
}

/// Heuristic: old-shape `codex app-server` (the JSON-RPC-over-stdio server
/// the daemon knows how to speak to) exposes transport-ish flags in its help
/// text. New-shape 0.121+ `codex app-server` is a tooling namespace whose help
/// text only lists `generate-ts` / `generate-json-schema` subcommands.
fn looks_like_protocol_server(help: &Option<String>) -> bool {
    let Some(text) = help else { return false };
    // New-shape tooling namespace — explicitly not the server we want.
    if text.contains("generate-ts") || text.contains("generate-json-schema") {
        return false;
    }
    // Old-shape protocol server surfaced a listen/transport option.
    text.contains("--listen") || text.contains("--transport") || text.contains("stdio")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_capabilities_prefer_safety() {
        let caps = CodexCapabilities::unknown();
        assert!(!caps.has_app_server_protocol_server);
        assert!(caps.args_include_explicit_policy(&[
            "--dangerously-bypass-approvals-and-sandbox".into(),
        ]));
    }

    #[test]
    fn args_include_explicit_policy_matches_equals_form() {
        let caps = CodexCapabilities::unknown();
        assert!(caps.args_include_explicit_policy(&["--sandbox=workspace-write".into()]));
        assert!(caps.args_include_explicit_policy(&["-s".into(), "workspace-write".into()]));
        assert!(!caps.args_include_explicit_policy(&["--json".into()]));
    }

    #[test]
    fn new_shape_app_server_help_is_not_a_server() {
        let help = Some(
            "[experimental] Run the app server or related tooling\n\
             Commands:\n  generate-ts\n  generate-json-schema\n"
                .to_string(),
        );
        assert!(!looks_like_protocol_server(&help));
    }

    #[test]
    fn old_shape_app_server_help_is_a_server() {
        let help = Some(
            "Run the Codex app server\n\
             Options:\n  --listen <URL>\n  --transport <stdio|ws>\n"
                .to_string(),
        );
        assert!(looks_like_protocol_server(&help));
    }
}
