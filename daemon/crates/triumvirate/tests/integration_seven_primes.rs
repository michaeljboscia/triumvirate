//! Seven Primes T-205: the kimi, mistral and qwen dispatch arms, through the real `triumvirate` binary.
//!
//! Every test runs its own daemon (or `doctor --live`) with a cleared env: HOME and TRIUMVIRATE_HOME are the test's
//! tempdir, PATH is `/usr/bin:/bin` (no vendor CLI lives there, asserted below), and each seat's binary is a tempdir
//! mock named by TRIUMVIRATE_<SEAT>_BIN. A mock records its argv, its env and one line per spawn, then replays a
//! fixture from agent-adapter/tests/fixtures. No real kimi, vibe or qwen can run, and no real credential is read:
//! the Qwen key is a fake written into the tempdir seat home.

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

use mcp_bridge::containment::{SeatInvocation, SeatRequest};
use serde_json::Value;

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
const SHELL_ADDED: [&str; 5] = ["PWD", "SHLVL", "_", "OLDPWD", "__CF_USER_TEXT_ENCODING"];

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
: > "$D/argv"
for a in "$@"; do printf '%s\n' "$a" >> "$D/argv"; done
env > "$D/env"
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
        let token = fs::read_to_string(self.th.join("daemon.token")).expect("token");
        let resp = client()
            .post(format!("{}/ask-agent", self.base()))
            .bearer_auth(token.trim())
            .json(&serde_json::json!({"agent": seat, "message": message, "cwd": self.cwd}))
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
