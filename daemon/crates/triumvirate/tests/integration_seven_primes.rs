//! Seven Primes T-205: the kimi, mistral and qwen dispatch arms, through the real `triumvirate` binary.
//!
//! Every test runs its own daemon (or `doctor --live`) with a cleared env: HOME and TRIUMVIRATE_HOME are the test's
//! tempdir, PATH is `/usr/bin:/bin` (no vendor CLI lives there, asserted below), and each seat's binary is a tempdir
//! mock named by TRIUMVIRATE_<SEAT>_BIN. A mock records its argv, its env and one line per spawn, then replays a
//! fixture from agent-adapter/tests/fixtures. No real kimi, vibe or qwen can run, and no real credential is read:
//! the Qwen key is a fake written into the tempdir seat home.
//!
//! T-206 adds the seatbelt wrap and the repo audit around every dispatch (REQ-031, REQ-035): on macOS each mock runs
//! under the real `/usr/bin/sandbox-exec`, and each mock may source a per-seat `action` snippet (a write attempt, a
//! read) in the dispatch cwd.

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    future::Future,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    pin::Pin,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::{Duration, Instant},
};

use mcp_bridge::containment::{SeatInvocation, SeatRequest};
use serde_json::Value;
use shared_types::{AskAgentRequest, AskAgentResponse, AskSessionRequest, SessionState};

const SEATS: [&str; 3] = ["kimi", "mistral", "qwen"];
const METERED: [&str; 5] = [
    "MISTRAL_API_KEY",
    "OPENAI_API_KEY",
    "DASHSCOPE_API_KEY",
    "KIMI_API_KEY",
    "MOONSHOT_API_KEY",
];
const CONNECTOR_TIMEOUT_SECS: u64 = 60;
/// Set by `sh` itself, or by macOS at every exec (`__CF_USER_TEXT_ENCODING`), never by the runner.
// The last two are the LLVM coverage runtime's own variables: under `cargo llvm-cov` the instrumented daemon carries
// them and they reach every child. They are tooling, never a credential.
const SHELL_ADDED: [&str; 7] = [
    "PWD",
    "SHLVL",
    "_",
    "OLDPWD",
    "__CF_USER_TEXT_ENCODING",
    "__LLVM_PROFILE_RT_INIT_ONCE",
    "LLVM_PROFILE_FILE",
];

/// Every test mutates this process's HOME and TRIUMVIRATE_HOME, so they run one at a time.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const KIMI_STREAM: &str =
    include_str!("../../agent-adapter/tests/fixtures/kimi-2.2.0-provisional-20261018.jsonl");
const MISTRAL_STREAM: &str =
    include_str!("../../agent-adapter/tests/fixtures/mistral-2.26.1-provisional-20261018.jsonl");
const MISTRAL_EXPORT: &str = include_str!(
    "../../agent-adapter/tests/fixtures/mistral-2.26.1-export-provisional-20261018.json"
);
const QWEN_STREAM: &str =
    include_str!("../../agent-adapter/tests/fixtures/qwen-0.25.2-provisional-20261018.jsonl");

fn version(seat: &str) -> &'static str {
    match seat {
        "kimi" => "2.2.0",
        "mistral" => "2.26.1",
        _ => "0.25.2",
    }
}

/// The fixture's final answer, its stdout and (Mistral) its export.json.
fn fixture(seat: &str) -> (&'static str, String, Option<String>) {
    match seat {
        "kimi" => (
            "main.rs defines an empty main function.",
            KIMI_STREAM.to_string(),
            None,
        ),
        "mistral" => (
            "Reading the file.\nThe add function is correct.",
            MISTRAL_STREAM.to_string(),
            Some(MISTRAL_EXPORT.to_string()),
        ),
        _ => (
            "main.rs prints hello and exits.",
            QWEN_STREAM.to_string(),
            None,
        ),
    }
}

/// A test-only key-shaped value, built at runtime so no secret-looking literal is committed.
fn fake_key(prefix: &str) -> String {
    [prefix, "t205fake", "0123456789"].concat()
}

struct Rig {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    th: PathBuf,
    cwd: PathBuf,
    port: u16,
    daemon: Option<Child>,
    log: PathBuf,
    /// Extra variables for the daemon's env (T-207: a per-seat stall window).
    extra_env: Vec<(String, String)>,
}

impl Rig {
    /// A tempdir world: HOME, TRIUMVIRATE_HOME, a cwd, one mock per seat, Qwen's fake credentials and a
    /// verified.json per seat for the mock's version. Also points THIS process's HOME and TRIUMVIRATE_HOME there.
    fn new() -> Self {
        for program in ["kimi", "vibe", "qwen"] {
            for dir in ["/usr/bin", "/bin"] {
                assert!(
                    !Path::new(dir).join(program).exists(),
                    "a real {program} in {dir} would be reachable from the test PATH"
                );
            }
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let root = fs::canonicalize(dir.path()).expect("canonical tempdir");
        let home = root.join("home");
        let th = root.join("th");
        let cwd = root.join("repo");
        for d in [&home, &th, &cwd] {
            fs::create_dir_all(d).unwrap();
        }
        // SAFETY: ENV_LOCK is held by every test for its whole life, so no other thread reads the env meanwhile.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("TRIUMVIRATE_HOME", &th);
        }
        let rig = Self {
            _dir: dir,
            root,
            home,
            th,
            cwd,
            port: free_port(),
            daemon: None,
            log: PathBuf::new(),
            extra_env: Vec::new(),
        };
        rig.assert_home_inside();
        for seat in SEATS {
            rig.write_mock(seat);
            let (_, stdout, export) = fixture(seat);
            rig.set_fixture(seat, &stdout, 0, "", export.as_deref());
            rig.write_verified(seat);
        }
        let qwen_home = rig.th.join("homes/qwen");
        fs::create_dir_all(&qwen_home).unwrap();
        let creds = qwen_home.join("credentials.env");
        // No trailing newline (A-9).
        fs::write(
            &creds,
            format!("QWENCLOUD_TOKEN_PLAN_API_KEY={}", rig.qwen_key()),
        )
        .unwrap();
        fs::set_permissions(&creds, fs::Permissions::from_mode(0o600)).unwrap();
        rig
    }

    /// Panics unless the state directory this process resolves is inside the test's tempdir.
    fn assert_home_inside(&self) {
        let resolved = mcp_bridge::seat_state::triumvirate_home().expect("triumvirate home");
        assert!(
            resolved.starts_with(&self.root),
            "TRIUMVIRATE_HOME resolved to {} outside the test tempdir {}",
            resolved.display(),
            self.root.display()
        );
    }

    fn qwen_key(&self) -> String {
        fake_key("sk-sp-")
    }

    fn mock_dir(&self, seat: &str) -> PathBuf {
        self.root.join(format!("mock-{seat}"))
    }

    fn mock_bin(&self, seat: &str) -> PathBuf {
        self.mock_dir(seat).join(format!("fake-{seat}"))
    }

