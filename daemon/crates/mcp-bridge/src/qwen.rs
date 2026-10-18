//! Qwen invocation builder (Seven Primes REQ-023, REQ-024, REQ-043). BACKEND_STRUCTURE.md section 6 holds the argv
//! contract; research/03-qwen-code-contract.md holds the CLI behavior each flag relies on.

use crate::containment::{SeatInvocation, SeatRequest};

/// The prompt limit (REQ-024): larger prompts are refused before spawning, never truncated.
pub const MAX_PROMPT_BYTES: usize = 65536;

/// The pinned Qwen model when the seat config names none (TECH_STACK.md: prefer `qwen3-coder-plus`). The plan also
/// serves GLM, Kimi and MiniMax models (research/03 "Auth and plan"), so an unpinned call could be answered by
/// another lab.
pub const DEFAULT_QWEN_MODEL: &str = "qwen3-coder-plus";

/// REQ-023 read-only denylist, the second layer behind `--approval-mode plan` (research/03 "Round 1 research
/// additions", packages/core/src/tools/tool-names.ts).
pub const EXCLUDED_TOOLS: &[&str] = &[
    "run_shell_command",
    "exec",
    "edit",
    "write_file",
    "notebook_edit",
    "agent",
    "save_memory",
    "manage_memory",
    "cron_create",
    "cron_delete",
    "create_sub_session",
    "send_message",
    "team_create",
    "task_create",
    "image_gen",
];

/// Operator extra args the builder owns or that break read-only use (BACKEND_STRUCTURE section 6).
pub const FORBIDDEN_EXTRA_ARGS: &[&str] = &[
    "--continue",
    "--yolo",
    "-y",
    "--approval-mode",
    "--resume",
    "--output-format",
    "-o",
    "-p",
    "--prompt",
    "-m",
    "--model",
    "--safe-mode",
    "--exclude-tools",
    "--allowed-tools",
];

/// research/03: `QWEN_CODE_UNATTENDED_RETRY=1` retries 429/529 forever. A seat must fail fast on quota so the
/// breaker can trip, so the builder always strips it from the inherited env.
pub const UNATTENDED_RETRY_ENV: &str = "QWEN_CODE_UNATTENDED_RETRY";

