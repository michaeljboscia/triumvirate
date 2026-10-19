//! Seven Primes live E2E (REQ-081, T-302): each new seat against the REAL daemon and the REAL vendor CLI.
//!
//! Every test is `#[ignore]` and also gated by `TRIUMVIRATE_LIVE_<SEAT>=1`, because each one spends subscription
//! quota. `scripts/verify-live-agents.sh` (T-301) runs them per AVAILABLE seat:
//! `TRIUMVIRATE_LIVE_<SEAT>=1 cargo test -p triumvirate --test integration_seven_primes_live -- --ignored <seat>`.
//! Test names start with the seat so that filter selects exactly that seat's tests.
//!
//! The daemon is the one at `TRIUMVIRATE_DAEMON_URL` (default http://127.0.0.1:8080), authenticated with
//! `$TRIUMVIRATE_HOME/daemon.token` (default ~/.triumvirate). The token is read, never printed.
//!
//! Every repo these tests create lives under CARGO_TARGET_TMPDIR, which must NOT be under /private/tmp or
//! /var/folders: the seatbelt allows writes under those temp roots (LESSONS.md 20), so a canary there would prove
//! nothing.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn live(seat: &str) -> bool {
    let var = format!("TRIUMVIRATE_LIVE_{}", seat.to_ascii_uppercase());
    let on = std::env::var(&var).is_ok_and(|v| v == "1");
    if !on {
        eprintln!("SKIP: {var}=1 is not set");
    }
    on
}

fn base() -> String {
    std::env::var("TRIUMVIRATE_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string())
}

fn token() -> String {
    let home = std::env::var("TRIUMVIRATE_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").expect("HOME")).join(".triumvirate")
    });
    std::fs::read_to_string(home.join("daemon.token"))
        .expect("daemon.token readable")
        .trim()
        .to_string()
}

fn unique(label: &str) -> String {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    format!("{label}-{}-{n}", std::process::id())
}

/// A fresh git repo under CARGO_TARGET_TMPDIR (never a seatbelt-writable temp root).
fn repo(label: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("seven-primes-live").join(unique(label));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = std::fs::canonicalize(&dir).unwrap();
    let s = dir.to_string_lossy();
    assert!(
        !s.starts_with("/private/tmp") && !s.starts_with("/private/var/folders") && !s.starts_with("/tmp"),
        "live repos must not sit under a seatbelt-writable temp root: {s}"
    );
    std::fs::write(dir.join("README.md"), "live fixture repo\n").unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["add", "."],
        vec!["-c", "user.email=live@example.invalid", "-c", "user.name=live", "commit", "-qm", "base"],
    ] {
        assert!(Command::new("git").arg("-C").arg(&dir).args(&args).status().unwrap().success());
    }
    dir
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(900)).build().unwrap()
}

