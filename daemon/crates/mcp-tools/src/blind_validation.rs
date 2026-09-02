//! Blind validation: a DIFFERENT agent writes the tests, and it never sees the code.
//!
//! WHY THIS EXISTS. Every layer built before this proves a reviewer LOOKED. The sight gate
//! proves a tool call named the artifact; the nonce proves the read reached the end of the file;
//! the partial-read rule proves it was the whole file. None of them prove the reviewer JUDGED
//! anything. All three peers said so independently and the limit was documented as unbuilt,
//! because on prose there is nothing mechanical left to check: an agent can read every byte and
//! write its verdict from nothing.
//!
//! On CODE there is. A different agent writes tests from the contract, the tests run, and the
//! process exits zero or it does not. That is not an opinion about the work, it is the work
//! being exercised. It is the first check in this system whose verdict nobody has to trust.
//!
//! THE BLINDNESS IS MECHANICAL, NOT REQUESTED.
//!
//! If the validator can read the implementation it will write tests that mirror it, and a
//! tautology passes. Asking it not to look is worth nothing: this repo has already watched an
//! agent describe its own toolless output as "rigorous sourcing".
//!
//! So the validator runs in its OWN directory, which contains the contract and nothing else. The
//! implementation is not on its disk. It cannot read what is not there.
//!
//! ```text
//!   impl worktree/                 validator dir/
//!     src/thing.rs      <-----       (absent)
//!     BRIEFING.md       ----->       BRIEFING.md
//!                                    tests/blind.rs   <- written here
//!
//!   then: copy tests/blind.rs into the impl worktree and run the suite there
//! ```
//!
//! Detection is kept as a SECOND line, not as the defence. `reads_outside_allowed_root` scans
//! the validator's tool calls for anything reaching outside its own directory. Containment is
//! what actually holds; detection catches a containment that was not applied, which is exactly
//! the arrangement the sight gate already uses and for the same reason: a write performed inside
//! a shell command is indistinguishable from a read.
//!
//! WHY THE CONTRACT CARRIES THE API SURFACE.
//!
//! The brief states the names and signatures the worker will produce, and the validator reads
//! that same brief. Without it, in a compiled language, the validator has to GUESS the interface
//! and a name mismatch is a compile error rather than a test failure. A red result would then
//! mostly measure whether two agents guessed the same identifier, which is a false-rejection
//! machine and the same shape as blaming an agent for a blocked instrument.
//!
//! With it, three things hold at once: the tests compile, a tautology is still impossible
//! (a signature says nothing about whether the logic is right), and "built the wrong thing"
//! still fails, because the CONTRACT is the authority rather than whatever happens to exist in
//! the worktree.
//!
//! The cost is real and belongs to whoever writes the brief: a contract too vague to test is
//! rejected before dispatch. That is a feature. A brief too vague to test was too vague to
//! dispatch.

use std::collections::BTreeMap;

/// One test's outcome, as reported by the harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestOutcome {
    Passed,
    Failed,
    /// Present in the run but not executed. Never counted as a pass.
    Ignored,
}

/// A whole test run, keyed by test name.
///
/// A `BTreeMap` rather than a `HashMap` so the report is deterministic. A validation report whose
/// line order changes between runs is one nobody diffs.
pub type TestRun = BTreeMap<String, TestOutcome>;

/// Parse `cargo test` output into per-test outcomes.
///
/// Deliberately parses the PER-TEST lines rather than the summary. The summary says how many
/// failed; it does not say WHICH, and "which" is the entire point of a baseline diff.
///
/// Unknown or malformed lines are skipped rather than guessed at. A line this cannot read
/// contributes nothing, which fails toward "no evidence" rather than toward "passed".
pub fn parse_cargo_test_output(output: &str) -> TestRun {
    let mut run = TestRun::new();
    for line in output.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("test ") else {
            continue;
        };
        // `test some::name ... ok` / `... FAILED` / `... ignored, reason`
        let Some((name, tail)) = rest.split_once(" ... ") else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name.contains(' ') {
            continue;
        }
        let outcome = if tail.starts_with("ok") {
            TestOutcome::Passed
        } else if tail.starts_with("FAILED") {
            TestOutcome::Failed
        } else if tail.starts_with("ignored") {
            TestOutcome::Ignored
        } else {
            continue;
        };
        run.insert(name.to_string(), outcome);
    }
    run
}

