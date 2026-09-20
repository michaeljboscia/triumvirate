//! What model actually served a codex turn. D-021.
//!
//! Codex is the heaviest seat on this board by input tokens and it was the last one that could not
//! say which model answered. Two earlier attempts to source it were wrong, and both were wrong in
//! the same way, by reading code instead of measuring:
//!
//! 1. "`CodexAppServerParser` captures it off the JSON-RPC result." It does, and it never runs:
//!    `codex_protocol()` defaults to `exec`. Worse, on codex-cli 0.154.0 `codex app-server` is a
//!    tooling namespace (`daemon`, `proxy`, `generate-ts`), not the JSON-RPC server that parser
//!    targets, so its model capture is dead code against the installed CLI. Its only test asserts
//!    a HAND-WRITTEN payload, which is why it read as a working source.
//! 2. "The exec stream has no model, therefore codex cannot report it." The first half is true: a
//!    real `codex exec --json` capture is five events and none names a model. The second half does
//!    not follow, and it was contradicted by looking one directory further.
//!
//! Codex writes a rollout file per thread under `$CODEX_HOME/sessions/YYYY/MM/DD/`, and it carries
//! a `turn_context` record per turn whose `model` is the model that served it:
//!
//! ```json
//! {"type":"turn_context","payload":{"turn_id":"...","model":"gpt-5.6-sol","cwd":"..."}}
//! ```
//!
//! Verified on a DAEMON-driven call, not just a hand-run one: thread
//! `01a0bffd-a827-7d83-bce6-b21ca723db5e` from 2026-09-20 14:04 resolves to `gpt-5.6-sol`.
//!
//! This is a FACT about what ran, recorded by codex after the fact. It is deliberately not
//! `--model`, `-c model=` or `~/.codex/config.toml`, which are all what we ASKED for. Charting
//! intent as fact is D-020's defect, and on the heaviest seat a guessed model would look
//! authoritative, which is worse than a blank.

use std::path::PathBuf;

/// Where codex keeps its state. `CODEX_HOME` wins, else `~/.codex`.
fn codex_home() -> Option<PathBuf> {
    if let Ok(h) = std::env::var("CODEX_HOME")
        && !h.trim().is_empty()
    {
        return Some(PathBuf::from(h));
    }
    std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".codex"))
}

/// Directories are `sessions/YYYY/MM/DD`. Walk newest first so a long-lived install does not pay
/// for its whole history to find a thread that started minutes ago.
fn day_dirs(sessions: &PathBuf) -> Vec<PathBuf> {
    fn sorted_children(dir: &PathBuf) -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        // `file_type()` comes from the directory entry and does NOT follow symlinks, unlike
        // `Path::is_dir()`. A symlinked directory here would let the walk leave codex's own tree.
        let mut out: Vec<PathBuf> = rd
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.path())
            .collect();
        out.sort();
        out.reverse();
        out
    }
    let mut days = Vec::new();
    for year in sorted_children(sessions) {
        for month in sorted_children(&year) {
            days.extend(sorted_children(&month));
        }
    }
    days
}

/// The rollout file for one thread, if it exists. Named `rollout-<timestamp>-<thread_id>.jsonl`,
/// and we know only the id, so match on the suffix.
fn rollout_for(session_id: &str) -> Option<PathBuf> {
    // A path separator or traversal in the id would let a caller-supplied value reach the
    // filesystem. Thread ids are uuids; anything else is refused rather than sanitized, because
    // a "cleaned" path is still a path we were never meant to read.
    if session_id.is_empty()
        || !session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    let sessions = codex_home()?.join("sessions");
    let suffix = format!("-{session_id}.jsonl");
    for day in day_dirs(&sessions) {
        let Ok(rd) = std::fs::read_dir(&day) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // Regular files only, and again via the entry so a symlink is not followed. Found by
            // Codex in review: `read_to_string` on a FIFO here would BLOCK FOREVER, on the
            // telemetry path of a call that has already succeeded, which is a worse outcome than
            // the missing model this whole module exists to fix.
            if name.starts_with("rollout-")
                && name.ends_with(&suffix)
                && entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            {
                return Some(path);
            }
        }
    }
    None
}