    fn write_mock(&self, seat: &str) {
        let d = self.mock_dir(seat);
        fs::create_dir_all(&d).unwrap();
        let script = format!(
            r#"#!/bin/sh
D='{d}'
if [ "$1" = --version ]; then echo {v}; exit 0; fi
echo spawn >> "$D/count"
n=$(wc -l < "$D/count" | tr -d ' ')
: > "$D/argv"
: > "$D/argv.$n"
for a in "$@"; do printf '%s\n' "$a" >> "$D/argv"; printf '%s\n' "$a" >> "$D/argv.$n"; done
env > "$D/env"
if [ -f "$D/action" ]; then . "$D/action"; fi
prev=""
for a in "$@"; do
  if [ "$prev" = --output-dir ] && [ -f "$D/export.json" ]; then cp "$D/export.json" "$a/export.json"; fi
  prev="$a"
done
if [ -f "$D/stderr" ]; then cat "$D/stderr" >&2; fi
if [ -f "$D/stdout" ]; then cat "$D/stdout"; fi
exit "$(cat "$D/exit")"
"#,
            d = d.display(),
            v = version(seat)
        );
        let bin = self.mock_bin(seat);
        fs::write(&bin, script).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn set_fixture(&self, seat: &str, stdout: &str, exit: i32, stderr: &str, export: Option<&str>) {
        let d = self.mock_dir(seat);
        fs::write(d.join("stdout"), stdout).unwrap();
        fs::write(d.join("exit"), exit.to_string()).unwrap();
        fs::write(d.join("stderr"), stderr).unwrap();
        let ex = d.join("export.json");
        match export {
            Some(text) => fs::write(&ex, text).unwrap(),
            None => {
                let _ = fs::remove_file(&ex);
            }
        }
    }

    /// A shell snippet the mock sources after recording its argv and env, run in the dispatch cwd (T-206).
    fn set_action(&self, seat: &str, script: &str) {
        fs::write(self.mock_dir(seat).join("action"), script).unwrap();
    }

    fn fresh_cwd(&self, label: &str) -> PathBuf {
        let path = self.root.join(format!("cwd-{label}"));
        fs::create_dir(&path).unwrap();
        path
    }

    fn mock_file(&self, seat: &str, name: &str) -> String {
        fs::read_to_string(self.mock_dir(seat).join(name)).unwrap_or_default()
    }

    fn write_verified(&self, seat: &str) {
        mcp_bridge::seat_state::write_verified(
            &self.th,
            seat,
            &mcp_bridge::seat_state::VerifiedRecord {
                cli_version: version(seat).into(),
                seat_home: self.th.join("homes").join(seat).display().to_string(),
                probed_at: "2026-10-18T00:00:00Z".into(),
                served_model: None,
            },
        )
        .unwrap();
    }

    fn spawns(&self, seat: &str) -> usize {
        fs::read_to_string(self.mock_dir(seat).join("count"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    fn recorded_argv(&self, seat: &str) -> Vec<String> {
        fs::read_to_string(self.mock_dir(seat).join("argv"))
            .expect("the mock ran and recorded its argv")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn recorded_argv_for(&self, seat: &str, spawn: usize) -> Vec<String> {
        fs::read_to_string(self.mock_dir(seat).join(format!("argv.{spawn}")))
            .expect("the requested mock invocation recorded its argv")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn recorded_env(&self, seat: &str) -> BTreeMap<String, String> {
        fs::read_to_string(self.mock_dir(seat).join("env"))
            .expect("the mock ran and recorded its env")
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The env every child of this rig starts from: cleared, then only what is listed here.
    fn base_env(&self) -> Vec<(String, String)> {
        let mut env: Vec<(String, String)> = vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), self.home.display().to_string()),
            ("TRIUMVIRATE_HOME".into(), self.th.display().to_string()),
            (
                "TRIUMVIRATE_DAEMON_BIND_ADDR".into(),
                format!("127.0.0.1:{}", self.port),
            ),
            ("TRIUMVIRATE_DAEMON_PREWARM".into(), "0".into()),
            ("TRIUMVIRATE_GEMINI_BACKEND".into(), "gemini-cli".into()),
            (
                "TRIUMVIRATE_CONNECTOR_TIMEOUT_SECS".into(),
                CONNECTOR_TIMEOUT_SECS.to_string(),
            ),
        ];
        for seat in SEATS {
            let upper = seat.to_ascii_uppercase();
            env.push((format!("TRIUMVIRATE_ENABLE_{upper}"), "1".into()));
            env.push((
                format!("TRIUMVIRATE_{upper}_BIN"),
                self.mock_bin(seat).display().to_string(),
            ));
        }
        // Existing seats can never launch a real CLI from here either.
        for other in ["CODEX", "GEMINI", "AGY", "CLAUDE", "GROK"] {
            env.push((format!("TRIUMVIRATE_{other}_BIN"), "/usr/bin/false".into()));
        }
        for k in METERED {
            env.push((k.to_string(), fake_key("metered-")));
        }
        env.extend(self.extra_env.iter().cloned());
        if let Ok(value) = std::env::var("TRIUMVIRATE_SESSION_BUSY_SECS") {
            env.push(("TRIUMVIRATE_SESSION_BUSY_SECS".into(), value));
        }
        env
    }

    fn command(&self, args: &[&str], log: &Path) -> Command {
        let out = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .expect("log");
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_triumvirate"));
        cmd.args(args)
            .env_clear()
            .envs(self.base_env())
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(out.try_clone().expect("dup"))
            .stderr(out);
        cmd
    }

    async fn start_daemon(&mut self) {
        // The resolved binaries are the mocks, never a vendor CLI.
        for seat in SEATS {
            let bin = self
                .base_env()
                .into_iter()
                .find(|(k, _)| *k == format!("TRIUMVIRATE_{}_BIN", seat.to_ascii_uppercase()))
                .map(|(_, v)| v)
                .unwrap();
            assert_eq!(PathBuf::from(&bin), self.mock_bin(seat));
            assert!(bin.starts_with(&*self.root.to_string_lossy()));
        }
        self.log = self.root.join("daemon.log");
        self.daemon = Some(
            self.command(&["daemon"], &self.log.clone())
                .spawn()
                .expect("start daemon"),
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Ok(token) = fs::read_to_string(self.th.join("daemon.token"))
                && let Ok(resp) = client()
                    .get(format!("{}/health", self.base()))
                    .bearer_auth(token.trim())
                    .send()
                    .await
                && resp.status().is_success()
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "daemon did not come up: {}",
                fs::read_to_string(&self.log).unwrap_or_default()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// POST /ask-agent: Ok(response json) or Err(error text).
    async fn ask(&self, seat: &str, message: &str) -> Result<Value, String> {
        self.ask_in(seat, message, &self.cwd).await
    }

    /// POST /ask-agent with an explicit cwd.
    async fn ask_in(&self, seat: &str, message: &str, cwd: &Path) -> Result<Value, String> {
        self.ask_body(serde_json::json!({"agent": seat, "message": message, "cwd": cwd}))
            .await
    }

    /// POST /ask-agent as a jury seat would send it (`ask_surface: jury`, T-204), with an explicit cwd.
    async fn ask_jury_in(&self, seat: &str, message: &str, cwd: &Path) -> Result<Value, String> {
        self.ask_body(serde_json::json!({
            "agent": seat, "message": message, "cwd": cwd, "ask_surface": "jury"
        }))
        .await
    }

    async fn ask_body(&self, body: Value) -> Result<Value, String> {
        let token = fs::read_to_string(self.th.join("daemon.token")).expect("token");
        let resp = client()
            .post(format!("{}/ask-agent", self.base()))
            .bearer_auth(token.trim())
            .json(&body)
            .send()
            .await
            .expect("ask-agent");
        let ok = resp.status().is_success();
        let body: Value = resp.json().await.expect("json");
        if ok {
            Ok(body)
        } else {
            Err(body["error"].as_str().unwrap_or_default().to_string())
        }
    }

    /// The builder's output for the call the mock recorded (Mistral's per-call output dir is read back from argv).
    fn expected_invocation(&self, seat: &str, message: &str) -> SeatInvocation {
        let argv = self.recorded_argv(seat);
        let output_dir = argv
            .iter()
            .position(|a| a == "--output-dir")
            .and_then(|i| argv.get(i + 1))
            .map_or_else(|| self.root.join("unused"), PathBuf::from);
        let seat_home = self.th.join("homes").join(seat);
        let req = SeatRequest {
            prompt: message,
            cwd: &self.cwd,
            seat_home: &seat_home,
            output_dir: &output_dir,
            model: None,
            resume_session_id: None,
            extra_args: &[],
            max_turns: 12,
            time_limit: Duration::from_secs(CONNECTOR_TIMEOUT_SECS),
        };
        match seat {
            "kimi" => mcp_bridge::kimi::build_kimi_invocation(&req),
            "mistral" => mcp_bridge::mistral::build_mistral_invocation(&req),
            _ => mcp_bridge::qwen::build_qwen_invocation(&req),
        }
        .expect("builder")
    }

    /// The child env the runner must produce: the daemon's env minus the builder's env_remove, then `child_env`.
    fn expected_env(&self, seat: &str, inv: &SeatInvocation) -> BTreeMap<String, String> {
        let profile = mcp_bridge::seat_profile::seat_profile(seat).unwrap();
        let base: Vec<(String, String)> = self
            .base_env()
            .into_iter()
            .filter(|(k, _)| {
                !inv.env_remove.iter().any(|p| match p.strip_suffix('*') {
                    Some(prefix) => k.starts_with(prefix),
                    None => k == p,
                })
            })
            .collect();
        let mut add = inv.env_add.clone();
        if seat == "qwen" {
            add.push(("QWENCLOUD_TOKEN_PLAN_API_KEY".into(), self.qwen_key()));
        }
        let seat_home = self.th.join("homes").join(seat);
        mcp_bridge::containment::child_env(&base, profile, &seat_home, &add)
            .unwrap()
            .into_iter()
            .collect()
    }

    /// The recorded argv equals the builder output, the recorded env equals the runner's env (plus what `sh` sets
    /// or the OS sets itself), and the child's HOME is the seat home inside this tempdir.
    fn assert_spawn_matches_builder(&self, seat: &str, message: &str) {
        let inv = self.expected_invocation(seat, message);
        assert_eq!(
            self.recorded_argv(seat),
            inv.args,
            "{seat}: argv is the builder's"
        );
        let got = self.recorded_env(seat);
        let want = self.expected_env(seat, &inv);
        for (k, v) in &want {
            assert_eq!(got.get(k), Some(v), "{seat}: env {k}");
        }
        for k in got.keys() {
            assert!(
                want.contains_key(k) || SHELL_ADDED.contains(&k.as_str()),
                "{seat}: unexpected env var {k} reached the child"
            );
        }
        let home = got.get("HOME").expect("HOME");
        assert!(
            home.starts_with(&*self.root.to_string_lossy()),
            "{seat}: HOME {home}"
        );
        assert_eq!(Path::new(home), self.th.join("homes").join(seat));
    }

    fn log_text(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("client")
}

async fn http_spawn(rig: &Rig, token: &str, agent: &str, name: &str, cwd: &Path) {
    let response = client()
        .post(format!("{}/session/spawn", rig.base()))
        .bearer_auth(token)
        .json(&serde_json::json!({"agent":agent,"name":name,"cwd":cwd}))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
}

async fn http_ask(
    rig: &Rig,
    token: &str,
    name: &str,
    message: &str,
) -> (reqwest::StatusCode, Value) {
    let response = client()
        .post(format!("{}/session/ask", rig.base()))
        .bearer_auth(token)
        .json(&serde_json::json!({"name":name,"message":message}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.json().await.unwrap();
    (status, body)
}

static IN_PROCESS_PRIOR_IDS: OnceLock<StdMutex<Vec<Option<String>>>> = OnceLock::new();
static IN_PROCESS_SAW_HTTP_ACTIVE: OnceLock<StdMutex<Vec<bool>>> = OnceLock::new();

fn recording_in_process_executor<'a>(
    req: &'a AskAgentRequest,
    _emitter: Option<mcp_tools::ProgressEmitter>,
) -> Pin<Box<dyn Future<Output = Result<AskAgentResponse, String>> + Send + 'a>> {
    IN_PROCESS_PRIOR_IDS
        .get_or_init(|| StdMutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(req.prior_cli_session_id.clone());
    let saw_http = std::env::var_os("TRIUMVIRATE_HOME")
        .map(PathBuf::from)
        .is_some_and(|home| home.join("http-active").exists());
    IN_PROCESS_SAW_HTTP_ACTIVE
        .get_or_init(|| StdMutex::new(Vec::new()))
        .lock()
        .unwrap()
        .push(saw_http);
    Box::pin(async move {
        if req.message == "hold" {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let mut response = AskAgentResponse::direct(
            "integration-request".to_string(),
            req.agent.clone(),
            "in-process answer".to_string(),
            Vec::new(),
        );
        response.cli_session_id = Some("in-process-id".to_string());
        Ok(response)
    })
}

/// Each seat answers through ask_agent with the fixture's answer and its tool record; the child's argv and env are
/// exactly the builder's (HOME is the seat home, scrubbed vars absent, Mistral VIBE_CLI=python, Qwen's plan key
/// present with the fake value); the read_only_seat marker is logged.
/// RED IF: a seat's arm is missing (unsupported agent), an answer or tool record is lost, the argv or env drifts from
/// the builder, or the marker is not logged.
#[tokio::test]
async fn each_seat_answers_with_the_builder_argv_and_env() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.start_daemon().await;
    for seat in SEATS {
        let (answer, _, _) = fixture(seat);
        let resp = rig
            .ask(seat, "review main")
            .await
            .unwrap_or_else(|e| panic!("{seat}: {e}"));
        assert_eq!(resp["response"], answer, "{seat}: {resp}");
        assert_eq!(resp["agent"], seat);
        assert_eq!(
            resp["tool_calls_made"], 1,
            "{seat}: the fixture's one tool record"
        );
        assert_eq!(rig.spawns(seat), 1);
        rig.assert_spawn_matches_builder(seat, "review main");
        let env = rig.recorded_env(seat);
        match seat {
            "mistral" => assert_eq!(env.get("VIBE_CLI").map(String::as_str), Some("python")),
            "qwen" => assert_eq!(
                env.get("QWENCLOUD_TOKEN_PLAN_API_KEY"),
                Some(&rig.qwen_key())
            ),
            _ => assert_eq!(
                env.get("KIMI_DISABLE_TELEMETRY").map(String::as_str),
                Some("1")
            ),
        }
        assert!(
            rig.log_text().contains(&format!(
                "read_only_seat: {seat} (answers only, never edits)"
            )),
            "{seat}: marker missing from the log"
        );
    }
}

/// Two different mock outputs give two different answers: nothing is canned.
/// RED IF: the runner returns a fixed answer or a cached one.
#[tokio::test]
async fn two_mock_outputs_give_two_answers() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.start_daemon().await;
    for seat in SEATS {
        let (answer, stdout, export) = fixture(seat);
        let last_line = answer.lines().last().unwrap();
        let mut seen = Vec::new();
        for word in ["alpha", "bravo"] {
            let swapped = stdout.replace(last_line, &format!("answer {word}"));
            rig.set_fixture(seat, &swapped, 0, "", export.as_deref());
            let resp = rig
                .ask(seat, "q")
                .await
                .unwrap_or_else(|e| panic!("{seat}: {e}"));
            let text = resp["response"].as_str().unwrap().to_string();
            assert!(text.ends_with(&format!("answer {word}")), "{seat}: {text}");
            seen.push(text);
        }
        assert_ne!(seen[0], seen[1], "{seat}");
    }
}

/// A quota failure is one attempt, terminal, and opens that seat's breaker: the next call is refused without a spawn.
/// RED IF: a quota failure is retried (spawn count above one), is not reported `seat_quota_exhausted:<seat>`, or the
/// breaker stays closed and the next call spawns again.
#[tokio::test]
async fn quota_is_one_spawn_and_opens_the_breaker() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    let kimi_retry = r#"{"role":"meta","type":"turn.step.retrying","failed_attempt":1,"next_attempt":2,"max_attempts":3,"delay_ms":1000,"error_name":"RateLimit","error_message":"too many requests","status_code":429}"#;
    let mistral_export = r#"{"schema_version":1,"outcome":"infrastructure_failure","error":{"message":"429 Too Many Requests: quota exceeded"},"session_id":"s"}"#;
    rig.set_fixture("kimi", &format!("{kimi_retry}\n"), 1, "", None);
    rig.set_fixture("mistral", "", 2, "", Some(mistral_export));
    rig.set_fixture(
        "qwen",
        "",
        1,
        "Error: 429 quota exhausted for this plan\n",
        None,
    );
    rig.start_daemon().await;
    for seat in SEATS {
        let err = rig.ask(seat, "q").await.expect_err("quota");
        assert!(
            err.contains(&format!("seat_quota_exhausted:{seat}")),
            "{seat}: {err}"
        );
        assert_eq!(rig.spawns(seat), 1, "{seat}: exactly one attempt");
        let err = rig.ask(seat, "q").await.expect_err("breaker open");
        assert!(
            err.contains(&format!("seat_quota_exhausted:{seat}")) && err.contains("breaker open"),
            "{seat}: {err}"
        );
        assert_eq!(
            rig.spawns(seat),
            1,
            "{seat}: an open breaker spawns nothing"
        );
    }
}

/// A 403 writes suspended.json and the next call is refused `seat_suspended` by the gate, with no spawn.
/// RED IF: a 403 is not classified as a suspension, suspended.json is not written, or the next call spawns.
#[tokio::test]
async fn forbidden_writes_suspended_and_the_next_call_is_refused() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    let mistral_export = r#"{"schema_version":1,"outcome":"infrastructure_failure","error":{"message":"403 Forbidden: account suspended"},"session_id":"s"}"#;
    rig.set_fixture("kimi", "", 1, "Error: 403 Forbidden\n", None);
    rig.set_fixture("mistral", "", 2, "", Some(mistral_export));
    rig.set_fixture("qwen", "", 1, "Error: 403 Forbidden: key banned\n", None);
    rig.start_daemon().await;
    for seat in SEATS {
        let err = rig.ask(seat, "q").await.expect_err("suspended");
        assert!(
            err.contains(&format!("seat_suspended: {seat}")),
            "{seat}: {err}"
        );
        assert!(
            rig.th
                .join("seats")
                .join(seat)
                .join("suspended.json")
                .is_file(),
            "{seat}"
        );
        let err = rig.ask(seat, "q").await.expect_err("refused");
        assert!(
            err.starts_with(&format!("seat_suspended: {seat}:")),
            "{seat}: {err}"
        );
        assert_eq!(
            rig.spawns(seat),
            1,
            "{seat}: the refused call spawned nothing"
        );
    }
}

/// A Qwen stream served by `glm-5` fails `qwen_seat_served_non_qwen_model`.
/// RED IF: the served-model check is skipped and another lab's answer is returned as Qwen's.
#[tokio::test]
async fn qwen_served_by_another_model_fails() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.set_fixture(
        "qwen",
        &QWEN_STREAM.replace("qwen3-coder-plus", "glm-5"),
        0,
        "",
        None,
    );
    rig.start_daemon().await;
    let err = rig.ask("qwen", "q").await.expect_err("non-qwen model");
    assert!(
        err.contains("qwen_seat_served_non_qwen_model: glm-5"),
        "{err}"
    );
    assert_eq!(rig.spawns("qwen"), 1);
}

/// Exit 0 with no recognizable events fails `empty_or_unrecognized_stream:<seat>`.
/// RED IF: an empty or foreign stream is returned as an empty success.
#[tokio::test]
async fn exit_zero_with_no_events_is_empty_or_unrecognized() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    for seat in SEATS {
        let export = (seat == "mistral").then_some(MISTRAL_EXPORT);
        rig.set_fixture(seat, "this is not a stream\n", 0, "", export);
    }
    rig.start_daemon().await;
    for seat in SEATS {
        let err = rig.ask(seat, "q").await.expect_err("empty");
        assert!(
            err.contains(&format!("empty_or_unrecognized_stream:{seat}")),
            "{seat}: {err}"
        );
    }
}

/// An ordinary ask_agent call (read_only == false) still records the read-only argv, equal to the builder output,
/// and returns the answer.
/// RED IF: a non-review call drops a read-only flag (kimi's agent file, mistral's tool allowlist, qwen's plan mode)
/// or is refused.
#[tokio::test]
async fn a_non_review_call_still_runs_read_only() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.start_daemon().await;
    for seat in SEATS {
        let resp = rig
            .ask(seat, "plain question")
            .await
            .unwrap_or_else(|e| panic!("{seat}: {e}"));
        assert_eq!(resp["response"], fixture(seat).0);
        rig.assert_spawn_matches_builder(seat, "plain question");
        let argv = rig.recorded_argv(seat);
        let after = |flag: &str| {
            argv.iter()
                .position(|a| a == flag)
                .and_then(|i| argv.get(i + 1))
                .cloned()
        };
        match seat {
            "kimi" => assert_eq!(
                after("--agent-file").map(PathBuf::from),
                Some(rig.th.join("homes/kimi/ro-agent.md"))
            ),
            "mistral" => {
                assert_eq!(after("--agent").as_deref(), Some("plan"));
                assert!(!argv.iter().any(|a| a == "--auto-approve" || a == "--trust"));
            }
            _ => {
                assert_eq!(after("--approval-mode").as_deref(), Some("plan"));
                assert!(argv.iter().any(|a| a == "--safe-mode"));
            }
        }
    }
}

/// Metered keys set in the daemon's env never reach a child; Qwen's only key var is the plan var with the tempdir
/// fake value.
/// RED IF: the scrub is skipped or applied before env_add is merged wrongly.
#[tokio::test]
async fn metered_keys_never_reach_a_child() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    // SAFETY: ENV_LOCK is held.
    unsafe {
        for k in METERED {
            std::env::set_var(k, fake_key("metered-"));
        }
    }
    rig.start_daemon().await;
    for seat in SEATS {
        rig.ask(seat, "q")
            .await
            .unwrap_or_else(|e| panic!("{seat}: {e}"));
        let env = rig.recorded_env(seat);
        for k in METERED {
            assert!(!env.contains_key(k), "{seat}: {k} reached the child");
        }
        let keyish: Vec<&String> = env
            .keys()
            .filter(|k| k.contains("API_KEY") || k.contains("TOKEN"))
            .collect();
        if seat == "qwen" {
            assert_eq!(keyish, ["QWENCLOUD_TOKEN_PLAN_API_KEY"]);
            assert_eq!(env["QWENCLOUD_TOKEN_PLAN_API_KEY"], rig.qwen_key());
        } else {
            assert!(keyish.is_empty(), "{seat}: {keyish:?}");
        }
    }
    // SAFETY: ENV_LOCK is held.
    unsafe {
        for k in METERED {
            std::env::remove_var(k);
        }
    }
}

/// `doctor --live` spawns each mock seat once and writes verified.json (installed version, seat home, served model);
/// a Qwen probe served by `glm-5` deletes the seat's verified.json.
/// RED IF: the live probe is still the T-203 placeholder, writes nothing, or keeps verified.json after a served-model
/// failure.
#[tokio::test]
async fn doctor_live_spawns_each_seat_and_writes_verified() {
    let _g = ENV_LOCK.lock().await;
    let rig = Rig::new();
    for seat in SEATS {
        fs::remove_file(rig.th.join("seats").join(seat).join("verified.json")).unwrap();
    }
    let log = rig.root.join("doctor.log");
    let status = rig
        .command(&["doctor", "--live"], &log)
        .status()
        .expect("doctor");
    let out = fs::read_to_string(&log).unwrap();
    assert!(status.success(), "{out}");
    for seat in SEATS {
        assert!(out.contains(&format!("{seat}.live: PASS")), "{seat}: {out}");
        assert_eq!(
            rig.spawns(seat),
            1,
            "{seat}: the probe spawned the mock once"
        );
        let rec: mcp_bridge::seat_state::VerifiedRecord = serde_json::from_slice(
            &fs::read(rig.th.join("seats").join(seat).join("verified.json"))
                .expect("verified.json"),
        )
        .unwrap();
        assert_eq!(rec.cli_version, version(seat));
        assert_eq!(
            PathBuf::from(&rec.seat_home),
            rig.th.join("homes").join(seat)
        );
        let served = if seat == "qwen" {
            Some("qwen3-coder-plus")
        } else {
            None
        };
        assert_eq!(rec.served_model.as_deref(), served, "{seat}");
    }

    rig.set_fixture(
        "qwen",
        &QWEN_STREAM.replace("qwen3-coder-plus", "glm-5"),
        0,
        "",
        None,
    );
    let status = rig
        .command(&["doctor", "--live"], &log)
        .status()
        .expect("doctor");
    let out = fs::read_to_string(&log).unwrap();
    assert!(status.success(), "{out}");
    assert!(
        out.contains("qwen.live: FAIL (qwen_seat_served_non_qwen_model: glm-5"),
        "{out}"
    );
    assert!(
        !rig.th.join("seats/qwen/verified.json").exists(),
        "served-model failure deletes it"
    );
    assert!(rig.th.join("seats/kimi/verified.json").exists());
}

// Seven Primes T-206: the seatbelt wrap and the repo audit around every new-seat dispatch.

/// Run git in `repo` with a throwaway identity and no signing (HOME is the rig's, so no user config is read).
fn git_in(repo: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .stdout(Stdio::null())
        .status()
        .expect("git must be on PATH")
        .success();
    assert!(ok, "git {args:?} in {}", repo.display());
}

/// `dir` becomes a git repo holding `files`, all committed.
fn git_repo(dir: &Path, files: &[(&str, &str)]) {
    fs::create_dir_all(dir).unwrap();
    for (name, text) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    git_in(dir, &["init", "-q"]);
    git_in(dir, &["add", "-A"]);
    git_in(dir, &["commit", "-qm", "base"]);
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

/// The hostile fixture copied into `dir` and committed there, so the fixture tree itself is never the cwd.
fn hostile_repo(dir: &Path) {
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hostile-repo"),
        dir,
    );
    git_repo(dir, &[]);
}

/// Every file under `dir` except `.git/` and `.triumvirate/`, with its bytes: the state a read-only call must leave
/// untouched. `.triumvirate/` is the daemon's own ledger dir (ledger/src/store.rs), not the seat's.
fn tree_bytes(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == ".git" || entry.file_name() == ".triumvirate" {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                stack.push(entry.path());
            } else {
                out.insert(
                    entry.path().strip_prefix(dir).unwrap().to_path_buf(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    out
}

/// The mock tries to write pwned.txt into the cwd and records the attempt's exit status and stderr.
const WRITE_PWNED: &str = r#"{ echo x > "$(pwd -P)/pwned.txt"; } 2> "$D/write_err"
echo "write_exit=$?" > "$D/write_record"
"#;

/// Under the real seatbelt a mock's write into a repo outside every allowed root is refused: pwned.txt is absent, the
/// mock's own record shows the write failed, and the call returns the mock's answer (nothing changed, so no
/// violation). The repo lives under CARGO_TARGET_TMPDIR because the profile allows the system temp roots.
/// RED IF: the seat runs unwrapped (pwned.txt exists and the call fails read_only_violation), the mock never ran
/// (no write record), or a clean seatbelted call is reported as a violation.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn seatbelt_refuses_a_repo_write() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    let outside = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("target tmp");
    let repo = fs::canonicalize(outside.path()).unwrap().join("repo");
    for root in [
        "/private/var/folders",
        "/private/tmp",
        "/var/folders",
        "/tmp",
    ] {
        assert!(
            !repo.starts_with(root),
            "{} is seat-writable",
            repo.display()
        );
    }
    git_repo(&repo, &[("README.md", "hello\n")]);
    rig.set_action("kimi", WRITE_PWNED);
    rig.start_daemon().await;
    let resp = rig
        .ask_in("kimi", "review", &repo)
        .await
        .unwrap_or_else(|e| panic!("{e}\n{}", rig.log_text()));
    assert_eq!(resp["response"], fixture("kimi").0);
    assert!(
        !repo.join("pwned.txt").exists(),
        "the seatbelt let the write through"
    );
    let record = rig.mock_file("kimi", "write_record");
    assert!(
        record.starts_with("write_exit=") && record.trim() != "write_exit=0",
        "the mock's failed write is recorded: {record:?}"
    );
    let err = rig.mock_file("kimi", "write_err");
    assert!(err.contains("not permitted"), "{err:?}");
}

/// Seatbelt bypass simulated: the repo sits under the system temp root, which the profile allows, so the mock's write
/// lands. The audit catches it and the call fails `read_only_violation:<seat>: pwned.txt` for every seat; nothing is
/// reverted.
/// RED IF: the post-run audit is missing or ignores untracked files, or the seat's answer is returned anyway.
#[tokio::test]
async fn a_write_past_the_seatbelt_is_a_read_only_violation() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.start_daemon().await;
    for seat in SEATS {
        let repo = rig.root.join(format!("bypass-{seat}"));
        git_repo(&repo, &[("README.md", "hello\n")]);
        rig.set_action(seat, WRITE_PWNED);
        let err = rig
            .ask_in(seat, "review", &repo)
            .await
            .expect_err("a write is a violation");
        assert!(
            err.contains(&format!("read_only_violation:{seat}: pwned.txt;")),
            "{seat}: {err}"
        );
        assert!(
            repo.join("pwned.txt").exists(),
            "{seat}: nothing is reverted"
        );
    }
}

/// Per seat, with the hostile fixture repo as the cwd, the call returns that seat mock's own answer (all three
/// distinct), the argv carries the read-only controls (Kimi's agent file listing only Read, Grep, Glob; Qwen
/// `--safe-mode` and `--auth-type openai`; Mistral no `--trust`), and no file in the repo changed.
/// RED IF: a hostile project file changes what the runner launches, an answer is canned or crossed between seats, a
/// read-only control drops, or the call writes into the repo.
#[tokio::test]
async fn hostile_repo_cwd_keeps_each_seat_read_only() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.start_daemon().await;
    let mut answers = Vec::new();
    for seat in SEATS {
        let repo = rig.root.join(format!("hostile-{seat}"));
        hostile_repo(&repo);
        assert!(repo.join(".kimi-code/agents/coder.md").is_file());
        let before = tree_bytes(&repo);
        let resp = rig
            .ask_in(seat, "review", &repo)
            .await
            .unwrap_or_else(|e| panic!("{seat}: {e}"));
        assert_eq!(resp["response"], fixture(seat).0, "{seat}");
        answers.push(resp["response"].as_str().unwrap().to_string());
        let argv = rig.recorded_argv(seat);
        let after = |flag: &str| {
            argv.iter()
                .position(|a| a == flag)
                .and_then(|i| argv.get(i + 1))
                .cloned()
        };
        match seat {
            "kimi" => {
                let file = after("--agent-file").expect("--agent-file");
                assert_eq!(PathBuf::from(&file), rig.th.join("homes/kimi/ro-agent.md"));
                let text = fs::read_to_string(&file).unwrap();
                assert!(text.contains("tools: [Read, Grep, Glob]"), "{text}");
            }
            "mistral" => assert!(!argv.iter().any(|a| a == "--trust"), "{argv:?}"),
            _ => {
                assert!(argv.iter().any(|a| a == "--safe-mode"), "{argv:?}");
                assert_eq!(after("--auth-type").as_deref(), Some("openai"), "{argv:?}");
            }
        }
        // The daemon's ledger (not the seat) adds `.triumvirate/` to the cwd's .gitignore on its first call there
        // (ledger/src/init.rs ensure_triumvirate_gitignore); every other byte must be as it was.
        let mut now = tree_bytes(&repo);
        if !before.contains_key(Path::new(".gitignore"))
            && now.get(Path::new(".gitignore")).map(Vec::as_slice)
                == Some(b".triumvirate/\n".as_slice())
        {
            now.remove(Path::new(".gitignore"));
        }
        assert_eq!(now, before, "{seat}: a repo file changed");
        assert!(!repo.join("pwned.txt").exists());
    }
    answers.sort();
    answers.dedup();
    assert_eq!(
        answers.len(),
        3,
        "each seat answers with its own mock's answer"
    );
}

/// A NEW gitignored path counts: a mock that creates `out/new.txt` in a repo whose .gitignore ignores `out/` fails
/// the call `read_only_violation`.
/// RED IF: the git-mode snapshot drops `--ignored=matching`, so writes into ignored paths are invisible.
#[tokio::test]
async fn a_new_gitignored_write_is_a_violation() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    let repo = rig.root.join("ignored");
    git_repo(&repo, &[(".gitignore", "out/\n"), ("README.md", "hello\n")]);
    rig.set_action("kimi", "mkdir -p out && echo x > out/new.txt\n");
    rig.start_daemon().await;
    let err = rig
        .ask_in("kimi", "review", &repo)
        .await
        .expect_err("violation");
    assert!(err.contains("read_only_violation:kimi: out/;"), "{err}");
    assert!(repo.join("out/new.txt").is_file());
}