/// The read-only argv for one dispatch. Errors carry a code first, for example `prompt_too_large_for_qwen`.
pub fn build_qwen_invocation(req: &SeatRequest<'_>) -> Result<SeatInvocation, String> {
    if req.prompt.len() > MAX_PROMPT_BYTES {
        return Err(format!(
            "prompt_too_large_for_qwen: {} bytes > {MAX_PROMPT_BYTES}",
            req.prompt.len()
        ));
    }
    for arg in req.extra_args {
        let flag = arg.split('=').next().unwrap_or(arg);
        if FORBIDDEN_EXTRA_ARGS.contains(&flag) {
            return Err(format!("forbidden_extra_arg_for_qwen: {arg}"));
        }
    }
    let model = req.model.unwrap_or(DEFAULT_QWEN_MODEL);
    if !is_qwen_model(model) {
        return Err(format!("qwen_seat_configured_non_qwen_model: {model}"));
    }
    // research/03 (headless.md:299): `--max-wall-time` takes `90`/`30s`/`5m`, minimum 1 second.
    let wall_secs = req.time_limit.as_secs().max(1);
    let mut args: Vec<String> = [
        "-p",
        req.prompt,
        "--output-format",
        "stream-json",
        // research/03 "Isolation": skips the reviewed repo's `.qwen/settings.json` and `.env`.
        "--safe-mode",
        "-m",
        model,
        "--max-session-turns",
        &req.max_turns.to_string(),
        "--max-wall-time",
        &format!("{wall_secs}s"),
        // research/03 "Invocation": plan = "Read-only analysis only; shell not executed".
        "--approval-mode",
        "plan",
        "--exclude-tools",
        &EXCLUDED_TOOLS.join(","),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    // research/03: `--resume <sessionId>` resumes by UUID; `--continue` (most recent) is never emitted.
    if let Some(id) = req.resume_session_id {
        args.push("--resume".to_string());
        args.push(id.to_string());
    }
    args.extend(req.extra_args.iter().cloned());
    Ok(SeatInvocation {
        program: "qwen".to_string(),
        args,
        env_add: Vec::new(),
        env_remove: vec![UNATTENDED_RETRY_ENV.to_string()],
        cwd: req.cwd.to_path_buf(),
    })
}

/// The key env var and international endpoint for a `qwen_plan` value (TECH_STACK.md, research/03 "Auth and plan").
/// `None` is the default plan, `qwencloud` (D-07). The seat home's settings.json names this var as its `envKey`.
///
/// `modelstudio` is REFUSED, not mapped: TECH_STACK.md says switching to the ModelStudio Token Plan relaxes the `sk-sp-`
/// key check and is a contract change that stops the build (`blocked_on: scope-change`) until Mike decides. A helper
/// that quietly returned its env var would let that change happen without the decision.
pub fn plan_auth(qwen_plan: Option<&str>) -> Result<(&'static str, &'static str), String> {
    match qwen_plan.unwrap_or("qwencloud") {
        "qwencloud" => Ok((
            "QWENCLOUD_TOKEN_PLAN_API_KEY",
            "https://token-plan.maas.qwencloudapi.com/compatible-mode/v1",
        )),
        "modelstudio" => Err(
            "qwen_plan_contract_change: modelstudio needs Mike's decision (TECH_STACK.md, blocked_on: scope-change)"
                .to_string(),
        ),
        other => Err(format!("unknown_qwen_plan: {other}")),
    }
}

fn is_qwen_model(model: &str) -> bool {
    model.to_ascii_lowercase().starts_with("qwen")
}

/// The served-model check (REQ-043): the Qwen seat must be answered by a Qwen model, else
/// `qwen_seat_served_non_qwen_model: <model>`.
pub fn check_served_model(model: &str) -> Result<(), String> {
    if is_qwen_model(model) {
        Ok(())
    } else {
        Err(format!("qwen_seat_served_non_qwen_model: {model}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_adapter::qwen::QwenStreamParser;
    use std::path::Path;
    use std::time::Duration;

    fn req<'a>(prompt: &'a str, resume: Option<&'a str>, extra: &'a [String]) -> SeatRequest<'a> {
        let p = Path::new("/repo");
        SeatRequest {
            prompt,
            cwd: p,
            seat_home: Path::new("/th/homes/qwen"),
            output_dir: Path::new("/th/out"),
            model: None,
            resume_session_id: resume,
            extra_args: extra,
            max_turns: 12,
            time_limit: Duration::from_secs(600),
        }
    }

    fn after(args: &[String], flag: &str) -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1).cloned())
    }

    /// RED IF: any call drops `--safe-mode`, `--approval-mode plan`, the full REQ-023 exclude list or the pinned
    /// model, or a resume call loses them.
    #[test]
    fn every_call_is_read_only_and_pinned() {
        for resume in [None, Some("7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01")] {
            let inv = build_qwen_invocation(&req("review this", resume, &[])).unwrap();
            assert_eq!(inv.program, "qwen");
            assert_eq!(inv.cwd, Path::new("/repo"));
            let a = &inv.args;
            assert!(a.contains(&"--safe-mode".to_string()));
            assert_eq!(after(a, "--approval-mode").as_deref(), Some("plan"));
            assert_eq!(after(a, "-p").as_deref(), Some("review this"));
            assert_eq!(after(a, "--output-format").as_deref(), Some("stream-json"));
            assert_eq!(after(a, "-m").as_deref(), Some(DEFAULT_QWEN_MODEL));
            assert_eq!(after(a, "--max-session-turns").as_deref(), Some("12"));
            assert_eq!(after(a, "--max-wall-time").as_deref(), Some("600s"));
            let excluded = after(a, "--exclude-tools").unwrap();
            let got: Vec<&str> = excluded.split(',').collect();
            assert_eq!(
                got,
                [
                    "run_shell_command",
                    "exec",
                    "edit",
                    "write_file",
                    "notebook_edit",
                    "agent",
                    "save_memory",
                    "manage_memory",
                    "cron_create",
                    "cron_delete",
                    "create_sub_session",
                    "send_message",
                    "team_create",
                    "task_create",
                    "image_gen"
                ]
            );
            assert_eq!(after(a, "--resume").as_deref(), resume);
            for never in ["--yolo", "-y", "--continue"] {
                assert!(!a.contains(&never.to_string()), "{never} emitted");
            }
        }
    }

    /// RED IF: a configured model is ignored, or a non-Qwen model is accepted as the pin.
    #[test]
    fn configured_model_is_pinned_and_must_be_qwen() {
        let mut r = req("p", None, &[]);
        r.model = Some("qwen3.7-plus");
        let inv = build_qwen_invocation(&r).unwrap();
        assert_eq!(after(&inv.args, "-m").as_deref(), Some("qwen3.7-plus"));
        r.model = Some("glm-5");
        assert_eq!(
            build_qwen_invocation(&r).unwrap_err(),
            "qwen_seat_configured_non_qwen_model: glm-5"
        );
    }

    /// RED IF: the child env can inherit QWEN_CODE_UNATTENDED_RETRY from the daemon env.
    #[test]
    fn unattended_retry_never_reaches_the_child() {
        let inv = build_qwen_invocation(&req("p", None, &[])).unwrap();
        let base = [
            (UNATTENDED_RETRY_ENV.to_string(), "1".to_string()),
            ("PATH".to_string(), "/usr/bin".to_string()),
        ];
        // The runner applies env_remove to the inherited env, then env_add (containment.rs SeatInvocation).
        let mut env: Vec<(String, String)> = base
            .into_iter()
            .filter(|(k, _)| !inv.env_remove.contains(k))
            .collect();
        env.extend(inv.env_add.iter().cloned());
        assert!(env.iter().all(|(k, _)| k != UNATTENDED_RETRY_ENV));
        assert!(env.iter().any(|(k, _)| k == "PATH"));
    }

    /// RED IF: a 65 KiB prompt is truncated or spawned instead of refused, or a 64 KiB prompt is refused.
    #[test]
    fn oversized_prompt_is_refused() {
        let big = "x".repeat(65 * 1024);
        assert_eq!(
            build_qwen_invocation(&req(&big, None, &[])).unwrap_err(),
            "prompt_too_large_for_qwen: 66560 bytes > 65536"
        );
        let edge = "x".repeat(MAX_PROMPT_BYTES);
        assert!(build_qwen_invocation(&req(&edge, None, &[])).is_ok());
    }

    /// RED IF: an operator can smuggle a writer, resume or override flag through extra args.
    #[test]
    fn forbidden_extra_args_are_rejected() {
        for bad in [
            "--continue",
            "--yolo",
            "-y",
            "--approval-mode=yolo",
            "--model",
            "--exclude-tools",
        ] {
            let extra = vec![bad.to_string()];
            assert_eq!(
                build_qwen_invocation(&req("p", None, &extra)).unwrap_err(),
                format!("forbidden_extra_arg_for_qwen: {bad}")
            );
        }
        let ok = vec!["--max-tool-calls".to_string(), "40".to_string()];
        let inv = build_qwen_invocation(&req("p", None, &ok)).unwrap();
        assert!(inv.args.ends_with(&ok));
    }

    /// RED IF: the plan-to-env-var mapping changes, an unknown plan is silently defaulted, or the ModelStudio contract
    /// change becomes reachable without a decision.
    #[test]
    fn plan_picks_env_var() {
        assert_eq!(plan_auth(None).unwrap().0, "QWENCLOUD_TOKEN_PLAN_API_KEY");
        assert_eq!(
            plan_auth(Some("qwencloud")).unwrap().1,
            "https://token-plan.maas.qwencloudapi.com/compatible-mode/v1"
        );
        assert!(
            plan_auth(Some("modelstudio"))
                .unwrap_err()
                .starts_with("qwen_plan_contract_change:")
        );
        assert_eq!(
            plan_auth(Some("coding")).unwrap_err(),
            "unknown_qwen_plan: coding"
        );
    }

    /// Provisional: the stream is transcribed from research/03 stream shapes; nothing is installed yet (T-302).
    /// RED IF: a Qwen call served by another lab's model passes the served-model check.
    #[test]
    fn glm_served_stream_fails_the_served_model_check() {
        let stream = concat!(
            r#"{"type":"system","subtype":"session_start","session_id":"s-1","model":"qwen3-coder-plus"}"#,
            "\n",
            r#"{"type":"assistant","session_id":"s-1","message":{"role":"assistant","model":"glm-5","content":[{"type":"text","text":"hi"}]}}"#,
            "\n",
            r#"{"type":"result","subtype":"success","session_id":"s-1","is_error":false,"result":"hi"}"#,
        );
        let mut p = QwenStreamParser::new();
        for line in stream.lines() {
            p.parse_line(line);
        }
        let served = p.finish(Some(0)).unwrap().served_model.unwrap();
        assert_eq!(
            check_served_model(&served).unwrap_err(),
            "qwen_seat_served_non_qwen_model: glm-5"
        );
        assert!(check_served_model("qwen3-coder-plus").is_ok());
        assert!(check_served_model("Qwen3.7-Plus").is_ok());
        assert!(check_served_model("").is_err());
    }
}