/// The model that served the most recent turn of this codex thread, or `None`.
///
/// The LAST `turn_context` wins. The daemon reuses codex workers, so one thread accumulates a
/// turn_context per turn, and a session can change model partway through. The question
/// `$ai_model` asks is "which model answered THIS call", so the newest record is the right one.
///
/// Every failure returns `None` and the row charts `unknown`, which is honest and legible. This
/// runs on the telemetry path, so it must never fail a call that already succeeded.
pub fn model_for_session(session_id: &str) -> Option<String> {
    /// A rollout is a transcript, so it grows with the session and has no natural bound. This is
    /// the telemetry path of an ALREADY SUCCESSFUL call, so an unbounded read is not acceptable
    /// even on a file codex wrote itself. 32 MiB is far above any real rollout and far below
    /// anything that would hurt. Reading the first N bytes is safe for this job: `turn_context`
    /// records appear early in each turn, and a truncated final line simply fails to parse.
    const MAX_ROLLOUT_BYTES: u64 = 32 * 1024 * 1024;

    let path = rollout_for(session_id)?;
    let file = std::fs::File::open(&path).ok()?;
    // Re-check through the OPEN handle, not the path, so the answer cannot change between the
    // check and the read.
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut raw = String::new();
    {
        use std::io::Read as _;
        file.take(MAX_ROLLOUT_BYTES).read_to_string(&mut raw).ok()?;
    }
    let mut newest: Option<String> = None;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || !line.contains("turn_context") {
            continue;
        }
        let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if json.get("type").and_then(|v| v.as_str()) != Some("turn_context") {
            continue;
        }
        if let Some(model) = json
            .get("payload")
            .and_then(|p| p.get("model"))
            .and_then(|m| m.as_str())
            .filter(|m| !m.trim().is_empty())
        {
            newest = Some(model.to_string());
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One real rollout line, copied verbatim from
    /// `~/.codex/sessions/2026/09/20/rollout-2026-09-20T14-04-27-01a0bffd-...jsonl`, the thread the
    /// DAEMON created at 14:04 on 2026-09-20. Trimmed to the fields this module reads.
    ///
    /// Verbatim on purpose. The previous attempt at codex model attribution was validated against
    /// a hand-written `{"result":{"model":"codex-app-server"}}` and passed for a protocol the
    /// installed CLI no longer speaks. A test that invents its own input proves only that the code
    /// agrees with the test author.
    const REAL_TURN_CONTEXT: &str = r#"{"timestamp":"2026-09-20T18:04:28.1Z","type":"turn_context","payload":{"turn_id":"01a0bffd-a911-71e0-b29a-fe2132cb5103","cwd":"/repo","model":"gpt-5.6-sol","approval_policy":"never"}}"#;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tv-rollout-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// Write a rollout where codex would, and read the model back out of it.
    fn write_rollout(home: &std::path::Path, day: &str, session_id: &str, lines: &[&str]) {
        let dir = home.join("sessions").join("2026").join("09").join(day);
        std::fs::create_dir_all(&dir).expect("session dir");
        let f = dir.join(format!("rollout-2026-09-{day}T14-04-27-{session_id}.jsonl"));
        std::fs::write(f, lines.join("\n")).expect("rollout");
    }

    /// The whole point: a real `turn_context` yields the model that served the turn.
    ///
    /// RED IF: the record type, the payload path, or the field name stops matching what codex
    /// actually writes.
    #[test]
    fn a_real_turn_context_yields_the_model_that_served_it() {
        let home = scratch("real");
        let id = "01a0bffd-a827-7d83-bce6-b21ca723db5e";
        write_rollout(&home, "20", id, &[REAL_TURN_CONTEXT]);
        temp_env_codex_home(&home, || {
            assert_eq!(model_for_session(id).as_deref(), Some("gpt-5.6-sol"));
        });
    }

    /// The daemon reuses codex workers, so one thread holds a turn_context PER TURN and the model
    /// can change partway through. `$ai_model` asks which model answered THIS call, so the newest
    /// record wins.
    ///
    /// RED IF: the scan returns the first match instead of the last.
    #[test]
    fn the_newest_turn_wins_when_a_thread_has_several() {
        let home = scratch("newest");
        let id = "01a0bffd-1111-2222-3333-444455556666";
        let older = REAL_TURN_CONTEXT.replace("gpt-5.6-sol", "gpt-5.6-old");
        write_rollout(&home, "20", id, &[&older, REAL_TURN_CONTEXT]);
        temp_env_codex_home(&home, || {
            assert_eq!(model_for_session(id).as_deref(), Some("gpt-5.6-sol"));
        });
    }

    /// Anything missing or malformed is `None`, never a plausible substitute.
    ///
    /// This runs on the telemetry path of a call that has ALREADY succeeded, so it must degrade to
    /// "we do not know" rather than fail or fabricate. `unknown` is a legible slice on a dashboard;
    /// a guessed model on the heaviest seat is not.
    ///
    /// RED IF: any default or fallback model is introduced, or a failure starts panicking.
    #[test]
    fn anything_missing_or_malformed_is_none_not_a_guess() {
        let home = scratch("missing");
        let id = "01a0bffd-dead-beef-dead-beefdeadbeef";
        temp_env_codex_home(&home, || {
            // No sessions directory at all.
            assert_eq!(model_for_session(id), None);
        });

        write_rollout(
            &home,
            "20",
            id,
            &[
                "not json at all",
                r#"{"type":"session_meta","payload":{"model_provider":"openai"}}"#,
                r#"{"type":"turn_context","payload":{"turn_id":"t"}}"#,
                r#"{"type":"turn_context","payload":{"model":"   "}}"#,
                r#"{"type":"token_usage_record","payload":{"model":"not-a-turn-context"}}"#,
            ],
        );
        temp_env_codex_home(&home, || {
            assert_eq!(
                model_for_session(id),
                None,
                "no turn_context names a model, so the honest answer is that we do not know"
            );
        });
    }

    /// A session id is used to build a PATH, so it is validated rather than cleaned.
    ///
    /// D-024 is the same lesson on the telemetry side: a caller-supplied value that reaches a
    /// bounded surface has to be refused when it does not belong, not massaged until it fits. A
    /// sanitized path is still a path we were never meant to read.
    ///
    /// RED IF: traversal or separators start being stripped instead of rejected.
    #[test]
    fn a_session_id_that_is_not_an_id_is_refused() {
        let home = scratch("hostile");

        // The discriminating case. A first draft of this test asserted only that hostile ids
        // return None, and it stayed GREEN when the validation was deleted, because those ids
        // match no file either way. It proved nothing: it checked the holder, not the guard. So
        // plant a rollout that a hostile id WOULD resolve to, and the assertion then measures the
        // guard rather than the absence of a file.
        // No separator: this must be a value that really does land in the day directory, so the
        // lookup genuinely WOULD find it if the guard were gone.
        let planted = "id with spaces";
        write_rollout(&home, "20", planted, &[REAL_TURN_CONTEXT]);

        temp_env_codex_home(&home, || {
            assert_eq!(
                model_for_session(planted),
                None,
                "a planted rollout must NOT be reachable through a non-id: without the guard this \
                 returns gpt-5.6-sol, which is how we know the guard is doing the work"
            );
            for hostile in ["../../../../etc/passwd", "a/b", "..", "", "id\nwith-newline"] {
                assert_eq!(model_for_session(hostile), None, "{hostile:?} must be refused");
            }
        });
    }

    /// Only a REGULAR FILE is read, and the check does not follow symlinks.
    ///
    /// Found by Codex in review. `Path::is_dir()` and `read_to_string()` both follow symlinks and
    /// neither requires a regular file, so a matching symlink, or a FIFO, could send this reader
    /// somewhere it was never meant to go. On a FIFO `read_to_string` BLOCKS FOREVER, and it would
    /// do so on the telemetry path of a call that has ALREADY SUCCEEDED, which is a worse outcome
    /// than the missing model this module exists to supply.
    ///
    /// The symlink case is asserted rather than the FIFO case on purpose: removing the guard makes
    /// this test FAIL, whereas a FIFO test would HANG, and a test that hangs on regression is not
    /// a usable signal.
    ///
    /// RED IF: the `file_type().is_file()` check is dropped, or the walk starts following links.
    #[test]
    fn a_symlinked_rollout_is_not_followed() {
        let home = scratch("symlink");
        let id = "01a0c086-e405-7551-9139-5e33ef5018cf";

        // A real rollout, with valid content, parked OUTSIDE the sessions tree.
        let elsewhere = home.join("elsewhere.jsonl");
        std::fs::write(&elsewhere, REAL_TURN_CONTEXT).expect("bait");

        // ...and a correctly-named symlink to it, where the walk will find it.
        let dir = home.join("sessions").join("2026").join("09").join("20");
        std::fs::create_dir_all(&dir).expect("session dir");
        let link = dir.join(format!("rollout-2026-09-20T14-04-27-{id}.jsonl"));
        std::os::unix::fs::symlink(&elsewhere, &link).expect("symlink");

        temp_env_codex_home(&home, || {
            assert_eq!(
                model_for_session(id),
                None,
                "a symlink is not a rollout: without the file_type guard this returns \
                 gpt-5.6-sol, which is how we know the guard is doing the work"
            );
        });
    }

    /// `CODEX_HOME` is honoured, because an operator who moved codex's state has not moved codex.
    #[test]
    fn codex_home_overrides_the_default_location() {
        let home = scratch("codexhome");
        let id = "01a0bffd-aaaa-bbbb-cccc-ddddeeeeffff";
        write_rollout(&home, "19", id, &[REAL_TURN_CONTEXT]);
        temp_env_codex_home(&home, || {
            assert_eq!(model_for_session(id).as_deref(), Some("gpt-5.6-sol"));
        });
    }

    /// Serialise the env mutation: `CODEX_HOME` is process-global and these tests all set it.
    ///
    /// The lock lives next to the state it guards, not inside one test module, which is the rule
    /// this repo learned the hard way (see `grok::GROK_ENV_LOCK`). A per-module lock serialises a
    /// module against itself and against nothing else.
    static CODEX_HOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn temp_env_codex_home(home: &std::path::Path, f: impl FnOnce()) {
        let _guard = CODEX_HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior = std::env::var("CODEX_HOME").ok();
        // SAFETY: serialised by CODEX_HOME_LOCK; restored before the guard drops.
        unsafe { std::env::set_var("CODEX_HOME", home) };
        f();
        unsafe {
            match prior {
                Some(v) => std::env::set_var("CODEX_HOME", v),
                None => std::env::remove_var("CODEX_HOME"),
            }
        }
    }
}