/// With MISTRAL_API_KEY set in the test process (and so in the daemon), the mistral mock's recorded env has HOME ==
/// its seat home and no MISTRAL_API_KEY, through the seatbelt wrap on macOS.
/// RED IF: the wrap drops the child env (HOME reverts) or the scrub is skipped.
#[tokio::test]
async fn the_wrapped_child_keeps_the_seat_env() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    // SAFETY: ENV_LOCK is held.
    unsafe {
        std::env::set_var("MISTRAL_API_KEY", fake_key("metered-"));
    }
    rig.start_daemon().await;
    let result = rig.ask("mistral", "q").await;
    // SAFETY: ENV_LOCK is held.
    unsafe {
        std::env::remove_var("MISTRAL_API_KEY");
    }
    result.unwrap_or_else(|e| panic!("{e}"));
    let env = rig.recorded_env("mistral");
    assert_eq!(
        env.get("HOME").map(PathBuf::from),
        Some(rig.th.join("homes/mistral"))
    );
    assert!(!env.contains_key("MISTRAL_API_KEY"), "{env:?}");
}

/// A clean read-only call (the mock cats a repo file and writes nothing) returns the mock's answer verbatim.
/// RED IF: the runner reports `read_only_violation` (or any error) for a call that changed nothing.
#[tokio::test]
async fn a_clean_read_only_call_returns_the_answer() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    let repo = rig.root.join("clean");
    git_repo(&repo, &[("README.md", "the readme\n")]);
    rig.set_action("kimi", "cat README.md > \"$D/catted\"\n");
    rig.start_daemon().await;
    let resp = rig
        .ask_in("kimi", "review", &repo)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resp["response"], fixture("kimi").0);
    assert_eq!(rig.mock_file("kimi", "catted"), "the readme\n");
}

