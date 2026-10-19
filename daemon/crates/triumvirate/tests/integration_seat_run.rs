//! Seven Primes W-1: `triumvirate seat-run`, the writer runner for kimi, mistral and qwen, through the real binary.
//!
//! Every run has a cleared env: HOME and TRIUMVIRATE_HOME are the test's tempdir, PATH is `/usr/bin:/bin` (no vendor
//! CLI lives there, asserted below), and each seat's binary is a tempdir mock named by TRIUMVIRATE_<SEAT>_BIN, so no
//! real kimi, vibe or qwen can run. A mock records its pid, argv and the `.triumvirate/` listing it saw, sources a
//! per-test `action` snippet in its cwd (the worktree), then replays a fixture from agent-adapter/tests/fixtures.
//!
//! The bypass seam: the seatbelt allows the system temp roots, so a parent repo under a tempdir is writable by the
//! mock and only the escape audit can catch a write there. The real-seatbelt test puts the parent under
//! CARGO_TARGET_TMPDIR instead, outside every allowed root.

use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

mod capture_server;

const SEATS: [&str; 3] = ["kimi", "mistral", "qwen"];

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

/// A test-only plan value, built at runtime so nothing credential-shaped is committed.
fn fake_plan_value() -> String {
    ["sk-sp-", "w1fake", "0123456789"].concat()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("HOME", dir)
        .stdin(Stdio::null())
        .output()
        .expect("git must be on PATH");
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct World {
    _sys: tempfile::TempDir,
    _repos: Option<tempfile::TempDir>,
    root: PathBuf,
    home: PathBuf,
    th: PathBuf,
    parent: PathBuf,
    wt: PathBuf,
}

impl World {
    /// Mocks, HOME and TRIUMVIRATE_HOME under a system tempdir; the parent repo and its linked worktree (branch
    /// `writer`) under `repo_base` when given (outside the seatbelt's allowed roots), else under the same tempdir.
    fn new(repo_base: Option<&Path>) -> Self {
        for program in ["kimi", "vibe", "qwen"] {
            for dir in ["/usr/bin", "/bin"] {
                assert!(
                    !Path::new(dir).join(program).exists(),
                    "a real {program} in {dir} would be reachable from the test PATH"
                );
            }
        }
        let sys = tempfile::tempdir().expect("tempdir");
        let root = fs::canonicalize(sys.path()).unwrap();
        let (repos, repo_root) = match repo_base {
            Some(base) => {
                let t = tempfile::tempdir_in(base).expect("repo tempdir");
                let r = fs::canonicalize(t.path()).unwrap();
                (Some(t), r)
            }
            None => (None, root.clone()),
        };
        let home = root.join("home");
        let th = root.join("th");
        for d in [&home, &th] {
            fs::create_dir_all(d).unwrap();
        }
        let parent = repo_root.join("parent");
        fs::create_dir_all(&parent).unwrap();
        fs::write(parent.join("README.md"), "hello\n").unwrap();
        git(&parent, &["init", "-q"]);
        git(&parent, &["config", "user.name", "w1"]);
        git(&parent, &["config", "user.email", "w1@example.invalid"]);
        git(&parent, &["config", "commit.gpgsign", "false"]);
        git(&parent, &["add", "-A"]);
        git(&parent, &["commit", "-qm", "base"]);
        let wt = repo_root.join("wt-member-1");
        git(
            &parent,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "writer",
                &wt.to_string_lossy(),
            ],
        );
        let w = Self {
            _sys: sys,
            _repos: repos,
            root,
            home,
            th,
            parent,
            wt,
        };
        for seat in SEATS {
            w.write_mock(seat);
            w.write_verified(seat);
        }
        let qwen_home = w.th.join("homes/qwen");
        fs::create_dir_all(&qwen_home).unwrap();
        let creds = qwen_home.join("credentials.env");
        fs::write(
            &creds,
            format!("QWENCLOUD_TOKEN_PLAN_API_KEY={}\n", fake_plan_value()),
        )
        .unwrap();
        fs::set_permissions(&creds, fs::Permissions::from_mode(0o600)).unwrap();
        w
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
        let (stdout, export) = match seat {
            "kimi" => (KIMI_STREAM, None),
            "mistral" => (MISTRAL_STREAM, Some(MISTRAL_EXPORT)),
            _ => (QWEN_STREAM, None),
        };
        fs::write(d.join("stdout"), stdout).unwrap();
        if let Some(e) = export {
            fs::write(d.join("export.json"), e).unwrap();
        }
        let script = format!(
            r#"#!/bin/sh
D='{d}'
if [ "$1" = --version ]; then echo {v}; exit 0; fi
echo $$ > "$D/pid"
: > "$D/argv"
for a in "$@"; do printf '%s\n' "$a" >> "$D/argv"; done
ls -A .triumvirate > "$D/triumvirate_ls" 2>/dev/null
if [ -f "$D/action" ]; then . "$D/action"; fi
prev=""
for a in "$@"; do
  if [ "$prev" = --output-dir ] && [ -f "$D/export.json" ]; then cp "$D/export.json" "$a/export.json"; fi
  prev="$a"
done
cat "$D/stdout"
exit 0
"#,
            d = d.display(),
            v = version(seat)
        );
        let bin = self.mock_bin(seat);
        fs::write(&bin, script).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn write_verified(&self, seat: &str) {
        mcp_bridge::seat_state::write_verified(
            &self.th,
            seat,
            &mcp_bridge::seat_state::VerifiedRecord {
                cli_version: version(seat).into(),
                seat_home: self.th.join("homes").join(seat).display().to_string(),
                probed_at: "2026-10-19T00:00:00Z".into(),
                served_model: None,
            },
        )
        .unwrap();
    }

    fn set_action(&self, seat: &str, script: &str) {
        fs::write(self.mock_dir(seat).join("action"), script).unwrap();
    }

    fn mock_file(&self, seat: &str, name: &str) -> String {
        fs::read_to_string(self.mock_dir(seat).join(name)).unwrap_or_default()
    }

    fn prompt_file(&self, seat: &str) -> PathBuf {
        self.wt
            .join(".triumvirate")
            .join(format!("seat-prompt-{seat}.txt"))
    }

    fn head(&self) -> String {
        git(&self.wt, &["rev-parse", "HEAD"])
    }

    /// seat-run for `seat`, prompt file written first, every pipe captured.
    fn command(&self, seat: &str, prompt: &str, timeout_secs: u64) -> Command {
        let pf = self.prompt_file(seat);
        fs::create_dir_all(pf.parent().unwrap()).unwrap();
        fs::write(&pf, prompt).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_triumvirate"));
        cmd.args([
            "seat-run",
            "--seat",
            seat,
            "--surface",
            "fleet",
            "--worktree",
            &self.wt.to_string_lossy(),
            "--prompt-file",
            &pf.to_string_lossy(),
            "--commit-message",
            &format!("fleet({seat}): wt-member-1"),
            "--timeout-secs",
            &timeout_secs.to_string(),
        ])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", &self.home)
        .env("TRIUMVIRATE_HOME", &self.th)
        // Read by the mocks' actions; not a credential, so the scrub leaves it.
        .env("W1_PARENT", &self.parent)
        .current_dir(&self.wt)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        for s in SEATS {
            let upper = s.to_ascii_uppercase();
            cmd.env(format!("TRIUMVIRATE_ENABLE_{upper}"), "1");
            cmd.env(format!("TRIUMVIRATE_{upper}_BIN"), self.mock_bin(s));
        }
        for other in ["CODEX", "GEMINI", "AGY", "CLAUDE", "GROK"] {
            cmd.env(format!("TRIUMVIRATE_{other}_BIN"), "/usr/bin/false");
        }
        cmd
    }

    fn run(&self, seat: &str, prompt: &str) -> Output {
        self.command(seat, prompt, 60)
            .output()
            .expect("run seat-run")
    }
}

fn last_stderr_line(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .last()
        .unwrap_or_default()
        .to_string()
}

fn pid_alive(pid: &str) -> bool {
    Command::new("/bin/kill")
        .args(["-0", pid.trim()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Polls until the mock's pid is gone (it is SIGKILLed with its group; init reaps it).
fn assert_pid_gone(pid: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while pid_alive(pid) {
        assert!(Instant::now() < deadline, "mock pid {pid} still alive");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The parent repo is exactly its base commit, with nothing untracked.
fn assert_parent_untouched(w: &World) {
    assert_eq!(
        git(
            &w.parent,
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        ""
    );
    assert_eq!(
        fs::read_to_string(w.parent.join("README.md")).unwrap(),
        "hello\n"
    );
}

/// Per seat: the mock writes out.txt in its cwd; seat-run exits 0, prints `seat-run: committed <sha>` on stderr, and
/// the worktree branch gains exactly that commit with the given message and out.txt (no `.triumvirate/` file). The
/// seat got the WRITER argv and the prompt, never saw its prompt file, and the parent repo is unchanged.
/// RED IF: seat-run does not commit, commits the prompt file or anything under `.triumvirate/`, the vendor sees its
/// prompt file, a seat gets its read-only argv, or the parent repo changes.
#[test]
fn writer_commits_its_work_for_every_seat() {
    for seat in SEATS {
        let w = World::new(None);
        w.set_action(seat, &format!("echo hello-{seat} > out.txt\n"));
        let before = w.head();
        let out = w.run(seat, &format!("W1-PROMPT-{seat}"));
        let line = last_stderr_line(&out);
        assert_eq!(out.status.code(), Some(0), "{seat}: {line}\n{:?}", out);
        let sha = w.head();
        assert_ne!(sha, before, "{seat}: no new commit");
        assert_eq!(line, format!("seat-run: committed {sha}"));
        assert_eq!(
            git(&w.wt, &["log", "-1", "--format=%s"]),
            format!("fleet({seat}): wt-member-1")
        );
        assert_eq!(
            git(&w.wt, &["show", "--name-only", "--format=", "HEAD"]),
            "out.txt"
        );
        assert_eq!(git(&w.wt, &["rev-parse", "HEAD~1"]), before);
        assert_eq!(
            git(&w.wt, &["show", "HEAD:out.txt"]),
            format!("hello-{seat}")
        );
        // The prompt file is gone, was never visible to the seat, and is not in the commit.
        assert!(!w.prompt_file(seat).exists(), "{seat}: prompt file left");
        assert!(
            !w.mock_file(seat, "triumvirate_ls").contains("seat-prompt"),
            "{seat}: the seat saw its prompt file"
        );
        let argv = w.mock_file(seat, "argv");
        assert!(
            argv.contains(&format!("W1-PROMPT-{seat}")),
            "{seat}: {argv}"
        );
        let writer_marker = match seat {
            "kimi" => "rw-agent.md",
            "mistral" => "auto-approve",
            _ => "yolo",
        };
        assert!(argv.contains(writer_marker), "{seat}: {argv}");
        assert_parent_untouched(&w);
    }
}

/// The daemon writes `.triumvirate/` into every repo's .gitignore; with that committed, seat-run still commits the
/// seat's work (and never `.triumvirate/`). Found live: `git add -A -- . ':!.triumvirate'` failed "paths are ignored"
/// and a fleet member's file was left uncommitted.
/// RED IF: an ignored `.triumvirate/` makes the commit step fail, or `.triumvirate/` content lands in the commit.
#[test]
fn an_ignored_triumvirate_dir_does_not_block_the_commit() {
    let w = World::new(None);
    fs::write(w.wt.join(".gitignore"), ".triumvirate/\n").unwrap();
    git(&w.wt, &["add", ".gitignore"]);
    git(&w.wt, &["commit", "-qm", "ignore the daemon dir"]);
    w.set_action("kimi", "echo hello > out.txt\n");
    let before = w.head();
    let out = w.run("kimi", "W1-PROMPT-ignored");
    let line = last_stderr_line(&out);
    assert_eq!(out.status.code(), Some(0), "{line}\n{out:?}");
    assert_ne!(w.head(), before, "no new commit");
    assert_eq!(
        git(&w.wt, &["show", "--name-only", "--format=", "HEAD"]),
        "out.txt"
    );
}

/// A mock that writes into the PARENT repo (allowed by the seatbelt through the temp-root seam) and into the worktree:
/// `writer_escaped_worktree:kimi: parent:escape.txt`, exit 1, nothing committed, nothing reverted.
/// RED IF: the parent-repo audit is missing, the escape is committed alongside the work, or the exit is 0.
#[test]
fn a_write_into_the_parent_repo_is_an_escape_and_commits_nothing() {
    let w = World::new(None);
    w.set_action(
        "kimi",
        "echo x > \"$W1_PARENT/escape.txt\"\necho ok > out.txt\n",
    );
    let before = w.head();
    let out = w.run("kimi", "p");
    let line = last_stderr_line(&out);
    assert_eq!(out.status.code(), Some(1), "{line}");
    assert_eq!(
        line,
        "seat-run: writer_escaped_worktree:kimi: parent:escape.txt"
    );
    assert_eq!(w.head(), before, "nothing may be committed");
    assert!(w.parent.join("escape.txt").exists(), "nothing is reverted");
}

/// A mock that changes nothing: exit 3, `seat_made_no_changes:kimi`, no commit.
/// RED IF: an empty commit is made, or the run reports success.
#[test]
fn no_changes_exits_3_and_commits_nothing() {
    let w = World::new(None);
    let before = w.head();
    let out = w.run("kimi", "p");
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    assert_eq!(
        last_stderr_line(&out),
        "seat-run: no_changes seat_made_no_changes:kimi"
    );
    assert_eq!(w.head(), before);
}

/// Git metadata under the bypass seam: rewriting the worktree's `.git` file, then (separately) planting a hook in the
/// common dir, each fails `writer_escaped_worktree` and commits nothing (the hook would otherwise run at commit).
/// RED IF: the `.git` entry or the common dir's hooks/ are not audited, or seat-run commits after either.
#[test]
fn git_metadata_writes_are_escapes() {
    let w = World::new(None);
    let before = w.head();
    let dot_git = fs::read_to_string(w.wt.join(".git")).unwrap();
    w.set_action("kimi", "echo >> .git\necho ok > out.txt\n");
    let out = w.run("kimi", "p");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        last_stderr_line(&out),
        "seat-run: writer_escaped_worktree:kimi: worktree:.git"
    );
    assert_eq!(w.head(), before);
    fs::write(w.wt.join(".git"), &dot_git).unwrap();

    w.set_action(
        "kimi",
        "printf '#!/bin/sh\\nexit 0\\n' > \"$W1_PARENT/.git/hooks/pre-commit\"\necho ok > out.txt\n",
    );
    let out = w.run("kimi", "p");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        last_stderr_line(&out),
        "seat-run: writer_escaped_worktree:kimi: git-common:hooks/pre-commit"
    );
    assert_eq!(w.head(), before);
}

/// SIGTERM to seat-run while the mock sleeps (after it wrote out.txt): exit 143, the mock's process is gone, no
/// commit.
/// RED IF: the signal leaves the vendor running, seat-run commits the partial work, or the exit is not 143.
#[test]
fn sigterm_kills_the_vendor_and_commits_nothing() {
    let w = World::new(None);
    w.set_action("kimi", "echo ok > out.txt\nsleep 30\n");
    let before = w.head();
    let child = w.command("kimi", "p", 60).spawn().expect("spawn");
    let pid_file = w.mock_dir("kimi").join("pid");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !pid_file.exists() || !w.wt.join("out.txt").exists() {
        assert!(Instant::now() < deadline, "the mock never started");
        std::thread::sleep(Duration::from_millis(50));
    }
    let mock_pid = fs::read_to_string(&pid_file).unwrap();
    assert!(
        Command::new("/bin/kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(143), "{out:?}");
    assert!(
        last_stderr_line(&out).starts_with("seat-run: seat_run_signalled:kimi: SIGTERM"),
        "{out:?}"
    );
    assert_pid_gone(&mock_pid);
    assert_eq!(w.head(), before);
}

/// `--timeout-secs 2` against a 30 s mock: exit 124 well before the mock would finish, mock gone, no commit.
/// RED IF: the timeout does not bound the vendor run, leaves the vendor alive, or commits.
#[test]
fn timeout_kills_the_vendor_and_commits_nothing() {
    let w = World::new(None);
    w.set_action("kimi", "echo ok > out.txt\nsleep 30\n");
    let before = w.head();
    let started = Instant::now();
    let out = w.command("kimi", "p", 2).output().unwrap();
    assert!(started.elapsed() < Duration::from_secs(20), "not bounded");
    assert_eq!(out.status.code(), Some(124), "{out:?}");
    assert!(
        last_stderr_line(&out).starts_with("seat-run: seat_timed_out:kimi:"),
        "{out:?}"
    );
    assert_pid_gone(&w.mock_file("kimi", "pid"));
    assert_eq!(w.head(), before);
}

/// A mock that prints a line, sleeps 2 s, prints another: seat-run's first line reaches its own stdout while the
/// mock is still running (well before seat-run's stdout closes), not buffered to the end.
/// RED IF: seat-run buffers the vendor's output until it exits (fleet's stall detection would see silence).
#[test]
fn vendor_output_streams_line_by_line() {
    let w = World::new(None);
    w.set_action("kimi", "echo W1-FIRST-LINE\nsleep 2\necho W1-SECOND-LINE\n");
    let mut child: Child = w.command("kimi", "p", 60).spawn().expect("spawn");
    let stdout = child.stdout.take().unwrap();
    let started = Instant::now();
    let mut first = None;
    let mut second = None;
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        if line == "W1-FIRST-LINE" {
            first = Some(started.elapsed());
        }
        if line == "W1-SECOND-LINE" {
            second = Some(started.elapsed());
        }
    }
    let (first, second) = (first.expect("first line"), second.expect("second line"));
    let _ = child.wait();
    assert!(
        second >= first + Duration::from_millis(1500),
        "first line at {first:?}, second at {second:?}: the first was held back"
    );
}

/// Under the real seatbelt (macOS), with the parent repo OUTSIDE every allowed root: the mock's write into the parent
/// fails (the mock records the failure), escape.txt is absent, and its in-worktree write is committed.
/// RED IF: the writer seatbelt allows anything outside the worktree, or blocks the worktree itself.
#[cfg(target_os = "macos")]
#[test]
fn real_seatbelt_blocks_the_parent_and_allows_the_worktree() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let w = World::new(Some(base));
    for root in [
        "/private/var/folders",
        "/private/tmp",
        "/var/folders",
        "/tmp",
    ] {
        assert!(
            !w.parent.starts_with(root),
            "{} is seat-writable",
            w.parent.display()
        );
    }
    w.set_action(
        "kimi",
        "{ echo x > \"$W1_PARENT/escape.txt\"; } 2> \"$D/write_err\"\necho \"write_exit=$?\" > \"$D/write_record\"\necho ok > out.txt\n",
    );
    let before = w.head();
    let out = w.run("kimi", "p");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(
        !w.parent.join("escape.txt").exists(),
        "the seatbelt let the write through"
    );
    let record = w.mock_file("kimi", "write_record");
    assert!(
        record.starts_with("write_exit=") && record.trim() != "write_exit=0",
        "{record:?}"
    );
    assert!(w.mock_file("kimi", "write_err").contains("not permitted"));
    assert_ne!(w.head(), before);
    assert_eq!(
        git(&w.wt, &["show", "--name-only", "--format=", "HEAD"]),
        "out.txt"
    );
}

/// The mock links `link.md` INSIDE the worktree to the parent's README.md and writes through it. Under the bypass
/// seam the write lands in the parent and the audit names it; either way nothing is committed.
/// RED IF: a write through an in-worktree symlink into the parent is committed or reported as success.
#[test]
fn a_write_through_a_symlink_into_the_parent_is_an_escape() {
    let w = World::new(None);
    w.set_action(
        "kimi",
        "ln -s \"$W1_PARENT/README.md\" link.md\necho pwned > link.md\n",
    );
    let before = w.head();
    let out = w.run("kimi", "p");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        last_stderr_line(&out),
        "seat-run: writer_escaped_worktree:kimi: parent:README.md"
    );
    assert_eq!(w.head(), before);
}

/// Under the real seatbelt (macOS), the same symlink write is denied (the seatbelt resolves the target), the parent
/// is unchanged, and the link the seat left behind, pointing outside the worktree, is refused rather than committed.
/// RED IF: the seatbelt follows the link out of the worktree, or seat-run commits a link into the parent.
#[cfg(target_os = "macos")]
#[test]
fn real_seatbelt_denies_a_symlink_write_and_the_link_is_not_committed() {
    let w = World::new(Some(Path::new(env!("CARGO_TARGET_TMPDIR"))));
    w.set_action(
        "kimi",
        "ln -s \"$W1_PARENT/README.md\" link.md\n{ echo pwned > link.md; } 2> \"$D/write_err\"\n",
    );
    let before = w.head();
    let out = w.run("kimi", "p");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert_eq!(
        last_stderr_line(&out),
        "seat-run: writer_escaped_worktree:kimi: symlink:link.md"
    );
    assert!(w.mock_file("kimi", "write_err").contains("not permitted"));
    assert_parent_untouched(&w);
    assert_eq!(w.head(), before);
}

/// Seven Primes P-1: seat-run's tv_seat_writer_run (and its containment event) reach a local capture server from the
/// real binary: committed (a file named with the sentinel), no_changes, escaped (a parent write), timeout and killed.
/// Each exit's event arrives with exactly its documented keys, the run tag, and an outcome matching the exit code; no
/// tv_seat_* event carries a sentinel (which is in the prompt, the commit message and a committed path).
/// RED IF: a seat-run exit is not delivered before the process exits, gains or loses a property, reports the wrong
/// outcome, surface or file count, or carries a sentinel.
#[test]
fn p1_writer_run_events_reach_a_local_capture_server() {
    use capture_server::{CaptureServer, SENTINELS, TEST_INGEST_VALUE, assert_keys};
    const TAG: &str = "p1-writer";
    let [upper, _] = SENTINELS;
    let capture = CaptureServer::start();
    let w = World::new(None);
    let run = |prompt: &str, timeout_secs: u64| {
        let mut cmd = w.command("kimi", prompt, timeout_secs);
        cmd.env("POSTHOG_HOST", &capture.url)
            .env("POSTHOG_API_KEY", TEST_INGEST_VALUE)
            .env("TRIUMVIRATE_TELEMETRY_RUN_TAG", TAG);
        cmd
    };
    let prompt = format!("write {upper}");
    let mut want: Vec<(&str, u64)> = Vec::new();

    w.set_action("kimi", &format!("echo a > {upper}.txt\necho b > b.txt\n"));
    assert_eq!(run(&prompt, 60).output().unwrap().status.code(), Some(0));
    want.push(("committed", 2));

    w.set_action("kimi", "");
    assert_eq!(run(&prompt, 60).output().unwrap().status.code(), Some(3));
    want.push(("no_changes", 0));

    w.set_action(
        "kimi",
        "echo x > \"$W1_PARENT/escape.txt\"\necho ok > out.txt\n",
    );
    let mut cmd = run(&prompt, 60);
    cmd.env("W1_PARENT", &w.parent);
    assert_eq!(cmd.output().unwrap().status.code(), Some(1));
    want.push(("escaped", 0));
    fs::remove_file(w.parent.join("escape.txt")).unwrap();
    fs::remove_file(w.wt.join("out.txt")).unwrap();

    w.set_action("kimi", "sleep 30\n");
    assert_eq!(run(&prompt, 1).output().unwrap().status.code(), Some(124));
    want.push(("timeout", 0));

    let _ = fs::remove_file(w.mock_dir("kimi").join("pid"));
    let child = run(&prompt, 60).spawn().expect("spawn");
    let pid_file = w.mock_dir("kimi").join("pid");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !pid_file.exists() {
        assert!(Instant::now() < deadline, "the mock never started");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        Command::new("/bin/kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(143));
    want.push(("killed", 0));

    let runs = capture.wait_for("tv_seat_writer_run", want.len(), |p| p["tv_run_tag"] == TAG);
    assert_eq!(runs.len(), want.len(), "{runs:?}");
    for (p, (outcome, files)) in runs.iter().zip(&want) {
        assert_keys("tv_seat_writer_run", p, Some(TAG));
        assert_eq!(
            (
                &p["tv_agent"],
                &p["tv_surface"],
                &p["tv_outcome"],
                &p["tv_files_changed"]
            ),
            (
                &serde_json::json!("kimi"),
                &serde_json::json!("fleet_spawn"),
                &serde_json::json!(outcome),
                &serde_json::json!(files)
            ),
            "{p}"
        );
        assert!(p["tv_duration_ms"].is_u64(), "{p}");
    }
    let escapes = capture.wait_for("tv_seat_containment", 1, |p| p["tv_run_tag"] == TAG);
    assert_keys("tv_seat_containment", &escapes[0], Some(TAG));
    assert_eq!(
        (
            &escapes[0]["tv_containment_code"],
            &escapes[0]["tv_paths_count"]
        ),
        (
            &serde_json::json!("writer_escaped_worktree"),
            &serde_json::json!(1)
        )
    );
    capture.assert_clean();
}