/// Tests that PASSED before the worker ran and do not pass now.
///
/// The baseline is why this is trustworthy. A repo with pre-existing failures would otherwise
/// blame the worker for every one of them, and a signal that cries wolf gets turned off. Only a
/// test that was green and is not green now is the worker's doing.
///
/// A test that DISAPPEARED counts as newly failing. That is deliberate: deleting a failing test
/// is the cheapest way to make a suite green, and it must not be a silent pass.
pub fn newly_failing(baseline: &TestRun, after: &TestRun) -> Vec<String> {
    baseline
        .iter()
        .filter(|(_, outcome)| **outcome == TestOutcome::Passed)
        .filter(|(name, _)| after.get(*name) != Some(&TestOutcome::Passed))
        .map(|(name, _)| name.clone())
        .collect()
}

/// Pick the agent that will write the tests. It must NOT be the one that wrote the code.
///
/// The same rule the peer-review engine applies to reviewers, for the same reason: an agent
/// validating its own work re-derives its own misunderstanding. Here it is worse than in review,
/// because the validator is writing the very tests that decide the verdict.
///
/// Deterministic rather than round-robin: a validation that picks a different peer on a rerun is
/// not reproducible, and the first question anyone asks a red result is "does it still fail".
pub fn pick_validator(worker_agent: &str, roster: &[String]) -> Result<String, String> {
    let worker = worker_agent.trim().to_ascii_lowercase();
    let pick = roster
        .iter()
        .map(|a| a.trim().to_ascii_lowercase())
        .find(|a| !a.is_empty() && *a != worker);
    pick.ok_or_else(|| {
        format!(
            "no validator available: the roster {roster:?} contains nobody but the worker \
             ({worker_agent}). An agent cannot blind-validate its own code, because it would \
             write tests from the same misunderstanding that produced it."
        )
    })
}