/// A cwd that is not a git repo falls back to the walk: a written file is a `read_only_violation`.
/// RED IF: a non-git cwd goes unaudited, or the walk misses a new file.
#[tokio::test]
async fn a_non_git_cwd_walk_detects_a_write() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    let dir = rig.root.join("plain");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("sub/a.txt"), "a\n").unwrap();
    let not_a_repo = Command::new("git")
        .arg("-C")
        .arg(&dir)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        !not_a_repo.success(),
        "the test dir must not be inside a git repo"
    );
    rig.set_action("kimi", "echo x > sub/a.txt; echo y > pwned.txt\n");
    rig.start_daemon().await;
    let err = rig
        .ask_in("kimi", "review", &dir)
        .await
        .expect_err("violation");
    assert!(
        err.contains("read_only_violation:kimi: pwned.txt, sub/a.txt;"),
        "{err}"
    );
}

/// A non-git cwd over the 10,000 entry cap fails `read_only_audit_unavailable:<seat>` and spawns nothing.
/// RED IF: the cap is missing (an unbounded walk), a partial snapshot is used, or the seat spawns unaudited.
#[tokio::test]
async fn over_the_walk_cap_is_audit_unavailable_and_spawns_nothing() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    let dir = rig.root.join("huge");
    fs::create_dir_all(&dir).unwrap();
    for i in 0..10_001 {
        fs::write(dir.join(format!("f{i}")), b"").unwrap();
    }
    rig.start_daemon().await;
    let err = rig
        .ask_in("kimi", "review", &dir)
        .await
        .expect_err("over the cap");
    assert!(
        err.contains("read_only_audit_unavailable:kimi: more than 10000 files;"),
        "{err}"
    );
    assert_eq!(rig.spawns("kimi"), 0, "nothing spawned");
}