async fn post(path: &str, body: Value) -> (u16, Value) {
    let r = client()
        .post(format!("{}{path}", base()))
        .bearer_auth(token())
        .json(&body)
        .send()
        .await
        .expect("daemon reachable");
    let status = r.status().as_u16();
    let text = r.text().await.unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn answer(v: &Value) -> String {
    v["response"].as_str().unwrap_or_default().to_string()
}

/// REQ-081 (1): a question whose answer is only in a named file is answered, and the sight gate (required_sources)
/// accepts the call, so the seat really opened the file.
async fn file_grounded_answer(seat: &str) {
    let dir = repo(&format!("{seat}-sight"));
    let token = unique("TOKEN").replace('-', "");
    let file = dir.join("secret.txt");
    std::fs::write(&file, format!("The token is {token}\n")).unwrap();
    let (status, out) = post(
        "/ask-agent",
        serde_json::json!({
            "agent": seat,
            "message": "Read secret.txt in this directory and reply with only the token it contains.",
            "cwd": dir,
            "strict_agent": true,
            "required_sources": [file],
        }),
    )
    .await;
    assert_eq!(status, 200, "{seat}: {out}");
    assert!(answer(&out).contains(&token), "{seat}: {out}");
}

/// REQ-081 (2), REQ-035: an explicit request to write a canary file leaves no file, and the call fails
/// read_only_violation only if a repo mutation actually landed (it must not land).
async fn canary_write_leaves_no_file(seat: &str) {
    let dir = repo(&format!("{seat}-canary"));
    let (status, out) = post(
        "/ask-agent",
        serde_json::json!({
            "agent": seat,
            "message": "Create a file named canary.txt in this directory containing the word CANARY, then reply DONE.",
            "cwd": dir,
            "strict_agent": true,
        }),
    )
    .await;
    assert!(!dir.join("canary.txt").exists(), "{seat} wrote the canary: {out}");
    let text = out.to_string();
    assert!(!text.contains("read_only_violation"), "{seat}: a mutation landed: {out}");
    let _ = status;
}

/// REQ-081 (3), REQ-067: a second turn in a named session resumes the same vendor session (recalls the first turn).
async fn second_turn_resumes(seat: &str) {
    let dir = repo(&format!("{seat}-session"));
    let name = unique(&format!("live-{seat}"));
    let code = unique("CODE").replace('-', "");
    let (status, out) = post(
        "/session/spawn",
        serde_json::json!({"agent": seat, "name": name, "cwd": dir}),
    )
    .await;
    assert_eq!(status, 200, "{seat}: {out}");
    let (status, out) = post(
        "/session/ask",
        serde_json::json!({"name": name, "message": format!("Remember this codeword: {code}. Reply only with OK.")}),
    )
    .await;
    assert_eq!(status, 200, "{seat}: {out}");
    assert!(!answer(&out).contains("session not resumable"), "{seat}: {out}");
    let (status, out) = post(
        "/session/ask",
        serde_json::json!({"name": name, "message": "What codeword did I ask you to remember? Reply with only the codeword."}),
    )
    .await;
    assert_eq!(status, 200, "{seat}: {out}");
    assert!(answer(&out).contains(&code), "{seat} did not resume: {out}");
    let _ = post("/session/dismiss", serde_json::json!({"name": name})).await;
}

#[tokio::test]
#[ignore]
async fn kimi_live_file_grounded_answer_passes_sight() {
    if live("kimi") {
        file_grounded_answer("kimi").await;
    }
}

#[tokio::test]
#[ignore]
async fn kimi_live_canary_write_leaves_no_file() {
    if live("kimi") {
        canary_write_leaves_no_file("kimi").await;
    }
}

#[tokio::test]
#[ignore]
async fn kimi_live_second_turn_resumes() {
    if live("kimi") {
        second_turn_resumes("kimi").await;
    }
}

/// REQ-081 (Kimi): a forced first-turn failure leaves no resumable id: the session is Broken and the next ask is
/// refused session_broken without spawning. The failure is forced with an over-limit prompt (the builder refuses it).
#[tokio::test]
#[ignore]
async fn kimi_live_failed_first_turn_leaves_no_resumable_id() {
    if !live("kimi") {
        return;
    }
    let dir = repo("kimi-broken");
    let name = unique("live-kimi-broken");
    let (status, out) = post(
        "/session/spawn",
        serde_json::json!({"agent": "kimi", "name": name, "cwd": dir}),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    let (status, out) = post(
        "/session/ask",
        serde_json::json!({"name": name, "message": "x".repeat(70_000)}),
    )
    .await;
    assert_ne!(status, 200, "an over-limit first turn must fail: {out}");
    let (status, out) = post("/session/ask", serde_json::json!({"name": name, "message": "hi"})).await;
    assert_eq!(status, 409, "{out}");
    assert!(out.to_string().contains("session_broken"), "{out}");
    let _ = post("/session/dismiss", serde_json::json!({"name": name})).await;
}

#[tokio::test]
#[ignore]
async fn mistral_live_file_grounded_answer_passes_sight() {
    if live("mistral") {
        file_grounded_answer("mistral").await;
    }
}

#[tokio::test]
#[ignore]
async fn mistral_live_canary_write_leaves_no_file() {
    if live("mistral") {
        canary_write_leaves_no_file("mistral").await;
    }
}

#[tokio::test]
#[ignore]
async fn mistral_live_second_turn_resumes() {
    if live("mistral") {
        second_turn_resumes("mistral").await;
    }
}

/// REQ-081 (Mistral): the Python CLI runs (vendor bug #1239). The daemon sets VIBE_CLI=python in the child env and
/// `triumvirate doctor` checks the final child env; this asserts that check passes on this machine.
#[tokio::test]
#[ignore]
async fn mistral_live_runs_the_python_cli() {
    if !live("mistral") {
        return;
    }
    let bin = std::env::var("TRIUMVIRATE_BIN").unwrap_or_else(|_| env!("CARGO_BIN_EXE_triumvirate").to_string());
    let mut doctor = Command::new(bin);
    doctor.arg("doctor").env("TRIUMVIRATE_ENABLE_MISTRAL", "1");
    let out = doctor.output().expect("triumvirate doctor runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("mistral.vibe_cli_python: PASS"), "{text}");
}

#[tokio::test]
#[ignore]
async fn qwen_live_file_grounded_answer_passes_sight() {
    if live("qwen") {
        file_grounded_answer("qwen").await;
    }
}

#[tokio::test]
#[ignore]
async fn qwen_live_canary_write_leaves_no_file() {
    if live("qwen") {
        canary_write_leaves_no_file("qwen").await;
    }
}

#[tokio::test]
#[ignore]
async fn qwen_live_second_turn_resumes() {
    if live("qwen") {
        second_turn_resumes("qwen").await;
    }
}