/// Paths the validator touched that lie OUTSIDE the directory it was given.
///
/// The second line of defence, not the first. Containment is what actually keeps the
/// implementation off the validator's disk; this catches the case where containment was not
/// applied, which is the failure the sight gate work already hit twice (a missing
/// `--sandbox read-only`, and a missing `--dangerously-skip-permissions`).
///
/// Conservative on purpose. It reports what it can see and does not pretend to see everything:
/// a read performed inside a shell command is not distinguishable from any other shell command,
/// exactly as `enforce_reviewer_sight` says of writes. A clean result here is not proof of
/// blindness. An unclean result IS proof of sightedness.
pub fn reads_outside_allowed_root(
    tool_calls: &[agent_adapter::ToolCallRecord],
    allowed_root: &str,
) -> Vec<String> {
    let root = allowed_root.trim_end_matches('/');
    let mut found = Vec::new();
    for call in tool_calls {
        let Some(args) = call.args_json.as_deref() else {
            continue;
        };
        for path in absolute_paths_in(args) {
            // Inside the allowed root at a DIRECTORY BOUNDARY. `/tmp/v` must not match
            // `/tmp/validator-impl`, which is the same off-by-one the sight gate already fixed
            // once with `strip_prefix`.
            let inside = path == root
                || path.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'));
            if !inside {
                found.push(path);
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Absolute-looking paths appearing anywhere in a tool's recorded arguments.
///
/// Works on the raw JSON text rather than on known field names, because every backend spells the
/// path field differently (`file_path`, `target_file`, `path`, `command`) and a check that only
/// knows some of them is a check with holes in it.
fn absolute_paths_in(args_json: &str) -> Vec<String> {
    // A `/` only starts an ABSOLUTE path when it opens a token. Without that check the relative
    // path `src/thing.rs` yields `/thing.rs`, which is not a path anyone wrote and which then
    // gets reported as an escape from the validator's directory. Caught by `bv_09`, which was
    // asserting the opposite property and failed for this reason rather than for its own.
    fn opens_a_token(prev: Option<u8>) -> bool {
        match prev {
            None => true,
            Some(c) => matches!(c, b'"' | b' ' | b'\'' | b'=' | b'(' | b'[' | b',' | b':' | b'\t'),
        }
    }

    let mut out = Vec::new();
    let bytes = args_json.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && opens_a_token(i.checked_sub(1).map(|p| bytes[p])) {
            let start = i;
            while i < bytes.len() && !matches!(bytes[i], b'"' | b' ' | b',' | b'\\' | b'\'') {
                i += 1;
            }
            let raw = &args_json[start..i];
            let cleaned = raw.trim_end_matches(|c| matches!(c, '.' | ':' | ';' | ')' | ']'));
            if cleaned.len() > 1 {
                out.push(cleaned.to_string());
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The brief the validator is given. The contract, and an instruction to write tests from it.
///
/// The implementation is not mentioned and not reachable. The prompt says so explicitly, because
/// an agent that believes it is missing context will go looking for it, and a validator hunting
/// for source it cannot find spends its budget on the hunt. Grok did exactly that on 2026-09-01
/// when a brief named a path that did not exist.
pub fn build_validator_prompt(contract: &str, test_file_path: &str, test_command: &str) -> String {
    format!(
        "You are writing tests for code you cannot see, and that is deliberate.\n\n\
         The implementation is NOT on this machine. Do not look for it. There is no source tree \
         here to read, and time spent searching is time not spent writing tests.\n\n\
         Write tests from the CONTRACT below and from nothing else. The contract states the \
         exact names and signatures the implementation provides, so your tests will compile \
         against it. Test the BEHAVIOUR the contract promises, including the edges it names and \
         the failure cases it names.\n\n\
         Write them to exactly this path, and write nothing else anywhere:\n\n  {test_file_path}\n\n\
         They will be run with:\n\n  {test_command}\n\n\
         A test that passes against a wrong implementation is worthless, so do not write tests \
         that merely assert the code runs. Assert what the contract says the answers are.\n\n\
         ----- BEGIN CONTRACT -----\n{contract}\n----- END CONTRACT -----"
    )
}

/// What a blind validation concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlindReport {
    pub validator_agent: String,
    /// Did the validator's own tests pass against the implementation?
    pub blind_tests_passed: bool,
    /// Tests that were green before the worker ran and are not green now.
    pub newly_failing: Vec<String>,
    /// Paths the validator touched outside its own directory. Non-empty means the run was not
    /// blind and the result cannot be trusted in either direction.
    pub blindness_violations: Vec<String>,
}

impl BlindReport {
    /// Did the worktree do what it was dispatched to do?
    ///
    /// Fails closed on a blindness violation. A sighted validator's PASS is worthless, because
    /// it may have written tests that mirror the implementation, and its FAIL is unattributable.
    /// Neither answer is usable, so the run is not an answer.
    pub fn accepted(&self) -> bool {
        self.blindness_violations.is_empty()
            && self.blind_tests_passed
            && self.newly_failing.is_empty()
    }

    /// Why it was not accepted, in the caller's words rather than a status code.
    pub fn why_rejected(&self) -> Option<String> {
        if !self.blindness_violations.is_empty() {
            return Some(format!(
                "the validation was NOT BLIND: {} read {} outside its own directory, so its \
                 tests may mirror the implementation rather than the contract. A sighted \
                 validator's pass proves nothing and its failure cannot be attributed. Fix the \
                 containment and re-run; do not read the verdict.",
                self.validator_agent,
                self.blindness_violations.join(", ")
            ));
        }
        if !self.blind_tests_passed {
            return Some(format!(
                "{} wrote tests from the contract and the implementation did not pass them. The \
                 worktree did not do what it was dispatched to do.",
                self.validator_agent
            ));
        }
        if !self.newly_failing.is_empty() {
            return Some(format!(
                "the contract was met, but {} test(s) that passed before this dispatch do not \
                 pass now: {}. These were green at baseline, so they are this worker's doing.",
                self.newly_failing.len(),
                self.newly_failing.join(", ")
            ));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_adapter::{ToolCallRecord, ToolKind};

    fn read_call(args: &str) -> ToolCallRecord {
        ToolCallRecord {
            id: Some("t".into()),
            tool: "Read".into(),
            kind: ToolKind::ReadFile,
            success: Some(true),
            duration_ms: None,
            args_json: Some(args.to_string()),
        }
    }

    /// The parser reads PER-TEST lines, not the summary. The summary says how many failed and
    /// not which, and "which" is the whole point of a baseline diff.
    /// RED IF: the parser starts trusting the summary line.
    #[test]
    fn bv_01_per_test_outcomes_are_parsed() {
        let out = "\
running 3 tests
test util::a ... ok
test util::b ... FAILED
test util::c ... ignored, needs a live key

test result: FAILED. 1 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out
";
        let run = parse_cargo_test_output(out);
        assert_eq!(run.get("util::a"), Some(&TestOutcome::Passed));
        assert_eq!(run.get("util::b"), Some(&TestOutcome::Failed));
        assert_eq!(run.get("util::c"), Some(&TestOutcome::Ignored));
        assert_eq!(run.len(), 3, "the summary line must not become a fourth entry");
    }

    /// An ignored test is NOT a pass. Treating it as one would let a worker silence a failure
    /// with `#[ignore]` and come back green.
    /// RED IF: Ignored is folded into Passed.
    #[test]
    fn bv_02_an_ignored_test_is_not_a_pass() {
        let baseline = parse_cargo_test_output("test a ... ok\n");
        let after = parse_cargo_test_output("test a ... ignored, flaky\n");
        assert_eq!(
            newly_failing(&baseline, &after),
            vec!["a".to_string()],
            "ignoring a test that used to pass must be reported, not absorbed"
        );
    }

    /// A test that DISAPPEARED counts as newly failing. Deleting a failing test is the cheapest
    /// way to make a suite green and it must not be a silent pass.
    /// RED IF: a missing test stops counting.
    #[test]
    fn bv_03_a_deleted_test_counts_as_broken() {
        let baseline = parse_cargo_test_output("test a ... ok\ntest b ... ok\n");
        let after = parse_cargo_test_output("test a ... ok\n");
        assert_eq!(newly_failing(&baseline, &after), vec!["b".to_string()]);
    }

    /// A test that was ALREADY failing before the dispatch is not the worker's doing. Without
    /// this the signal cries wolf on any repo with a pre-existing failure, and a signal that
    /// cries wolf gets turned off.
    /// RED IF: the baseline stops being consulted.
    #[test]
    fn bv_04_a_pre_existing_failure_is_not_blamed_on_the_worker() {
        let baseline = parse_cargo_test_output("test a ... ok\ntest b ... FAILED\n");
        let after = parse_cargo_test_output("test a ... ok\ntest b ... FAILED\n");
        assert!(
            newly_failing(&baseline, &after).is_empty(),
            "b was already red; the worker did not break it"
        );
    }

    /// The validator must not be the worker. An agent validating its own code writes tests from
    /// the same misunderstanding that produced it.
    /// RED IF: the worker can be selected as its own validator.
    #[test]
    fn bv_05_an_agent_cannot_validate_its_own_code() {
        let roster = vec!["codex".to_string(), "claude".to_string(), "grok".to_string()];
        assert_eq!(pick_validator("codex", &roster).unwrap(), "claude");
        assert_eq!(pick_validator("claude", &roster).unwrap(), "codex");
        assert_eq!(pick_validator("CODEX", &roster).unwrap(), "claude", "case must not decide");

        let err = pick_validator("codex", &["codex".to_string()]).unwrap_err();
        assert!(err.contains("no validator available"), "got: {err}");
    }

    /// Deterministic, so a rerun picks the same validator. The first question anyone asks a red
    /// result is "does it still fail", and a rotating validator cannot answer it.
    /// RED IF: selection becomes round-robin or random.
    #[test]
    fn bv_06_validator_selection_is_reproducible() {
        let roster = vec!["codex".to_string(), "claude".to_string(), "grok".to_string()];
        let first = pick_validator("grok", &roster).unwrap();
        for _ in 0..5 {
            assert_eq!(pick_validator("grok", &roster).unwrap(), first);
        }
    }

    /// The blindness check, on the shapes the backends actually emit.
    /// RED IF: a path outside the validator's directory stops being reported.
    #[test]
    fn bv_07_reads_outside_the_validator_dir_are_reported() {
        let calls = vec![
            read_call(r#"{"file_path":"/tmp/validator/BRIEFING.md"}"#),
            read_call(r#"{"file_path":"/tmp/impl/src/thing.rs"}"#),
            read_call(r#"{"command":"cat /tmp/impl/src/other.rs"}"#),
            read_call(r#"{"target_file":"/tmp/validator/tests/blind.rs"}"#),
        ];
        let found = reads_outside_allowed_root(&calls, "/tmp/validator");
        assert_eq!(
            found,
            vec!["/tmp/impl/src/other.rs".to_string(), "/tmp/impl/src/thing.rs".to_string()],
            "both the structured read and the one buried in a shell command must be reported"
        );
    }

    /// The directory-boundary rule. `/tmp/v` must not swallow `/tmp/validator-impl`, which is
    /// the same off-by-one the sight gate already fixed once with a bare `strip_prefix`.
    /// RED IF: the boundary check is dropped.
    #[test]
    fn bv_08_a_prefix_is_not_a_parent_directory() {
        let calls = vec![read_call(r#"{"file_path":"/tmp/v-impl/src/thing.rs"}"#)];
        let found = reads_outside_allowed_root(&calls, "/tmp/v");
        assert_eq!(
            found,
            vec!["/tmp/v-impl/src/thing.rs".to_string()],
            "/tmp/v-impl is NOT inside /tmp/v"
        );
    }

    /// A clean scan is not proof of blindness and the code says so. This pins the honest reading:
    /// the check reports what it can see. Containment is the defence.
    /// RED IF: the doc claim and the behaviour drift apart.
    #[test]
    fn bv_09_a_clean_scan_is_only_the_absence_of_evidence() {
        // A shell command with no absolute path in it: unreadable to this check, by construction.
        let calls = vec![read_call(r#"{"command":"cd .. && cat src/thing.rs"}"#)];
        assert!(
            reads_outside_allowed_root(&calls, "/tmp/validator").is_empty(),
            "this check cannot see a relative traversal, which is exactly why containment and \
             not detection is the defence. If this ever fires, the check got stronger and the \
             module doc must be updated to stop understating it."
        );
    }

    /// A blindness violation fails CLOSED, and does so before the test result is consulted.
    /// A sighted validator's pass proves nothing and its failure cannot be attributed.
    /// RED IF: a violated run can be accepted, or is reported as a mere test failure.
    #[test]
    fn bv_10_a_sighted_validation_is_not_an_answer() {
        let report = BlindReport {
            validator_agent: "claude".to_string(),
            blind_tests_passed: true,
            newly_failing: vec![],
            blindness_violations: vec!["/tmp/impl/src/thing.rs".to_string()],
        };
        assert!(!report.accepted(), "a sighted PASS must not be accepted");
        let why = report.why_rejected().expect("must explain");
        assert!(why.contains("NOT BLIND"), "got: {why}");
        assert!(
            !why.contains("did not pass them"),
            "it must not be reported as a test failure: the tests passed, the RUN is invalid"
        );
    }

    /// The three rejection reasons are distinct and ordered, because they need different fixes:
    /// a broken containment, a worktree that did not do the job, and collateral damage.
    /// RED IF: they collapse into one message.
    #[test]
    fn bv_11_the_three_failures_report_differently() {
        let base = BlindReport {
            validator_agent: "grok".to_string(),
            blind_tests_passed: true,
            newly_failing: vec![],
            blindness_violations: vec![],
        };
        assert!(base.accepted());
        assert_eq!(base.why_rejected(), None);

        let failed_contract = BlindReport { blind_tests_passed: false, ..base.clone() };
        assert!(failed_contract.why_rejected().unwrap().contains("did not do what it was dispatched"));

        let broke_things =
            BlindReport { newly_failing: vec!["other::test".into()], ..base.clone() };
        let why = broke_things.why_rejected().unwrap();
        assert!(why.contains("passed before this dispatch"), "got: {why}");
        assert!(why.contains("other::test"), "the caller must be told WHICH; got: {why}");
    }

    /// The prompt must tell the validator the implementation is absent. An agent that believes
    /// it is missing context goes looking for it, and a validator hunting for source it cannot
    /// find spends its budget on the hunt. That happened for real on 2026-09-01 when a brief
    /// named a path that did not exist.
    /// RED IF: the prompt stops saying the code is not there, or stops naming the output path.
    #[test]
    fn bv_12_the_prompt_says_the_code_is_absent_and_names_the_output() {
        let p = build_validator_prompt("pub fn f() -> u8", "/tmp/v/tests/blind.rs", "cargo test");
        assert!(p.contains("NOT on this machine"));
        assert!(p.contains("Do not look for it"));
        assert!(p.contains("/tmp/v/tests/blind.rs"), "the output path must be explicit");
        assert!(p.contains("cargo test"), "the run command must be stated");
        assert!(p.contains("pub fn f() -> u8"), "the contract must be carried");
        assert!(
            p.contains("do not write tests that merely assert the code runs"),
            "a smoke test that passes against a wrong implementation is worthless"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Orchestration.
//
// Kept behind injected callbacks, the same shape ABE uses, so the whole flow is testable without
// spawning a model or touching a real repo. The guarantees above are pure functions; this is the
// plumbing that arranges for them to be true.
// ---------------------------------------------------------------------------------------------

use std::path::{Path, PathBuf};

/// Everything a blind validation needs, with the side effects injected.
pub struct BlindValidationCallbacks<'a> {
    /// Run the agent in `cwd` with `prompt` and return (stdout, tool calls it made).
    ///
    /// Returning the tool calls is what lets `reads_outside_allowed_root` run at all. An adapter
    /// whose parser records nothing yields an empty list, which is why the module doc insists
    /// containment is the defence: an empty list is indistinguishable from a clean run.
    #[allow(clippy::type_complexity)]
    pub run_agent: &'a (dyn Fn(
        &str,
        &str,
        &Path,
    ) -> Result<(String, Vec<agent_adapter::ToolCallRecord>), String>
                  + Send
                  + Sync),
    /// Run the test suite in `cwd` and return its raw output.
    pub run_tests: &'a (dyn Fn(&Path) -> Result<String, String> + Send + Sync),
}

/// The inputs that describe one validation.
#[derive(Debug, Clone)]
pub struct BlindValidationRequest {
    /// The worktree the worker built in. The validator must never see this.
    pub impl_worktree: PathBuf,
    /// A directory containing the contract and nothing else. The validator's whole world.
    pub validator_dir: PathBuf,
    /// The contract text, which carries the API surface.
    pub contract: String,
    /// Where the validator writes its tests, relative to `validator_dir`.
    pub test_file_rel: String,
    /// The command used to run tests, quoted into the prompt so the validator writes for it.
    pub test_command: String,
    /// Who wrote the code. Cannot be the validator.
    pub worker_agent: String,
    /// Candidate validators, in preference order.
    pub roster: Vec<String>,
    /// The suite's outcome BEFORE the worker ran. Without this, pre-existing failures are
    /// blamed on the worker.
    pub baseline: TestRun,
}

/// Run one blind validation, end to end.
///
/// ORDER MATTERS and every step is a refusal point:
///
/// 1. Pick a validator that is not the worker. No validator, no validation.
/// 2. Stage the contract into the validator's own directory. If the implementation is reachable
///    from there, stop: the run cannot be blind, so it cannot be an answer.
/// 3. Let the validator write tests, seeing only that directory.
/// 4. Check what it touched. A violation ends the run before any verdict is formed.
/// 5. Copy the tests into the implementation worktree and run the suite THERE.
/// 6. Diff against the baseline.
///
/// Step 4 is deliberately before step 5. Running the tests of a sighted validator and then
/// discarding the result would waste the run and, worse, would produce a number that somebody
/// eventually quotes.
pub fn run_blind_validation(
    req: &BlindValidationRequest,
    cb: &BlindValidationCallbacks<'_>,
) -> Result<BlindReport, String> {
    let validator = pick_validator(&req.worker_agent, &req.roster)?;

    // The validator's directory must NOT contain the implementation. Checked rather than
    // assumed, because the entire guarantee rests on it and a caller that passes the same path
    // twice would otherwise get a confident, meaningless PASS.
    let impl_canon = canonical(&req.impl_worktree);
    let val_canon = canonical(&req.validator_dir);
    if val_canon == impl_canon || val_canon.starts_with(&impl_canon) {
        return Err(format!(
            "the validator directory {} is inside the implementation worktree {}. The validator \
             would be able to read the code it is supposed to be blind to, so this run could not \
             mean anything. Give it a directory of its own.",
            val_canon.display(),
            impl_canon.display()
        ));
    }

    std::fs::create_dir_all(&req.validator_dir)
        .map_err(|e| format!("could not create the validator directory: {e}"))?;
    let contract_path = req.validator_dir.join("CONTRACT.md");
    std::fs::write(&contract_path, &req.contract)
        .map_err(|e| format!("could not stage the contract: {e}"))?;

    let test_path = req.validator_dir.join(&req.test_file_rel);
    if let Some(parent) = test_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create the validator test directory: {e}"))?;
    }

    let prompt = build_validator_prompt(
        &req.contract,
        &test_path.to_string_lossy(),
        &req.test_command,
    );
    let (_answer, tool_calls) = (cb.run_agent)(&validator, &prompt, &req.validator_dir)?;

    let violations =
        reads_outside_allowed_root(&tool_calls, &req.validator_dir.to_string_lossy());
    if !violations.is_empty() {
        // Stop here. Do not run the tests: see the ordering note above.
        return Ok(BlindReport {
            validator_agent: validator,
            blind_tests_passed: false,
            newly_failing: Vec::new(),
            blindness_violations: violations,
        });
    }

    let written = std::fs::read_to_string(&test_path).map_err(|e| {
        format!(
            "{validator} did not write tests to {}: {e}. Without tests there is nothing to run, \
             and an empty run must not be read as a pass.",
            test_path.display()
        )
    })?;
    if written.trim().is_empty() {
        return Err(format!(
            "{validator} wrote an EMPTY test file at {}. An empty suite passes trivially, which \
             would report the strongest possible result for the weakest possible reason.",
            test_path.display()
        ));
    }

    let dest = req.impl_worktree.join(&req.test_file_rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("could not create the destination test directory: {e}"))?;
    }
    std::fs::copy(&test_path, &dest)
        .map_err(|e| format!("could not copy the blind tests into the worktree: {e}"))?;

    let output = (cb.run_tests)(&req.impl_worktree)?;
    let after = parse_cargo_test_output(&output);

    // The blind tests are the ones the validator just wrote. Identified by their presence in
    // this run and absence from the baseline, which is exactly what a new test file produces.
    let blind_tests: Vec<&String> = after.keys().filter(|k| !req.baseline.contains_key(*k)).collect();
    let blind_tests_passed = !blind_tests.is_empty()
        && blind_tests.iter().all(|k| after.get(*k) == Some(&TestOutcome::Passed));

    Ok(BlindReport {
        validator_agent: validator,
        blind_tests_passed,
        newly_failing: newly_failing(&req.baseline, &after),
        blindness_violations: Vec::new(),
    })
}

/// Canonicalise a path that may not exist yet, by canonicalising its deepest existing ancestor
/// and re-joining the rest.
///
/// A plain `canonicalize().unwrap_or(as-given)` is NOT enough here and the difference is a real
/// bypass, not a test artifact. The validator directory is created AFTER the containment check,
/// so it does not exist when the check runs. On macOS a tempdir lives at `/var/...` and
/// canonicalises to `/private/var/...`, so the existing implementation worktree normalised and
/// the not-yet-created validator directory did not. `starts_with` then compared
/// `/var/x/impl/validator` against `/private/var/x/impl` and found no overlap, so a validator
/// placed INSIDE the implementation worktree passed the check and returned a confident,
/// meaningless PASS.
///
/// Found by `bv_25` failing for a reason that was not the one it was written to test, which is
/// the third time in this work that a path-normalisation mismatch has masqueraded as something
/// else.
fn canonical(p: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    let mut current = p.to_path_buf();
    while let Some(parent) = current.parent().map(Path::to_path_buf) {
        let Some(name) = current.file_name().map(|n| n.to_os_string()) else {
            break;
        };
        suffix.push(name);
        if let Ok(c) = std::fs::canonicalize(&parent) {
            let mut out = c;
            for part in suffix.iter().rev() {
                out.push(part);
            }
            return out;
        }
        current = parent;
    }
    p.to_path_buf()
}

#[cfg(test)]
mod orchestration_tests {
    use super::*;
    use agent_adapter::{ToolCallRecord, ToolKind};

    struct Harness {
        writes: String,
        tool_calls: Vec<ToolCallRecord>,
        test_output: String,
    }

    fn run(h: &Harness, req: &BlindValidationRequest) -> Result<BlindReport, String> {
        let run_agent = |_agent: &str, _prompt: &str, cwd: &Path| {
            // The stand-in writes where a real validator was told to write.
            let path = cwd.join(&req.test_file_rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, &h.writes).unwrap();
            Ok((String::from("done"), h.tool_calls.clone()))
        };
        let run_tests = |_cwd: &Path| Ok(h.test_output.clone());
        run_blind_validation(
            req,
            &BlindValidationCallbacks { run_agent: &run_agent, run_tests: &run_tests },
        )
    }

    fn request(dir: &Path) -> BlindValidationRequest {
        let impl_wt = dir.join("impl");
        let val = dir.join("validator");
        std::fs::create_dir_all(&impl_wt).unwrap();
        BlindValidationRequest {
            impl_worktree: impl_wt,
            validator_dir: val,
            contract: "pub fn duration_secs(s: &str) -> Option<u64>".to_string(),
            test_file_rel: "tests/blind.rs".to_string(),
            test_command: "cargo test".to_string(),
            worker_agent: "codex".to_string(),
            roster: vec!["codex".into(), "claude".into(), "grok".into()],
            baseline: parse_cargo_test_output("test existing::a ... ok\n"),
        }
    }

    /// THE HAPPY PATH, and it must be a real pass rather than a vacuous one: the blind tests are
    /// the ones absent from the baseline, and there must be at least one.
    /// RED IF: a run with no new tests reports success.
    #[test]
    fn bv_20_a_worktree_that_did_the_job_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let req = request(dir.path());
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... ok\ntest blind_a ... ok\n".to_string(),
        };
        let report = run(&h, &req).unwrap();
        assert_eq!(report.validator_agent, "claude", "must not be the worker");
        assert!(report.accepted(), "{:?}", report.why_rejected());
    }

    /// The failure this whole feature exists to catch: the worktree did not do what it was told.
    /// RED IF: a failing blind test is accepted.
    #[test]
    fn bv_21_a_worktree_that_did_not_do_the_job_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let req = request(dir.path());
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... ok\ntest blind_a ... FAILED\n".to_string(),
        };
        let report = run(&h, &req).unwrap();
        assert!(!report.accepted());
        assert!(report.why_rejected().unwrap().contains("did not do what it was dispatched"));
    }

    /// An EMPTY suite passes trivially. A validator that wrote no tests must be an error, not the
    /// strongest possible result for the weakest possible reason.
    /// RED IF: a run with zero new tests reports blind_tests_passed.
    #[test]
    fn bv_22_a_validator_that_wrote_no_tests_is_not_a_pass() {
        let dir = tempfile::tempdir().unwrap();
        let req = request(dir.path());

        // Nothing new in the run at all.
        let h = Harness {
            writes: "// no tests here".to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... ok\n".to_string(),
        };
        let report = run(&h, &req).unwrap();
        assert!(!report.blind_tests_passed, "no new tests ran, so nothing was validated");
        assert!(!report.accepted());

        // And a literally empty file is refused before anything is run.
        let h2 = Harness { writes: "   \n".to_string(), ..Harness { writes: String::new(), tool_calls: vec![], test_output: String::new() } };
        let err = run(&h2, &req).unwrap_err();
        assert!(err.contains("EMPTY test file"), "got: {err}");
    }

    /// Collateral damage, caught by the baseline and attributed correctly.
    /// RED IF: a test that broke outside the contract goes unreported.
    #[test]
    fn bv_23_breaking_an_unrelated_test_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let req = request(dir.path());
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... FAILED\ntest blind_a ... ok\n".to_string(),
        };
        let report = run(&h, &req).unwrap();
        assert!(report.blind_tests_passed, "the contract itself was met");
        assert!(!report.accepted(), "but something else broke");
        assert_eq!(report.newly_failing, vec!["existing::a".to_string()]);
    }

    /// A sighted validator ends the run BEFORE the tests are executed. Running them and then
    /// discarding the result would waste the run and produce a number somebody eventually quotes.
    /// RED IF: the tests are run despite a violation, or a violated run is accepted.
    #[test]
    fn bv_24_a_sighted_validator_stops_before_the_tests_run() {
        let dir = tempfile::tempdir().unwrap();
        let req = request(dir.path());
        let peeked = req.impl_worktree.join("src/thing.rs").to_string_lossy().to_string();
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![ToolCallRecord {
                id: Some("t".into()),
                tool: "Read".into(),
                kind: ToolKind::ReadFile,
                success: Some(true),
                duration_ms: None,
                args_json: Some(format!("{{\"file_path\":\"{peeked}\"}}")),
            }],
            // Would be a clean pass if it were ever consulted.
            test_output: "test existing::a ... ok\ntest blind_a ... ok\n".to_string(),
        };
        let report = run(&h, &req).unwrap();
        assert!(!report.accepted());
        assert!(!report.blindness_violations.is_empty());
        assert!(
            report.newly_failing.is_empty() && !report.blind_tests_passed,
            "no verdict may be formed from a sighted run: {report:?}"
        );
        assert!(report.why_rejected().unwrap().contains("NOT BLIND"));
    }

    /// The containment check on the ARGUMENTS themselves. A caller that points the validator at
    /// the implementation worktree would otherwise get a confident, meaningless PASS.
    /// RED IF: the two directories are allowed to overlap.
    #[test]
    fn bv_25_the_validator_may_not_live_inside_the_implementation() {
        let dir = tempfile::tempdir().unwrap();
        let mut req = request(dir.path());
        req.validator_dir = req.impl_worktree.join("validator");
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            test_output: "test blind_a ... ok\n".to_string(),
        };
        let err = run(&h, &req).unwrap_err();
        assert!(err.contains("inside the implementation worktree"), "got: {err}");
    }
}