// ---------------------------------------------------------------------------------------------------------------
// T-207 (REQ-053, REQ-051): one call per seat at a time (the seat lock), and the stall window applied to new seats.
// ---------------------------------------------------------------------------------------------------------------

/// The mock records `start <pid> <ns>` before its work and `end <pid> <ns>` after it, in `$D/times`.
fn timed_action(work: &str) -> String {
    format!(
        "echo \"start $$ $(/bin/date +%s%N)\" >> \"$D/times\"\n{work}\necho \"end $$ $(/bin/date +%s%N)\" >> \"$D/times\"\n"
    )
}

/// Each mock run's [start, end] in wall-clock nanoseconds, keyed by its pid, in start order. `end` is None for a run
/// that never reached its end line (killed).
fn intervals(rig: &Rig, seat: &str) -> Vec<(u128, Option<u128>)> {
    let text = rig.mock_file(seat, "times");
    let mut runs: Vec<(String, u128, Option<u128>)> = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let [kind, pid, ns] = parts[..] else {
            panic!("bad times line {line:?}")
        };
        let ns: u128 = ns.parse().unwrap_or_else(|_| panic!("bad ns {line:?}"));
        match kind {
            "start" => runs.push((pid.to_string(), ns, None)),
            "end" => {
                let run = runs
                    .iter_mut()
                    .find(|r| r.0 == pid)
                    .unwrap_or_else(|| panic!("end before start: {text}"));
                run.2 = Some(ns);
            }
            _ => panic!("bad times line {line:?}"),
        }
    }
    runs.sort_by_key(|r| r.1);
    runs.into_iter().map(|(_, s, e)| (s, e)).collect()
}

fn now_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// A fresh empty cwd per call, so neither the daemon's ledger nor a concurrent call can change a cwd under audit,
/// and two calls never share a project lane.
fn fresh_cwd(rig: &Rig, name: &str) -> PathBuf {
    let dir = rig.root.join(name);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Non-blocking probe on a fresh fd of `$TRIUMVIRATE_HOME/locks/<seat>` (seat_lock.rs's own probe pattern): true
/// when someone else holds the seat right now. The probe's own lock, if taken, is released when the file drops.
fn seat_held(rig: &Rig, seat: &str) -> bool {
    use std::os::fd::AsRawFd;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(rig.th.join("locks").join(seat))
        .expect("the lock file exists once a call has taken the seat");
    // SAFETY: the fd belongs to `file`, alive for this call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    rc != 0
}

/// Wait until the seat's mock has written `n` start lines.
async fn wait_for_starts(rig: &Rig, seat: &str, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while rig.mock_file(seat, "times").matches("start ").count() < n {
        assert!(
            Instant::now() < deadline,
            "{seat}: mock never started: {}",
            rig.log_text()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Two concurrent calls to one seat, one plain ask_agent and one on the Jury surface, against a mock that records its
/// start and end and sleeps 1 s: the two [start, end] intervals never overlap, and both calls return the answer.
/// RED IF: the seat lock is missing or not held for the child's whole life (the mocks overlap), or a queued call is
/// failed instead of waiting its turn.
#[tokio::test]
async fn one_seat_runs_one_call_at_a_time() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.set_action("kimi", &timed_action("sleep 1"));
    rig.start_daemon().await;
    let (a, b) = (fresh_cwd(&rig, "cwd-a"), fresh_cwd(&rig, "cwd-b"));
    let (ra, rb) = tokio::join!(
        rig.ask_in("kimi", "review a", &a),
        rig.ask_jury_in("kimi", "review b", &b)
    );
    for r in [ra, rb] {
        let resp = r.unwrap_or_else(|e| panic!("{e}\n{}", rig.log_text()));
        assert_eq!(resp["response"], fixture("kimi").0);
    }
    let runs = intervals(&rig, "kimi");
    assert_eq!(runs.len(), 2, "two mock runs: {runs:?}");
    let (s1, e1) = (runs[0].0, runs[0].1.expect("first run ended"));
    let (s2, e2) = (runs[1].0, runs[1].1.expect("second run ended"));
    assert!(s1 < e1 && s2 < e2, "{runs:?}");
    assert!(
        e1 <= s2,
        "the two kimi runs overlapped: first [{s1}, {e1}], second [{s2}, {e2}]"
    );
}

/// While a call's mock is sleeping, a non-blocking probe on the seat's lock file reports it taken; after the call
/// returns normally the same probe acquires it.
/// RED IF: the lock is not held while the child runs, or is leaked after a normal return.
#[tokio::test]
async fn the_seat_lock_is_held_during_the_call_and_free_after() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.set_action("kimi", &timed_action("sleep 2"));
    rig.start_daemon().await;
    let cwd = fresh_cwd(&rig, "cwd-held");
    let call = rig.ask_in("kimi", "review", &cwd);
    let probe = async {
        wait_for_starts(&rig, "kimi", 1).await;
        let held = seat_held(&rig, "kimi");
        let ended = rig.mock_file("kimi", "times").contains("end ");
        (held, ended)
    };
    let (resp, (held, ended_before_probe)) = tokio::join!(call, probe);
    assert!(!ended_before_probe, "the probe ran while the mock slept");
    assert!(held, "the seat lock must be held while the mock runs");
    let resp = resp.unwrap_or_else(|e| panic!("{e}\n{}", rig.log_text()));
    assert_eq!(resp["response"], fixture("kimi").0);
    assert!(
        !seat_held(&rig, "kimi"),
        "the seat lock must be free after the call returns"
    );
}

/// Two concurrent calls to one seat with a 2 s stall window. The first call's mock prints nothing and sleeps (it is
/// stall-killed); the second is sent while the first is running. The second mock starts only after the first was
/// killed (it waited at least the first's stall window), and it answers: the lock was released on the failure path.
/// RED IF: a stall kill leaks the seat lock (the second call hangs or fails), or the second call does not wait.
#[tokio::test]
async fn a_stall_killed_call_releases_the_seat() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.extra_env
        .push(("TRIUMVIRATE_STALL_SECS_KIMI".into(), "2".into()));
    // The first run to get the seat (mkdir is atomic) sleeps silently; any later run answers at once.
    rig.set_action(
        "kimi",
        &timed_action("if mkdir \"$D/first.d\" 2>/dev/null; then sleep 60; fi"),
    );
    rig.start_daemon().await;
    let (a, b) = (fresh_cwd(&rig, "cwd-a"), fresh_cwd(&rig, "cwd-b"));
    let first = async {
        let r = rig.ask_in("kimi", "first", &a).await;
        (r, now_ns())
    };
    let second = async {
        wait_for_starts(&rig, "kimi", 1).await;
        // Sent 1 s into the first run, so it is queued behind it.
        tokio::time::sleep(Duration::from_secs(1)).await;
        rig.ask_in("kimi", "second", &b).await
    };
    let ((ra, a_failed_ns), rb) = tokio::join!(first, second);
    let err = ra.expect_err("the first call is stall-killed");
    assert!(err.contains("stalled after 2 s"), "{err}");
    let resp = rb.unwrap_or_else(|e| panic!("second call: {e}\n{}", rig.log_text()));
    assert_eq!(resp["response"], fixture("kimi").0);
    let runs = intervals(&rig, "kimi");
    assert_eq!(runs.len(), 2, "{runs:?}");
    let (a_start, a_end) = runs[0];
    let b_start = runs[1].0;
    assert!(
        a_end.is_none(),
        "the first mock was killed before its end line"
    );
    // The first call cannot be killed before its 2 s window runs out; its window starts just before its mock does,
    // so 300 ms covers the spawn. Without the lock the second mock would start about 1 s after the first.
    assert!(
        b_start >= a_start + 1_700_000_000,
        "the second mock started {} ms after the first, before the first's stall kill",
        (b_start - a_start) / 1_000_000
    );
    // And it started close to the moment the first call failed, not at some later timeout.
    assert!(
        b_start <= a_failed_ns + 5_000_000_000,
        "second start {b_start}, first failed {a_failed_ns}"
    );
}

/// A call queued on the seat lock behind a long, progressing run is NOT stall-killed while it waits: the queued call
/// emits a heartbeat (TRIUMVIRATE_SEAT_QUEUE_HEARTBEAT_SECS) that the stall clock reads as progress.
/// RED IF: the lock wait counts as silence, so a call queued longer than the stall window fails "stalled after" before
/// its mock ever ran.
#[tokio::test]
async fn a_queued_call_is_not_stall_killed_while_waiting() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.extra_env
        .push(("TRIUMVIRATE_STALL_SECS_KIMI".into(), "3".into()));
    rig.extra_env
        .push(("TRIUMVIRATE_SEAT_QUEUE_HEARTBEAT_SECS".into(), "1".into()));
    // The first run streams a progress line every second for 6 s (never stalled); later runs answer at once.
    rig.set_action(
        "kimi",
        &timed_action(
            "if mkdir \"$D/first.d\" 2>/dev/null; then for i in 1 2 3 4 5 6; do echo '{\"role\":\"assistant\",\"content\":\"tick\"}'; sleep 1; done; fi",
        ),
    );
    rig.start_daemon().await;
    let (a, b) = (fresh_cwd(&rig, "cwd-a"), fresh_cwd(&rig, "cwd-b"));
    let first = rig.ask_in("kimi", "first", &a);
    let second = async {
        wait_for_starts(&rig, "kimi", 1).await;
        rig.ask_in("kimi", "second", &b).await
    };
    let (ra, rb) = tokio::join!(first, second);
    ra.unwrap_or_else(|e| panic!("first call (it streamed progress): {e}\n{}", rig.log_text()));
    let resp =
        rb.unwrap_or_else(|e| panic!("queued second call was killed: {e}\n{}", rig.log_text()));
    assert_eq!(resp["response"], fixture("kimi").0);
    let runs = intervals(&rig, "kimi");
    assert_eq!(runs.len(), 2, "{runs:?}");
    // It really was queued longer than the 3 s window: the second mock started about 6 s after the first.
    assert!(
        runs[1].0 >= runs[0].0 + 4_000_000_000,
        "second started {} ms after the first",
        (runs[1].0 - runs[0].0) / 1_000_000
    );
}

/// A kimi call and a qwen call overlap in time: the lock is per seat, not global.
/// RED IF: the lock becomes global (one file for every seat) or new seats are serialized some other way.
#[tokio::test]
async fn different_seats_run_concurrently() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    for seat in ["kimi", "qwen"] {
        rig.set_action(seat, &timed_action("sleep 1"));
    }
    rig.start_daemon().await;
    let (k, q) = (fresh_cwd(&rig, "cwd-k"), fresh_cwd(&rig, "cwd-q"));
    let (rk, rq) = tokio::join!(rig.ask_in("kimi", "k", &k), rig.ask_in("qwen", "q", &q));
    for (seat, r) in [("kimi", rk), ("qwen", rq)] {
        let resp = r.unwrap_or_else(|e| panic!("{seat}: {e}\n{}", rig.log_text()));
        assert_eq!(resp["response"], fixture(seat).0);
    }
    let kimi = intervals(&rig, "kimi");
    let qwen = intervals(&rig, "qwen");
    assert_eq!((kimi.len(), qwen.len()), (1, 1));
    let (ks, ke) = (kimi[0].0, kimi[0].1.unwrap());
    let (qs, qe) = (qwen[0].0, qwen[0].1.unwrap());
    assert!(
        ks < qe && qs < ke,
        "kimi [{ks}, {ke}] and qwen [{qs}, {qe}] must overlap"
    );
}

/// A mock that prints nothing and sleeps 60 s is killed and the call fails `stalled after <n> s`, and the time to
/// failure tracks TRIUMVIRATE_STALL_SECS_KIMI: about 2 s with =2, about 5 s with =5.
/// RED IF: new seats get no stall kill (the call waits out the 60 s connector timeout), the window is a constant, or
/// it is read from anywhere but `mcp_bridge::stall::stall_window`.
#[tokio::test]
async fn the_stall_kill_follows_the_seat_window() {
    let _g = ENV_LOCK.lock().await;
    let mut took = Vec::new();
    for secs in [2u64, 5] {
        let mut rig = Rig::new();
        rig.extra_env
            .push(("TRIUMVIRATE_STALL_SECS_KIMI".into(), secs.to_string()));
        rig.set_action("kimi", "sleep 60\n");
        rig.start_daemon().await;
        let cwd = fresh_cwd(&rig, "cwd-stall");
        let started = Instant::now();
        let err = rig
            .ask_in("kimi", "review", &cwd)
            .await
            .expect_err("a silent mock is stall-killed");
        let elapsed = started.elapsed();
        assert!(err.contains("stalled after"), "{err}");
        assert!(err.contains(&format!("stalled after {secs} s")), "{err}");
        took.push(elapsed);
    }
    let (two, five) = (took[0], took[1]);
    assert!(
        two >= Duration::from_millis(1_800) && two < Duration::from_millis(4_500),
        "=2 took {two:?}"
    );
    assert!(
        five >= Duration::from_millis(4_800) && five < Duration::from_millis(9_000),
        "=5 took {five:?}"
    );
    assert!(five > two + Duration::from_secs(2), "{two:?} then {five:?}");
}

/// A new-seat record is Pending before its first turn, then the next turn resumes the published vendor id.
/// RED IF: HTTP spawn starts Ready, the first id is not persisted, or the second ask starts a fresh Kimi session.
#[tokio::test]
async fn http_new_seat_starts_pending_then_resumes_published_id() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    let spawn = client()
        .post(format!("{}/session/spawn", rig.base()))
        .bearer_auth(token.trim())
        .json(&serde_json::json!({"agent":"kimi","name":"stateful","cwd":rig.cwd}))
        .send()
        .await
        .unwrap();
    assert!(
        spawn.status().is_success(),
        "{}",
        spawn.text().await.unwrap()
    );
    let persisted: Value =
        serde_json::from_str(&fs::read_to_string(rig.th.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(persisted["stateful"]["phase"], "pending");

    for _ in 0..2 {
        let ask = client()
            .post(format!("{}/session/ask", rig.base()))
            .bearer_auth(token.trim())
            .json(&serde_json::json!({"name":"stateful","message":"review"}))
            .send()
            .await
            .unwrap();
        assert!(ask.status().is_success(), "{}", ask.text().await.unwrap());
    }
    let argv = rig.recorded_argv("kimi");
    let resume = argv.iter().position(|arg| arg == "--session").unwrap();
    assert_eq!(
        argv.get(resume + 1).map(String::as_str),
        Some("sess-kimi-fixture-0001")
    );
    let persisted: Value =
        serde_json::from_str(&fs::read_to_string(rig.th.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(persisted["stateful"]["phase"], "ready");
    assert_eq!(persisted["stateful"]["generation"], 2);
}

/// A successful first turn without a vendor id returns its answer marked non-resumable, then HTTP rejects the
/// broken session with 409 and does not spawn the mock again.
/// RED IF: missing_session_id is silent, is not persisted, has the wrong HTTP status, or allows another child.
#[tokio::test]
async fn http_missing_first_id_breaks_session_and_returns_conflict_next() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    let stdout = KIMI_STREAM
        .lines()
        .filter(|line| !line.contains("session.resume_hint"))
        .collect::<Vec<_>>()
        .join("\n");
    rig.set_fixture("kimi", &stdout, 0, "", None);
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    let spawn = client()
        .post(format!("{}/session/spawn", rig.base()))
        .bearer_auth(token.trim())
        .json(&serde_json::json!({"agent":"kimi","name":"missing","cwd":rig.cwd}))
        .send()
        .await
        .unwrap();
    assert!(spawn.status().is_success());
    let first = client()
        .post(format!("{}/session/ask", rig.base()))
        .bearer_auth(token.trim())
        .json(&serde_json::json!({"name":"missing","message":"review"}))
        .send()
        .await
        .unwrap();
    assert!(first.status().is_success());
    let first_body: Value = first.json().await.unwrap();
    assert!(
        first_body["response"]
            .as_str()
            .unwrap()
            .contains("session not resumable: missing_session_id:kimi")
    );
    assert_eq!(rig.spawns("kimi"), 1);

    let second = client()
        .post(format!("{}/session/ask", rig.base()))
        .bearer_auth(token.trim())
        .json(&serde_json::json!({"name":"missing","message":"again"}))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), reqwest::StatusCode::CONFLICT);
    let second_body: Value = second.json().await.unwrap();
    assert_eq!(
        second_body["error"],
        "session_broken: missing_session_id:kimi; dismiss_session and spawn a new one"
    );
    assert_eq!(rig.spawns("kimi"), 1);
}

/// RED IF: two HTTP turns for one new-seat session overlap, or an HTTP turn overlaps an in-process turn.
#[tokio::test]
async fn t208_1_same_session_http_and_cross_surface_turns_never_overlap() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.set_action(
        "kimi",
        "if ! mkdir \"$TRIUMVIRATE_HOME/http-critical\"; then touch \"$TRIUMVIRATE_HOME/http-overlap\"; fi\ntouch \"$TRIUMVIRATE_HOME/http-active\"\nsleep 1\nrm -f \"$TRIUMVIRATE_HOME/http-active\"\nrmdir \"$TRIUMVIRATE_HOME/http-critical\" 2>/dev/null || true",
    );
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    http_spawn(
        &rig,
        token.trim(),
        "kimi",
        "http-serial",
        &rig.fresh_cwd("http-serial"),
    )
    .await;
    let (first, second) = tokio::join!(
        http_ask(&rig, token.trim(), "http-serial", "one"),
        http_ask(&rig, token.trim(), "http-serial", "two")
    );
    assert!(
        first.0.is_success() && second.0.is_success(),
        "{first:?} {second:?}"
    );
    assert!(!rig.th.join("http-overlap").exists());

    http_spawn(
        &rig,
        token.trim(),
        "kimi",
        "mixed-serial",
        &rig.fresh_cwd("mixed-serial"),
    )
    .await;
    IN_PROCESS_SAW_HTTP_ACTIVE
        .get_or_init(|| StdMutex::new(Vec::new()))
        .lock()
        .unwrap()
        .clear();
    let sessions = Arc::new(tokio::sync::Mutex::new(
        HashMap::<String, SessionState>::new(),
    ));
    let sessions_file = daemon_core::sessions_file_path(&rig.th);
    let request = AskSessionRequest {
        name: "mixed-serial".to_string(),
        message: "mixed".to_string(),
        ..Default::default()
    };
    let http = http_ask(&rig, token.trim(), "mixed-serial", "hold");
    let in_process = async {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !rig.th.join("http-active").exists() {
            assert!(
                Instant::now() < deadline,
                "HTTP mock never entered its critical section"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        mcp_tools::inter_agent::ask_session(
            &sessions,
            Some(&sessions_file),
            &request,
            false,
            recording_in_process_executor,
        )
        .await
    };
    let (http_result, in_process_result) = tokio::join!(http, in_process);
    assert!(http_result.0.is_success(), "{http_result:?}");
    assert!(in_process_result.is_ok(), "{in_process_result:?}");
    assert_eq!(
        *IN_PROCESS_SAW_HTTP_ACTIVE.get().unwrap().lock().unwrap(),
        vec![false]
    );
}

/// RED IF: persisted vendor ids do not cross from HTTP to in-process asks and back with exact resume values.
#[tokio::test]
async fn t208_1b_cross_surface_continuity_uses_exact_persisted_id() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    let sessions_file = daemon_core::sessions_file_path(&rig.th);

    http_spawn(
        &rig,
        token.trim(),
        "kimi",
        "http-first",
        &rig.fresh_cwd("http-first"),
    )
    .await;
    assert!(
        http_ask(&rig, token.trim(), "http-first", "publish")
            .await
            .0
            .is_success()
    );
    IN_PROCESS_PRIOR_IDS
        .get_or_init(|| StdMutex::new(Vec::new()))
        .lock()
        .unwrap()
        .clear();
    let sessions = Arc::new(tokio::sync::Mutex::new(
        HashMap::<String, SessionState>::new(),
    ));
    mcp_tools::inter_agent::ask_session(
        &sessions,
        Some(&sessions_file),
        &AskSessionRequest {
            name: "http-first".to_string(),
            message: "resume".to_string(),
            ..Default::default()
        },
        false,
        recording_in_process_executor,
    )
    .await
    .unwrap();
    assert_eq!(
        *IN_PROCESS_PRIOR_IDS.get().unwrap().lock().unwrap(),
        vec![Some("sess-kimi-fixture-0001".to_string())]
    );

    http_spawn(
        &rig,
        token.trim(),
        "kimi",
        "process-first",
        &rig.fresh_cwd("process-first"),
    )
    .await;
    let sessions = Arc::new(tokio::sync::Mutex::new(
        HashMap::<String, SessionState>::new(),
    ));
    mcp_tools::inter_agent::ask_session(
        &sessions,
        Some(&sessions_file),
        &AskSessionRequest {
            name: "process-first".to_string(),
            message: "publish".to_string(),
            ..Default::default()
        },
        false,
        recording_in_process_executor,
    )
    .await
    .unwrap();
    assert!(
        http_ask(&rig, token.trim(), "process-first", "resume")
            .await
            .0
            .is_success()
    );
    let argv = rig.recorded_argv("kimi");
    let flag = argv.iter().position(|arg| arg == "--session").unwrap();
    assert_eq!(
        argv.get(flag + 1).map(String::as_str),
        Some("in-process-id")
    );
}

/// RED IF: a held session lock does not return HTTP 409 session_busy, or the rejected turn spawns a child.
#[tokio::test]
async fn t208_2b_http_lock_timeout_is_conflict_and_spawns_nothing() {
    let _g = ENV_LOCK.lock().await;
    unsafe { std::env::set_var("TRIUMVIRATE_SESSION_BUSY_SECS", "1") };
    let mut rig = Rig::new();
    rig.set_action("kimi", "sleep 3");
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    http_spawn(&rig, token.trim(), "kimi", "busy", &rig.fresh_cwd("busy")).await;
    let first = http_ask(&rig, token.trim(), "busy", "hold");
    let second = async {
        let deadline = Instant::now() + Duration::from_secs(5);
        while rig.spawns("kimi") == 0 {
            assert!(Instant::now() < deadline, "first mock never spawned");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        http_ask(&rig, token.trim(), "busy", "blocked").await
    };
    let (first, second) = tokio::join!(first, second);
    unsafe { std::env::remove_var("TRIUMVIRATE_SESSION_BUSY_SECS") };
    assert!(first.0.is_success(), "{first:?}");
    assert_eq!(second.0, reqwest::StatusCode::CONFLICT);
    assert_eq!(second.1["error"], "session_busy: busy");
    assert_eq!(rig.spawns("kimi"), 1);
}

/// RED IF: HTTP spawn does not persist a Pending record before any ask for every new seat.
#[tokio::test]
async fn t208_2c_http_spawn_persists_pending_before_first_ask() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    for seat in SEATS {
        http_spawn(
            &rig,
            token.trim(),
            seat,
            seat,
            &rig.fresh_cwd(&format!("pending-{seat}")),
        )
        .await;
        let persisted: Value =
            serde_json::from_str(&fs::read_to_string(rig.th.join("sessions.json")).unwrap())
                .unwrap();
        assert_eq!(persisted[seat]["phase"], "pending", "{seat}");
        assert_eq!(rig.spawns(seat), 0, "{seat}");
    }
}

/// RED IF: locks for different session names and seats serialize independent child intervals.
#[tokio::test]
async fn t208_3_different_sessions_on_different_seats_overlap() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    let action = "if ! mkdir \"$TRIUMVIRATE_HOME/independent-active\"; then touch \"$TRIUMVIRATE_HOME/independent-overlap\"; fi\nsleep 1\nrmdir \"$TRIUMVIRATE_HOME/independent-active\" 2>/dev/null || true";
    rig.set_action("kimi", action);
    rig.set_action("qwen", action);
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    http_spawn(
        &rig,
        token.trim(),
        "kimi",
        "independent-k",
        &rig.fresh_cwd("independent-k"),
    )
    .await;
    http_spawn(
        &rig,
        token.trim(),
        "qwen",
        "independent-q",
        &rig.fresh_cwd("independent-q"),
    )
    .await;
    let (kimi, qwen) = tokio::join!(
        http_ask(&rig, token.trim(), "independent-k", "one"),
        http_ask(&rig, token.trim(), "independent-q", "two")
    );
    assert!(
        kimi.0.is_success() && qwen.0.is_success(),
        "{kimi:?} {qwen:?}"
    );
    assert!(rig.th.join("independent-overlap").exists());
}

/// RED IF: a failed first Kimi turn does not break, block, dismiss, respawn Pending, and then recover.
#[tokio::test]
async fn t208_4_broken_first_turn_requires_dismiss_before_recovery() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.set_fixture("kimi", "", 1, "first turn exploded\n", None);
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    http_spawn(
        &rig,
        token.trim(),
        "kimi",
        "recover",
        &rig.fresh_cwd("recover-first"),
    )
    .await;
    let first = http_ask(&rig, token.trim(), "recover", "fail").await;
    assert_eq!(first.0, reqwest::StatusCode::BAD_GATEWAY);
    assert_eq!(rig.spawns("kimi"), 1);
    let blocked = http_ask(&rig, token.trim(), "recover", "blocked").await;
    assert_eq!(blocked.0, reqwest::StatusCode::CONFLICT);
    // The reason is the failed turn's first error line (the dispatcher wraps the vendor text), so assert the shape
    // and the vendor text rather than one exact wrapper.
    let blocked_err = blocked.1["error"].as_str().unwrap_or_default();
    assert!(
        blocked_err.starts_with("session_broken: ")
            && blocked_err.contains("first turn exploded")
            && blocked_err.ends_with("; dismiss_session and spawn a new one"),
        "{blocked_err}"
    );
    assert_eq!(rig.spawns("kimi"), 1);

    let dismissed = client()
        .post(format!("{}/session/dismiss", rig.base()))
        .bearer_auth(token.trim())
        .json(&serde_json::json!({"name":"recover"}))
        .send()
        .await
        .unwrap();
    assert!(dismissed.status().is_success());
    let (_, stdout, export) = fixture("kimi");
    rig.set_fixture("kimi", &stdout, 0, "", export.as_deref());
    http_spawn(
        &rig,
        token.trim(),
        "kimi",
        "recover",
        &rig.fresh_cwd("recover-second"),
    )
    .await;
    let persisted: Value =
        serde_json::from_str(&fs::read_to_string(rig.th.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(persisted["recover"]["phase"], "pending");
    assert!(
        http_ask(&rig, token.trim(), "recover", "works")
            .await
            .0
            .is_success()
    );
    assert_eq!(rig.spawns("kimi"), 2);
}

/// RED IF: a Ready session whose vendor rejects its id does not become Broken and block the next child.
#[tokio::test]
async fn t208_5_vendor_rejected_ready_session_becomes_broken() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    http_spawn(
        &rig,
        token.trim(),
        "kimi",
        "rejected",
        &rig.fresh_cwd("rejected"),
    )
    .await;
    assert!(
        http_ask(&rig, token.trim(), "rejected", "publish")
            .await
            .0
            .is_success()
    );
    rig.set_fixture("kimi", "", 1, "Session not found\n", None);
    assert_eq!(
        http_ask(&rig, token.trim(), "rejected", "reject").await.0,
        reqwest::StatusCode::BAD_GATEWAY
    );
    let persisted: Value =
        serde_json::from_str(&fs::read_to_string(rig.th.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(persisted["rejected"]["phase"], "broken");
    let reason = persisted["rejected"]["broken_reason"].as_str().unwrap_or_default();
    assert!(reason.contains("Session not found"), "{reason}");
    let blocked = http_ask(&rig, token.trim(), "rejected", "blocked").await;
    assert_eq!(blocked.0, reqwest::StatusCode::CONFLICT);
    assert_eq!(rig.spawns("kimi"), 2);
}

/// RED IF: any new seat treats a successful first turn without an id as resumable or spawns a second child.
#[tokio::test]
async fn t208_6_each_new_seat_missing_first_id_breaks_and_blocks() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.set_fixture(
        "kimi",
        &KIMI_STREAM
            .lines()
            .filter(|line| !line.contains("session.resume_hint"))
            .collect::<Vec<_>>()
            .join("\n"),
        0,
        "",
        None,
    );
    // Mistral's id lives in export.json, so "success without an id" is an export with no session_id (a missing
    // export is a different failure, missing_export:mistral).
    let mut export: Value = serde_json::from_str(MISTRAL_EXPORT).unwrap();
    export.as_object_mut().unwrap().remove("session_id");
    rig.set_fixture("mistral", MISTRAL_STREAM, 0, "", Some(&export.to_string()));
    rig.set_fixture(
        "qwen",
        &QWEN_STREAM
            .lines()
            .filter(|line| !line.contains("session_start"))
            .collect::<Vec<_>>()
            .join("\n"),
        0,
        "",
        None,
    );
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    for seat in SEATS {
        let name = format!("missing-{seat}");
        http_spawn(&rig, token.trim(), seat, &name, &rig.fresh_cwd(&name)).await;
        let first = http_ask(&rig, token.trim(), &name, "first").await;
        assert!(first.0.is_success(), "{seat}: {first:?}");
        assert!(
            first.1["response"]
                .as_str()
                .unwrap()
                .contains(&format!("session not resumable: missing_session_id:{seat}")),
            "{seat}: {first:?}"
        );
        let blocked = http_ask(&rig, token.trim(), &name, "blocked").await;
        assert_eq!(blocked.0, reqwest::StatusCode::CONFLICT, "{seat}");
        assert_eq!(rig.spawns(seat), 1, "{seat}");
    }
}

/// RED IF: any new seat fails to publish its exact id or uses the wrong resume flag and value on the next turn.
#[tokio::test]
async fn t208_7_each_new_seat_publishes_and_resumes_exact_id() {
    let _g = ENV_LOCK.lock().await;
    let mut rig = Rig::new();
    rig.start_daemon().await;
    let token = fs::read_to_string(rig.th.join("daemon.token")).unwrap();
    for (seat, flag, id) in [
        ("kimi", "--session", "sess-kimi-fixture-0001"),
        ("mistral", "--resume", "vibe-sess-7f3a"),
        ("qwen", "--resume", "7f3c9a2e-5b1d-4e8a-9c40-2d6f1e8b7a01"),
    ] {
        let name = format!("resume-{seat}");
        http_spawn(&rig, token.trim(), seat, &name, &rig.fresh_cwd(&name)).await;
        let first = http_ask(&rig, token.trim(), &name, "first").await;
        assert!(first.0.is_success(), "{seat}: {first:?}");
        let persisted: Value =
            serde_json::from_str(&fs::read_to_string(rig.th.join("sessions.json")).unwrap())
                .unwrap();
        assert_eq!(persisted[&name]["cli_session_id"], id, "{seat}");
        let second = http_ask(&rig, token.trim(), &name, "second").await;
        assert!(second.0.is_success(), "{seat}: {second:?}");
        let argv = rig.recorded_argv_for(seat, 2);
        let position = argv.iter().position(|arg| arg == flag).unwrap();
        assert_eq!(
            argv.get(position + 1).map(String::as_str),
            Some(id),
            "{seat}"
        );
    }
}
