//! Blind validation: a DIFFERENT agent writes the tests, and it never sees the code.
//!
//! WHY THIS EXISTS. Every layer built before this proves a reviewer LOOKED. The sight gate
//! proves a tool call named the artifact; the nonce proves the read reached the end of the file;
//! the partial-read rule proves it was the whole file. None of them prove the reviewer JUDGED
//! anything. All three peers said so independently and the limit was documented as unbuilt,
//! because on prose there is nothing mechanical left to check: an agent can read every byte and
//! write its verdict from nothing.
//!
//! On CODE there is more to work with. A different agent writes tests from the contract and
//! those tests are RUN, so the result is the work being exercised rather than an opinion about
//! it.
//!
//! Not "a verdict nobody has to trust", which is what this said until round 6 and which the rest
//! of this comment then spends sixty lines retracting. Grok pointed out the contradiction twice.
//! What you still have to trust is that the tests are not weak, and for the most common case
//! (a contract naming an API that does not exist yet) nothing here establishes that. See the
//! limit stated at the end of this block.
//!
//! THE BLINDNESS IS MECHANICAL, NOT REQUESTED.
//!
//! If the validator can read the implementation it will write tests that mirror it, and a
//! tautology passes. Asking it not to look is worth nothing: this repo has already watched an
//! agent describe its own toolless output as "rigorous sourcing".
//!
//! So the validator runs in its OWN directory, which this module requires to be EMPTY before it
//! stages the contract into it. Keeping the implementation off that disk is the CALLER's job:
//! this module refuses an overlapping or non-empty directory, and it cannot make a worktree.
//! Grok's round 4 wording is the honest one, and it applies until a caller exists: containment
//! here is a precondition that is checked, not a sandbox that is applied.
//!
//! ```text
//!   impl worktree/                 validator dir/
//!     src/thing.rs      <-----       (absent)
//!     the contract      ----->       CONTRACT.md
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
//! With it, the tests compile, and "built the wrong thing" still fails, because the CONTRACT is
//! the authority rather than whatever happens to exist in the worktree. Both of those hold. What
//! does NOT follow from them is anything about assertion strength, which is the next paragraph.
//!
//! WHAT BLINDNESS DOES NOT BUY, corrected after Grok's round 4.
//!
//! An earlier version of this comment claimed a tautology was "impossible" because the validator
//! cannot see the code. That was wrong. Blindness stops a tautology built FROM the
//! implementation. It does nothing about a WEAK test: `assert!(true)`, `let _ = f(x);` and
//! `assert!(f(x).is_some())` all pass against a wrong body, and the prompt merely ASKS for
//! better. Asking is what this repo has repeatedly proven worthless.
//!
//! `classify_baseline_proof` is a PARTIAL answer, and round 5 is the second time this comment
//! has had to stop overclaiming about it. The first version said a tautology was impossible
//! because of blindness. The second said red/green was "the mechanical answer" to weak tests.
//! Grok refuted that too, and the refutation is the important part of this file:
//!
//! Red/green catches a weak test only when that test COMPILES against the old tree.
//! `assert!(true)` does, so it is caught. `let _ = f(x);` and `assert!(f(x).is_some())` do NOT
//! when `f` is the new thing the contract asked for: the old tree cannot build them, cargo emits
//! no test lines, and treating that silence as redness let a build failure stand in for a proof.
//!
//! That case is now reported as `InconclusiveNoApi` rather than counted as proof. It does not
//! block, because blocking it would reject every honest dispatch that adds a new API, which is
//! most of them. It is not proof of assertion strength and no longer claims to be.
//!
//! So, said plainly and without a third overclaim: this catches a tautology written against
//! EXISTING code. It does not catch a weak test written against a NEW API. Nothing here does.
//! That would need a mutation run (break the implementation deliberately and require the blind
//! tests to go red) and it is not built.
//!
//! Also corrected: there is NO "vague contract" rejection anywhere in this crate. The earlier
//! comment said one was a feature of the design. Whoever writes the brief still carries that
//! cost, but nothing here enforces it.

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
/// A previously-PASSING test that disappeared counts here. A previously-FAILING one does not,
/// and that gap is why `deleted_tests` exists alongside this. See FIND-BLIND-02.
pub fn newly_failing(baseline: &TestRun, after: &TestRun) -> Vec<String> {
    baseline
        .iter()
        .filter(|(_, outcome)| **outcome == TestOutcome::Passed)
        .filter(|(name, _)| after.get(*name) != Some(&TestOutcome::Passed))
        .map(|(name, _)| name.clone())
        .collect()
}

/// Tests present at baseline in ANY state and absent from the run now.
///
/// FIND-BLIND-02, found by Grok. `newly_failing` filters the baseline down to tests that were
/// PASSING, so a test that was already red and then vanished was invisible. The doc on that
/// function claimed "a test that DISAPPEARED counts as newly failing... deleting a failing test
/// is the cheapest way to make a suite green". It named the cheap cheat and then tested the
/// other one: `bv_03` deletes a test that was GREEN.
///
/// So deletion is tracked on its own, whatever the test's previous state. A worker is not
/// forbidden from deleting a test, but it does not get to do it silently while the report says
/// nothing changed.
pub fn deleted_tests(baseline: &TestRun, after: &TestRun) -> Vec<String> {
    baseline
        .keys()
        .filter(|name| !after.contains_key(*name))
        .cloned()
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

/// The test function names DECLARED in the validator's file.
///
/// FIND-BLIND-01, found by Codex. The first version inferred "the blind tests" as whatever was
/// present after the run and absent from the baseline. Codex's attack: the WORKER adds its own
/// passing test, the validator writes a non-empty file containing no tests at all, and the
/// worker's own test is then counted as a blind test that passed. `accepted()` returned true
/// with ZERO validator tests having run. The worker validates itself, which is precisely the
/// thing this feature exists to prevent.
///
/// So the names come from the file the validator wrote. A test that is not in that file is not
/// evidence about the contract, whoever wrote it and whenever it appeared.
///
/// Recognises `#[test]` and the attribute forms that wrap it (`#[tokio::test]`,
/// `#[actix_rt::test]`), because a validator writing async tests is ordinary and treating those
/// as "no tests" would be a false rejection.
pub fn declared_test_names(source: &str) -> Vec<String> {
    let source = &strip_comments_and_strings(source);
    let mut names = Vec::new();
    let mut rest = source.as_str();
    // Scans the whole source rather than line by line. The first version was line-based and
    // could not see `#[test] fn blind_a() {}` written on ONE line, which is exactly how a
    // compact test file looks and exactly how the harness writes them. It reported "no tests
    // declared" for a file full of tests, which would have rejected honest validators.
    while let Some(attr) = rest.find("test]") {
        rest = &rest[attr + "test]".len()..];
        // The next `fn` after the attribute. Bounded so an attribute at the end of a file with
        // no following function does not run away into unrelated code.
        let window = &rest[..rest.len().min(400)];
        let Some(fn_at) = find_fn_keyword(window) else {
            continue;
        };
        let after = &window[fn_at..];
        let ident: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !ident.is_empty() {
            names.push(ident);
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Blank out comments and string literals so they cannot declare tests.
///
/// Codex found in round 5 that a file containing only `// #[test] fn x() {}` declared `x`. On
/// its own that is a false report; chained with the name-collision hole it found in the same
/// pass, a validator that wrote nothing but a comment could have its "test" satisfied by one the
/// WORKER added. Two independent defects composing into a false accept.
///
/// Replaces content with spaces rather than removing it, so nothing that follows shifts and the
/// 400-byte window after an attribute keeps meaning the same thing.
///
/// Handles `//`, `/* */` (nested, as Rust allows), `"..."` with escapes, and `r#"..."#`. This is
/// not a Rust tokeniser and does not claim to be. Its failure directions are both safe: an
/// over-report requires a test that does not exist to pass, which rejects, and an under-report
/// shrinks the declared set toward empty, which is never a pass.
fn strip_comments_and_strings(source: &str) -> String {
    let b = source.as_bytes();
    let mut out = vec![b' '; b.len()];
    let mut i = 0;
    while i < b.len() {
        // Line comment.
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment, nested.
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let mut depth = 1;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // CHAR AND BYTE-CHAR LITERALS. Codex found this in round 6 and it is a bad-worktree-pass,
        // not a wording issue.
        //
        // `'"'` is a valid Rust char literal containing a double quote. Without this arm the
        // scanner saw that quote, believed a string had opened, and blanked THE REST OF THE FILE
        // hunting for a close that never comes. Every test declared after such a line vanished
        // from the declared set, so a validator test that FAILED was simply not checked and the
        // worktree passed. Under-reporting is the dangerous direction and only over-reporting
        // had been guarded.
        //
        // `'` is also the LIFETIME sigil (`&'a str`), so it cannot simply be treated as an
        // opener. A char literal is a quote followed either by an escape, or by exactly one
        // character and a closing quote. Anything else is a lifetime and is left alone.
        if b[i] == b'\'' || (b[i] == b'b' && i + 1 < b.len() && b[i + 1] == b'\'') {
            let q = if b[i] == b'b' { i + 1 } else { i };
            let body = q + 1;
            let is_escape = body < b.len() && b[body] == b'\\';
            let is_single = body + 1 < b.len() && b[body + 1] == b'\'';
            if is_escape {
                // `'\n'`, `'\''`, `'\\'`, and `'\u{1F600}'`.
                let mut j = body + 1;
                if j < b.len() && b[j] == b'u' {
                    while j < b.len() && b[j] != b'}' {
                        j += 1;
                    }
                }
                while j < b.len() && b[j] != b'\'' {
                    j += 1;
                }
                i = (j + 1).min(b.len());
                continue;
            }
            if is_single {
                i = body + 2;
                continue;
            }
            // A lifetime. Copy the tick and carry on; the identifier after it is ordinary code.
            out[i] = b[i];
            i += 1;
            continue;
        }
        // Byte string: b"..."
        if b[i] == b'b' && i + 1 < b.len() && b[i + 1] == b'"' {
            i += 1;
            continue;
        }
        // Raw string: r#"..."# with any number of hashes.
        if b[i] == b'r' {
            let mut j = i + 1;
            let mut hashes = 0;
            while j < b.len() && b[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < b.len() && b[j] == b'"' {
                let close = format!("\"{}", "#".repeat(hashes));
                let after = j + 1;
                let end = source[after..].find(&close).map(|p| after + p + close.len());
                i = end.unwrap_or(b.len());
                continue;
            }
        }
        // Ordinary string, with escapes.
        if b[i] == b'"' {
            i += 1;
            let mut closed = false;
            while i < b.len() {
                if b[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if b[i] == b'"' {
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                // Ran to EOF inside a string. The file is not valid Rust, OR this scanner
                // mis-identified an opener and has just swallowed the tail. Either way the
                // declared set is now an UNDER-report, which is the direction that lets a
                // failing test disappear from the acceptance criteria.
                //
                // Signalled rather than absorbed: `declared_test_names` turns this into an empty
                // result, and an empty declared set is never a pass.
                return String::new();
            }
            continue;
        }
        out[i] = b[i];
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Byte offset just past the next `fn ` keyword, at a word boundary.
///
/// A bare `find("fn ")` also matches inside identifiers such as `my_fn foo`, so the character
/// before it has to be a non-identifier one.
fn find_fn_keyword(haystack: &str) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let mut i = 0;
    while let Some(rel) = haystack[i..].find("fn ") {
        let at = i + rel;
        let prev_ok = at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        if prev_ok {
            let mut j = at + 3;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            return Some(j);
        }
        i = at + 3;
    }
    None
}

/// Did every test the validator DECLARED actually run and pass?
///
/// Matched by suffix, because the harness reports a module path (`blind::duration_parses`) while
/// the file declares a bare name (`duration_parses`). Matching on the whole string would fail
/// every real run; matching on a bare `contains` would let `not_duration_parses` satisfy
/// `duration_parses`, so the boundary is checked.
///
/// An empty declaration list is NEVER a pass. That is the second half of Codex's finding: a
/// non-empty file with no tests in it must not be able to borrow somebody else's green.
pub fn declared_tests_all_passed(declared: &[String], run: &TestRun) -> bool {
    if declared.is_empty() {
        return false;
    }
    declared.iter().all(|name| {
        let candidates = name_candidates(run, name);
        // At least one test answering to this name, and EVERY one of them passed.
        //
        // `any` was the first version and Codex broke it in round 5: with `blind::x ... FAILED`
        // and `worker_added::x ... ok`, declared `x` was satisfied by the WORKER's test while
        // the validator's own test of that name had failed. That is the self-validation family
        // again, re-entered through a name collision rather than through the baseline diff.
        //
        // When two tests answer to one name, this cannot tell which is the validator's, so it
        // requires all of them. A collision therefore rejects rather than picks the convenient
        // one, which is the correct direction for a gate.
        !candidates.is_empty() && candidates.iter().all(|(_, o)| **o == TestOutcome::Passed)
    })
}

/// Every test in the run answering to a bare declared name.
///
/// Suffix matching is necessary because the harness reports a module path
/// (`blind::duration_parses`) while the file declares a bare name (`duration_parses`). It is
/// anchored on `::` so `not_duration_parses` cannot satisfy `duration_parses`.
fn name_candidates<'a>(run: &'a TestRun, name: &str) -> Vec<(&'a String, &'a TestOutcome)> {
    let suffix = format!("::{name}");
    run.iter()
        .filter(|(reported, _)| *reported == name || reported.ends_with(&suffix))
        .collect()
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
    // JSON may escape a forward slash: `"\/tmp\/impl\/x.rs"` is valid and means `/tmp/impl/x.rs`.
    // Codex found that the raw scan missed it entirely, so an escaped absolute path was reported
    // as clean. Unescaped first, so the scan sees the path the agent actually opened.
    let args_json = &args_json.replace("\\/", "/");
    // A `/` only starts an ABSOLUTE path when it opens a token. Without that check the relative
    // path `src/thing.rs` yields `/thing.rs`, which is not a path anyone wrote and which then
    // gets reported as an escape from the validator's directory. Caught by `bv_09`, which was
    // asserting the opposite property and failed for this reason rather than for its own.
    fn opens_a_token(prev: Option<u8>) -> bool {
        match prev {
            None => true,
            // `:` is deliberately NOT here. Codex found that including it made
            // `https://example.com/x` scan as the path `//example.com/x`, which is outside any
            // validator root, so a tool argument carrying a URL would REFUSE an honest blind
            // run. That fails closed rather than open, but a gate that rejects correct work is
            // how gates get switched off.
            Some(c) => matches!(c, b'"' | b' ' | b'\'' | b'=' | b'(' | b'[' | b',' | b'\t'),
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

/// What the pre-change run actually established about the blind tests.
///
/// FOUR OUTCOMES, NOT A BOOLEAN. FIND-BLIND-05, found by Grok in round 5. (An earlier
/// version of this line said THREE, while listing four variants directly beneath it.)
///
/// The first version asked one question, "was this name Passed at baseline", and treated every
/// other answer as redness. Grok showed what that means in the case this feature will actually
/// see most: a contract for an API that does not exist yet. The blind tests do not COMPILE
/// against the old tree, so `cargo test` emits no per-test lines at all, so every declared name
/// is absent, so the proof passes. `assert!(duration_secs("5m").is_some())` was therefore
/// "proven" by a build failure, and it asserts nothing about the contract.
///
/// Absence and failure are different evidence and are now different answers:
///
/// - `Proven`: every declared test RAN and did not pass. Red then green. The strong case.
/// - `Refuted`: some declared test was already GREEN. A tautology, or behaviour that already
///   existed. This blocks.
/// - `InconclusiveNoApi`: the old tree could not build the tests, which is expected and
///   legitimate when the contract adds something new. It is NOT proof of assertion strength and
///   no longer pretends to be. It does not block, because blocking it would reject every honest
///   new-API dispatch.
/// - `InconclusiveNoEvidence`: no test lines and no sign of a build failure either. The caller
///   probably did not run the blind tests at all, which Grok noted is easy to get wrong because
///   this module copies them only into the post-change tree. This blocks: it is indistinguishable
///   from a check that never happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaselineProof {
    Proven,
    Refuted { already_green: Vec<String> },
    InconclusiveNoApi,
    InconclusiveNoEvidence,
}

impl BaselineProof {
    /// Does this outcome stop the run?
    ///
    /// Only `Refuted` and `InconclusiveNoEvidence`. See the enum docs: refusing
    /// `InconclusiveNoApi` would reject every honest dispatch that adds a new API, which is most
    /// of them.
    pub fn blocks(&self) -> bool {
        matches!(self, Self::Refuted { .. } | Self::InconclusiveNoEvidence)
    }
}

/// Does this output look like the compiler refused to build?
///
/// Used to tell "the old tree had no such API" from "nobody ran anything". Matches the shapes
/// cargo and rustc actually emit. Deliberately narrow: an unrecognised empty output is treated
/// as no evidence, which blocks, rather than as a build failure, which would not.
pub fn looks_like_build_failure(output: &str) -> bool {
    output.contains("error[E")
        || output.contains("error: could not compile")
        || output.contains("error: aborting")
        || output.contains("cannot find function")
        || output.contains("unresolved import")
}

/// Classify what the pre-change run proved about the declared tests.
pub fn classify_baseline_proof(
    declared: &[String],
    baseline_output: &str,
    baseline_run: &TestRun,
) -> BaselineProof {
    if declared.is_empty() {
        return BaselineProof::InconclusiveNoEvidence;
    }

    // A name that was GREEN before the change proves nothing about the change, whatever else
    // happened in the run. Checked first: it is the only conclusive negative.
    let already_green: Vec<String> = declared
        .iter()
        .filter(|name| {
            name_candidates(baseline_run, name)
                .iter()
                .any(|(_, o)| **o == TestOutcome::Passed)
        })
        .cloned()
        .collect();
    if !already_green.is_empty() {
        return BaselineProof::Refuted { already_green };
    }

    let ran: Vec<&String> = declared
        .iter()
        .filter(|name| !name_candidates(baseline_run, name).is_empty())
        .collect();

    if ran.len() == declared.len() {
        return BaselineProof::Proven;
    }
    if looks_like_build_failure(baseline_output) {
        return BaselineProof::InconclusiveNoApi;
    }
    BaselineProof::InconclusiveNoEvidence
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
    /// What the pre-change run established. See `BaselineProof`.
    pub baseline_proof: BaselineProof,
    /// Tests that were green before the worker ran and are not green now.
    pub newly_failing: Vec<String>,
    /// Tests present at baseline in ANY state and gone now. FIND-BLIND-02.
    pub deleted: Vec<String>,
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
            && !self.baseline_proof.blocks()
            && self.newly_failing.is_empty()
            && self.deleted.is_empty()
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
        match &self.baseline_proof {
            BaselineProof::Refuted { already_green } => {
                return Some(format!(
                    "{} test(s) written by {} were ALREADY GREEN before this dispatch: {}. A test \
                     that passed against the old tree is not evidence that the change works: \
                     `assert!(true)` passes both times, and so does a test of behaviour that \
                     already existed. Re-run the validator asking for tests of the NEW behaviour \
                     the contract names.",
                    already_green.len(),
                    self.validator_agent,
                    already_green.join(", ")
                ));
            }
            BaselineProof::InconclusiveNoEvidence => {
                return Some(format!(
                    "the pre-change run produced no result for {}'s tests and no sign of a build \
                     failure either, so nothing was established. The usual cause is that the \
                     blind tests were never copied into the pre-change tree: this module copies \
                     them only into the post-change worktree, and the caller must arrange the \
                     other. An unproven suite is indistinguishable from a tautological one.",
                    self.validator_agent
                ));
            }
            BaselineProof::Proven | BaselineProof::InconclusiveNoApi => {}
        }
        if !self.deleted.is_empty() {
            // The message names BOTH explanations, because this cannot tell them apart.
            //
            // Grok found in round 5 that a key-set difference assumes both runs enumerated the
            // same tests. A different filter, a different feature set, or a test binary that
            // failed to compile all look exactly like deletion. Reporting only the accusation
            // would blame an honest worker, and a gate that rejects correct work while lying
            // about why is how gates get switched off.
            return Some(format!(
                "{} test(s) present before this dispatch did not appear in this run: {}. Two \
                 explanations and this cannot distinguish them. Either the worker deleted them, \
                 which is the cheapest way to make a suite green and is why this blocks, OR the \
                 two runs did not enumerate the same tests: a different filter or feature set \
                 between the baseline capture and this run, or a test binary that failed to \
                 compile and therefore reported nothing. Check the two commands match before \
                 concluding the worker did it.",
                self.deleted.len(),
                self.deleted.join(", ")
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

    /// A previously-PASSING test that disappeared counts as newly failing.
    ///
    /// The doc here used to say "deleting a FAILING test" while the body deletes a green one,
    /// which is the mismatch Grok turned into FIND-BLIND-02: the prose named the cheap cheat and
    /// the test covered the other one. The failing-test case is now `bv_33`, against
    /// `deleted_tests`, which is where it belongs.
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

    /// CODEX ROUND 4, FINDING 3. A JSON-escaped slash is still a slash.
    ///
    /// `{"file_path":"\\/tmp\\/impl\\/x.rs"}` is valid JSON meaning `/tmp/impl/x.rs`. The raw
    /// scan missed it entirely and reported the run as clean, so an escaped outside read passed
    /// the blindness check.
    /// RED IF: the unescaping step is removed.
    #[test]
    fn bv_13_an_escaped_path_is_still_a_path() {
        let calls = vec![read_call(r#"{"file_path":"\/tmp\/impl\/src\/thing.rs"}"#)];
        assert_eq!(
            reads_outside_allowed_root(&calls, "/tmp/validator"),
            vec!["/tmp/impl/src/thing.rs".to_string()]
        );
    }

    /// CODEX ROUND 4, FINDING 4. A URL is not a filesystem escape.
    ///
    /// Treating `:` as a token opener made `https://example.com/x` scan as the path
    /// `//example.com/x`, outside any validator root, so a tool argument carrying a URL would
    /// REFUSE an honest blind run. It fails closed rather than open, but a gate that rejects
    /// correct work is how gates get switched off.
    /// RED IF: `:` returns to the opener set.
    #[test]
    fn bv_14_a_url_is_not_a_path_escape() {
        let calls = vec![read_call(r#"{"url":"https://example.com/docs/x","note":"fetch"}"#)];
        assert!(
            reads_outside_allowed_root(&calls, "/tmp/validator").is_empty(),
            "a URL in tool arguments must not be reported as reading outside the directory"
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
            baseline_proof: BaselineProof::Proven,
            newly_failing: vec![],
            deleted: vec![],
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
            baseline_proof: BaselineProof::Proven,
            newly_failing: vec![],
            deleted: vec![],
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
    /// Run the suite with the blind tests present but the worker's change ABSENT, and return the
    /// raw output.
    ///
    /// This is what makes FIND-BLIND-04 checkable. The caller decides how to produce that tree,
    /// because only the caller knows how the worktree was made: a second worktree at the base
    /// commit, a stash, a `git worktree add <base>`. The check does not care how, only that the
    /// blind tests are exercised against code that predates the change.
    ///
    /// The RAW output is returned, not a parsed run, because telling "the old tree could not
    /// build these tests" from "nobody ran anything" needs the compiler's own words.
    ///
    /// The caller must ensure the blind tests are actually PRESENT in that tree. This module
    /// copies them only into the post-change worktree, so arranging the other is the caller's
    /// job, and Grok noted in round 5 that it is easy to get wrong: a caller that runs the suite
    /// on a clean base worktree without copying the tests over produces silence, which used to
    /// read as proof and now reads as `InconclusiveNoEvidence`.
    ///
    /// Returning `Ok(None)` means the caller could not produce such a tree at all. That is not a
    /// pass either.
    #[allow(clippy::type_complexity)]
    pub run_tests_at_baseline: &'a (dyn Fn(&Path) -> Result<Option<String>, String> + Send + Sync),
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
    // SYMMETRIC. Codex found the inverse: rejecting only "validator inside implementation" left
    // "implementation inside validator" wide open. A caller passing `/tmp/job` as the validator
    // directory and `/tmp/job/impl` as the worktree gives the validator an allowed root that
    // CONTAINS the code, and `reads_outside_allowed_root` then passes every read of it as
    // legitimately inside. The blindness would be nominal and the report would say it held.
    if val_canon == impl_canon
        || val_canon.starts_with(&impl_canon)
        || impl_canon.starts_with(&val_canon)
    {
        return Err(format!(
            "the validator directory {} and the implementation worktree {} overlap. One contains \
             the other, so the validator can read the code it is supposed to be blind to and the \
             blindness check would pass those reads as legitimately inside its own root. Give \
             each a directory of its own.",
            val_canon.display(),
            impl_canon.display()
        ));
    }

    // The validator's directory must START EMPTY, or "contains the contract and nothing else" is
    // a comment rather than a fact. Grok pointed out that the orchestration never emptied it, so
    // a pre-populated directory kept whatever was there, including anything a previous run or a
    // caller had left. Refused rather than cleared: silently deleting a caller's files is worse
    // than declining to run.
    if let Ok(mut entries) = std::fs::read_dir(&req.validator_dir)
        && entries.next().is_some()
    {
        return Err(format!(
            "the validator directory {} is not empty. It must contain the contract and nothing \
             else, or the validator may read whatever was left there and the run is not blind. \
             Refusing rather than deleting somebody's files.",
            req.validator_dir.display()
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

    // CANONICAL root, matching the nesting check above.
    //
    // Grok found that `canonical()` was introduced in this very commit because /var against
    // /private/var let a nested validator pass, and then the fix was applied to ONE comparison
    // while detection kept using the raw path. On macOS a tool emitting the canonical path of a
    // file INSIDE the validator directory was therefore reported as an escape: a false refuse,
    // from the same mismatch, twenty lines apart.
    let violations = reads_outside_allowed_root(&tool_calls, &val_canon.to_string_lossy());
    if !violations.is_empty() {
        // Stop here. Do not run the tests: see the ordering note above.
        return Ok(BlindReport {
            validator_agent: validator,
            blind_tests_passed: false,
            baseline_proof: BaselineProof::InconclusiveNoEvidence,
            newly_failing: Vec::new(),
            deleted: Vec::new(),
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

    // The blind tests are the ones the validator DECLARED, read out of the file it wrote.
    //
    // NOT "present after and absent from the baseline", which is what this used to be and which
    // Codex broke: a worker that adds its own passing test lets an empty validator file borrow
    // that green, and the worker validates itself.
    let declared = declared_test_names(&written);

    let output = (cb.run_tests)(&req.impl_worktree)?;
    let after = parse_cargo_test_output(&output);
    let blind_tests_passed = declared_tests_all_passed(&declared, &after);

    // What the pre-change tree established about those same tests. See `BaselineProof`: this is
    // four-way, because absence and failure are different evidence and absence has two
    // different causes.
    let baseline_output = (cb.run_tests_at_baseline)(&req.impl_worktree)?;
    let baseline_proof = match baseline_output.as_deref() {
        Some(text) => classify_baseline_proof(&declared, text, &parse_cargo_test_output(text)),
        // The caller could not build a pre-change tree at all. No evidence, which blocks.
        None => BaselineProof::InconclusiveNoEvidence,
    };

    Ok(BlindReport {
        validator_agent: validator,
        blind_tests_passed,
        baseline_proof,
        newly_failing: newly_failing(&req.baseline, &after),
        deleted: deleted_tests(&req.baseline, &after),
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

    /// A temp directory created while holding the crate's process-env lock.
    ///
    /// `tempfile::tempdir()` reads `$TMPDIR` through `getenv`, and `abe`'s tests call `set_var`
    /// in this same binary. POSIX `setenv` may reallocate the environ array, so an unguarded
    /// read can follow a freed pointer. Antigravity found this in round 6; it is undefined
    /// behaviour rather than a flaky assertion, so it is fixed at every call site rather than
    /// tolerated.
    ///
    /// The lock is held only for the creation, not for the body of the test.
    fn scratch() -> tempfile::TempDir {
        let _guard = crate::PROCESS_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tempfile::tempdir().expect("tempdir")
    }

    struct Harness {
        writes: String,
        tool_calls: Vec<ToolCallRecord>,
        test_output: String,
        /// What the SAME blind tests do against the pre-change tree. `None` models a caller that
        /// could not build one, which must not read as a pass.
        baseline_output: Option<String>,
    }

    /// The blind tests were RED before the change, which is the ordinary case for a test that
    /// proves new behaviour works.
    fn red_first(names: &[&str]) -> Option<String> {
        Some(
            names
                .iter()
                .map(|n| format!("test blind::{n} ... FAILED\n"))
                .collect::<String>(),
        )
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
        let run_baseline = |_cwd: &Path| Ok(h.baseline_output.clone());
        run_blind_validation(
            req,
            &BlindValidationCallbacks {
                run_agent: &run_agent,
                run_tests: &run_tests,
                run_tests_at_baseline: &run_baseline,
            },
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

    /// THE HAPPY PATH, and it must be a real pass rather than a vacuous one.
    ///
    /// The description here used to say "the blind tests are the ones absent from the baseline",
    /// which was the FIND-BLIND-01 mechanism this work removed: they are now read out of the
    /// file the validator wrote. Grok flagged the stale sentence in rounds 5 and 6. The body was
    /// always a happy path and is unchanged.
    /// RED IF: a run with no declared tests reports success.
    #[test]
    fn bv_20_a_worktree_that_did_the_job_is_accepted() {
        let dir = scratch();
        let req = request(dir.path());
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... ok\ntest blind_a ... ok\n".to_string(),
            baseline_output: red_first(&["blind_a", "checks_five_minutes", "checks_two_hours", "checks_async"]),
        };
        let report = run(&h, &req).unwrap();
        assert_eq!(report.validator_agent, "claude", "must not be the worker");
        assert!(report.accepted(), "{:?}", report.why_rejected());
    }

    /// The failure this whole feature exists to catch: the worktree did not do what it was told.
    /// RED IF: a failing blind test is accepted.
    #[test]
    fn bv_21_a_worktree_that_did_not_do_the_job_is_rejected() {
        let dir = scratch();
        let req = request(dir.path());
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... ok\ntest blind_a ... FAILED\n".to_string(),
            baseline_output: red_first(&["blind_a", "checks_five_minutes", "checks_two_hours", "checks_async"]),
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
        let dir = scratch();
        let req = request(dir.path());

        // Nothing new in the run at all.
        let h = Harness {
            writes: "// no tests here".to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... ok\n".to_string(),
            baseline_output: red_first(&["blind_a", "checks_five_minutes", "checks_two_hours", "checks_async", "freebie"]),
        };
        let report = run(&h, &req).unwrap();
        assert!(!report.blind_tests_passed, "no new tests ran, so nothing was validated");
        assert!(!report.accepted());

        // And a literally empty file is refused before anything is run.
        //
        // A FRESH directory, because the run above already staged a contract into the first one
        // and the non-empty guard would now fire instead. Reusing it made this assert against a
        // different refusal than the one it names, which is the "green for the wrong reason"
        // shape in reverse.
        let dir2 = scratch();
        let req2 = request(dir2.path());
        let h2 = Harness {
            writes: "   \n".to_string(),
            tool_calls: vec![],
            test_output: String::new(),
            baseline_output: None,
        };
        let err = run(&h2, &req2).unwrap_err();
        assert!(err.contains("EMPTY test file"), "got: {err}");
    }

    /// Collateral damage, caught by the baseline and attributed correctly.
    /// RED IF: a test that broke outside the contract goes unreported.
    #[test]
    fn bv_23_breaking_an_unrelated_test_is_reported() {
        let dir = scratch();
        let req = request(dir.path());
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... FAILED\ntest blind_a ... ok\n".to_string(),
            baseline_output: red_first(&["blind_a", "checks_five_minutes", "checks_two_hours", "checks_async"]),
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
        let dir = scratch();
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
            baseline_output: red_first(&["blind_a", "checks_five_minutes", "checks_two_hours", "checks_async"]),
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

    /// CODEX ROUND 4, FINDING 2, THE SEVERE ONE. The worker cannot validate itself.
    ///
    /// The attack, in Codex's words: the worker adds its own passing test before validation,
    /// the validator writes a non-empty file containing NO tests, `run_tests` reports the
    /// worker's test as present-and-absent-from-baseline, and the run is accepted with zero
    /// validator tests having executed.
    ///
    /// `bv_22` missed this because it only covered the case where NOTHING new appeared. The
    /// difference between "no new tests" and "new tests that are not the validator's" is the
    /// whole finding.
    /// RED IF: blind tests are inferred from the baseline diff again rather than read out of
    /// the validator's file.
    #[test]
    fn bv_26_a_worker_added_test_cannot_stand_in_for_the_validators() {
        let dir = scratch();
        let req = request(dir.path());
        let h = Harness {
            // Non-empty, so the empty-file guard does not catch it. Declares no tests.
            writes: "// I looked at the contract and had no notes.\nuse std::fmt;\n".to_string(),
            tool_calls: vec![],
            // The worker added `worker_added::freebie`, which is new and green.
            test_output: "test existing::a ... ok\ntest worker_added::freebie ... ok\n".to_string(),
            baseline_output: red_first(&["blind_a", "checks_five_minutes", "checks_two_hours", "checks_async", "freebie"]),
        };
        let report = run(&h, &req).unwrap();
        assert!(
            !report.blind_tests_passed,
            "the validator declared no tests, so nothing about the contract was checked. A test \
             the WORKER added is not evidence about the worker."
        );
        assert!(!report.accepted(), "the worker must not be able to validate itself");
    }

    /// The other half of the same finding: a declared test that never RAN is not a pass.
    /// A validator can write a test that fails to compile into the run, or name one thing and
    /// define another. Silence is not success.
    /// RED IF: a declared test missing from the run stops mattering.
    #[test]
    fn bv_27_a_declared_test_that_never_ran_is_not_a_pass() {
        let dir = scratch();
        let req = request(dir.path());
        let h = Harness {
            writes: "#[test] fn checks_five_minutes() {}\n#[test] fn checks_two_hours() {}"
                .to_string(),
            tool_calls: vec![],
            // Only one of the two declared tests appears.
            test_output: "test existing::a ... ok\ntest blind::checks_five_minutes ... ok\n"
                .to_string(),
            baseline_output: red_first(&["blind_a", "checks_five_minutes", "checks_two_hours", "checks_async"]),
        };
        let report = run(&h, &req).unwrap();
        assert!(
            !report.blind_tests_passed,
            "checks_two_hours was declared and never ran; a suite that silently dropped half the \
             contract must not report success"
        );
    }

    /// A CONTROL, not a guard, and Antigravity was right to say so in round 5.
    ///
    /// It asserts the happy path: declared tests that ran and passed, matched across the module
    /// path the harness prefixes onto them. That path was already green before FIND-BLIND-01,
    /// so this test would survive reverting that fix. It earns its place by catching the
    /// OPPOSITE failure, a matcher so strict that every honest run fails, which the guards below
    /// cannot see.
    ///
    /// The actual guards for FIND-BLIND-01 are `bv_26` (a worker-added test cannot stand in) and
    /// `bv_27` (a declared test that never ran is not a pass). Both go red when the fix is
    /// reverted; this one does not, and that is stated rather than left for the next reviewer to
    /// discover.
    /// RED IF: suffix matching breaks and every honest run starts failing.
    #[test]
    fn bv_28_declared_tests_are_matched_across_the_module_path() {
        let dir = scratch();
        let req = request(dir.path());
        let h = Harness {
            writes: "#[test]\nfn checks_five_minutes() {}\n\n#[tokio::test]\nasync fn checks_async() {}"
                .to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... ok\n\
                          test blind::checks_five_minutes ... ok\n\
                          test blind::checks_async ... ok\n"
                .to_string(),
            baseline_output: red_first(&["blind_a", "checks_five_minutes", "checks_two_hours", "checks_async"]),
        };
        let report = run(&h, &req).unwrap();
        assert!(report.accepted(), "{:?}", report.why_rejected());
    }

    /// Antigravity, round 5: the canonical detection root had no test.
    ///
    /// `run_blind_validation` passes `val_canon` into `reads_outside_allowed_root` rather than
    /// the raw path. On macOS a tempdir is `/var/...` and canonicalises to `/private/var/...`,
    /// so a tool reporting the canonical path of a file INSIDE the validator directory was
    /// flagged as an escape. A false refuse, from the same normalisation mismatch that caused a
    /// false ACCEPT twenty lines away, and the fix had gone in untested.
    /// RED IF: the detection root goes back to the raw path.
    #[test]
    fn bv_45_a_canonical_path_inside_the_validator_dir_is_not_an_escape() {
        let dir = scratch();
        let req = request(dir.path());

        // A REAL SYMLINK, so the raw and canonical spellings differ on EVERY platform.
        //
        // Antigravity found in round 6 that the first version relied on the macOS
        // /var to /private/var difference. On Linux the two spellings are identical, the buggy
        // string comparison would still have matched, and the test would have been green
        // against broken code. A test that only guards on one platform is not a guard.
        let real = dir.path().join("real-validator-home");
        std::fs::create_dir_all(&real).unwrap();
        let linked = dir.path().join("linked-validator-home");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &linked).unwrap();
        #[cfg(not(unix))]
        let linked = real.clone();

        // The validator is given the path THROUGH the symlink; the tool reports the real one.
        let mut req = req;
        req.validator_dir = linked.join("v");
        let canonical_inside = std::fs::canonicalize(&real)
            .unwrap()
            .join("v")
            .join("CONTRACT.md")
            .to_string_lossy()
            .to_string();

        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![ToolCallRecord {
                id: Some("t".into()),
                tool: "Read".into(),
                kind: ToolKind::ReadFile,
                success: Some(true),
                duration_ms: None,
                args_json: Some(format!("{{\"file_path\":\"{canonical_inside}\"}}")),
            }],
            test_output: "test existing::a ... ok\ntest blind::blind_a ... ok\n".to_string(),
            baseline_output: red_first(&["blind_a"]),
        };
        let report = run(&h, &req).unwrap();
        assert!(
            report.blindness_violations.is_empty(),
            "reading the contract in its own directory is not an escape, whichever spelling of \
             that directory the tool reports: {:?}",
            report.blindness_violations
        );
        assert!(report.accepted(), "{:?}", report.why_rejected());
    }

    /// CODEX ROUND 6, SEVERE. A char literal must not swallow the rest of the file.
    ///
    /// Codex's case, verbatim in shape: `'"'` is a valid Rust char literal containing a double
    /// quote. The scanner saw that quote, believed a string had opened, and blanked everything
    /// after it. `second` then never entered the declared set, so a validator test that FAILED
    /// was not checked and the worktree passed.
    ///
    /// This is the direction that matters. Over-reporting invents a name with no candidate and
    /// REJECTS; under-reporting quietly shrinks what has to pass.
    /// RED IF: char or byte-char literals stop being recognised.
    #[test]
    fn bv_46_a_char_literal_does_not_hide_later_tests() {
        // ONE quote from the char literal, then a normal two-quote string. The count is ODD,
        // which is load bearing: the first version of this test used `'"'` plus `b'"'` plus one
        // string, four quotes in total, and they re-balanced by accident. Removing the fix left
        // the test GREEN, which the mutation caught. A test whose input accidentally cancels the
        // defect proves nothing.
        let src = r#"
            #[test] fn first() {}
            const QUOTE: char = '"';
            #[test] fn second() { let s = "text"; let _ = s; }
        "#;
        assert_eq!(
            declared_test_names(src),
            vec!["first".to_string(), "second".to_string()],
            "a quote inside a char literal must not open a string"
        );

        // The byte-char form, on its own, with the same odd count.
        let byte_src = r#"
            #[test] fn b_first() {}
            const B: u8 = b'"';
            #[test] fn b_second() { let s = "text"; let _ = s; }
        "#;
        assert_eq!(
            declared_test_names(byte_src),
            vec!["b_first".to_string(), "b_second".to_string()]
        );

        // The consequence, which is the part that actually bites: with `second` missing from the
        // declared set, its FAILURE would not be checked at all.
        let run = parse_cargo_test_output("test blind::first ... ok\ntest blind::second ... FAILED\n");
        assert!(
            !declared_tests_all_passed(&declared_test_names(src), &run),
            "second failed, so the suite must not pass"
        );
    }

    /// Lifetimes are not char literals, and the scanner must not eat them.
    ///
    /// `'` opens both. Treating every tick as a literal would consume `'a str` and swallow real
    /// code, which is the same under-report by a different route.
    /// RED IF: lifetimes start being consumed.
    #[test]
    fn bv_47_lifetimes_survive_the_stripper() {
        let src = "fn helper<'a>(s: &'a str) -> &'a str { s }\n#[test] fn after_lifetimes() {}";
        assert_eq!(declared_test_names(src), vec!["after_lifetimes".to_string()]);

        let escapes = "const N: char = '\\n'; const T: char = '\\''; #[test] fn after_escapes() {}";
        assert_eq!(declared_test_names(escapes), vec!["after_escapes".to_string()]);
    }

    /// An UNTERMINATED string yields nothing at all, rather than a truncated declared set.
    ///
    /// Running to EOF inside a string means either the file is not valid Rust or the scanner
    /// mis-identified an opener. Either way what remains is an under-report, so it refuses
    /// wholesale: an empty declared set is never a pass.
    /// RED IF: an unterminated literal starts yielding a partial list.
    #[test]
    fn bv_48_an_unterminated_string_yields_nothing() {
        let src = "#[test] fn first() {}\nlet s = \"never closed\n#[test] fn second() {}";
        assert!(
            declared_test_names(src).is_empty(),
            "a partial list here would silently drop whatever came after the bad literal"
        );
        let run = parse_cargo_test_output("test blind::first ... ok\n");
        assert!(
            !declared_tests_all_passed(&declared_test_names(src), &run),
            "and an empty declared set must never be a pass"
        );
    }

    /// Grok, round 6: the partial run WITH a build-failure matcher had no test.
    ///
    /// `bv_43` pins a partial run with no matcher, which blocks. The mirror is a partial run
    /// whose output DOES contain something like `error[E`, which does not block. That is the
    /// intended new-API answer when a second test binary failed to build, and it is also what a
    /// test that merely PRINTS compiler output would produce. Pinned so the trade is on the
    /// record rather than emergent.
    /// RED IF: a partial run with a matcher starts blocking, which would reject honest new-API
    /// work, or the matcher stops being consulted at all.
    #[test]
    fn bv_49_a_partial_run_with_a_build_failure_is_the_new_api_case() {
        let declared = vec!["ran".to_string(), "never_built".to_string()];
        let out = "test other::ran ... FAILED\n                   error[E0425]: cannot find function `duration_secs` in this scope\n";
        let proof = classify_baseline_proof(&declared, out, &parse_cargo_test_output(out));
        assert_eq!(proof, BaselineProof::InconclusiveNoApi);
        assert!(!proof.blocks(), "an honest new-API suite must not be rejected");

        // And the guard rail that makes the trade safe: the substring can only choose between
        // the two inconclusive answers. It can never promote a run to Proven or hide a Refuted.
        let green = "test blind::ran ... ok\nerror[E0425]: something\n";
        assert!(
            matches!(
                classify_baseline_proof(&["ran".to_string()], green, &parse_cargo_test_output(green)),
                BaselineProof::Refuted { .. }
            ),
            "a green name is Refuted even when the output also mentions a compiler error"
        );
    }

    /// Antigravity, round 6: the word boundary in `find_fn_keyword` had no test.
    ///
    /// A bare search for `fn ` also matches inside an identifier such as `my_fn foo`, which
    /// would declare a test named `foo` that does not exist. Over-reporting fails closed, but a
    /// phantom name still rejects an honest suite, and the comment claimed a guard that nothing
    /// exercised.
    /// RED IF: the boundary check is dropped.
    #[test]
    fn bv_50_fn_inside_an_identifier_is_not_a_declaration() {
        // `my_fn` ends in `fn `, and `helper` follows it.
        let src = "#[test]\nlet x = my_fn helper;\nfn real() {}";
        assert_eq!(
            declared_test_names(src),
            vec!["real".to_string()],
            "`my_fn helper` must not declare `helper`"
        );
    }

    /// GROK ROUND 5, FIND-BLIND-05. Absence is not redness.
    ///
    /// The case this feature will actually see most: the contract names a function that does not
    /// exist yet, so the blind tests do not COMPILE against the old tree, cargo emits no per-test
    /// lines, and every declared name is absent. The first version read that silence as redness,
    /// so `assert!(f(x).is_some())` was "proven" by a build failure while asserting nothing.
    ///
    /// It is now reported honestly as inconclusive. It does NOT block, because blocking it would
    /// reject every honest dispatch that adds a new API.
    /// RED IF: a build failure is counted as proof again, or starts blocking honest work.
    #[test]
    fn bv_40_a_build_failure_is_inconclusive_not_proof() {
        let declared = vec!["checks_five_minutes".to_string()];
        let out = "error[E0425]: cannot find function `duration_secs` in this scope\n                   error: could not compile `thing` (test) due to 1 previous error\n";
        let proof = classify_baseline_proof(&declared, out, &parse_cargo_test_output(out));
        assert_eq!(proof, BaselineProof::InconclusiveNoApi);
        assert!(
            !proof.blocks(),
            "an honest new-API suite must not be rejected: the old tree could not build it, \
             which is expected"
        );
    }

    /// The other half of the same finding, and the reason it is not simply "silence is fine".
    ///
    /// Silence with NO sign of a build failure means the caller probably never ran the blind
    /// tests at all, which Grok noted is easy to get wrong because this module copies them only
    /// into the post-change tree. That is indistinguishable from a check that never happened,
    /// so it blocks.
    /// RED IF: unexplained silence stops blocking.
    #[test]
    fn bv_41_silence_without_a_build_failure_blocks() {
        let declared = vec!["checks_five_minutes".to_string()];
        for out in ["", "running 0 tests\n\ntest result: ok. 0 passed\n", "some unrelated log"] {
            let proof = classify_baseline_proof(&declared, out, &parse_cargo_test_output(out));
            assert_eq!(
                proof,
                BaselineProof::InconclusiveNoEvidence,
                "output {out:?} establishes nothing and must not be read as proof"
            );
            assert!(proof.blocks());
        }
    }

    /// The strong case still works: every declared test RAN and failed. Red then green.
    /// RED IF: a genuinely proven suite stops being recognised.
    #[test]
    fn bv_42_a_test_that_ran_and_failed_is_proven() {
        let declared = vec!["a".to_string(), "b".to_string()];
        let out = "test blind::a ... FAILED\ntest blind::b ... FAILED\n";
        let proof = classify_baseline_proof(&declared, out, &parse_cargo_test_output(out));
        assert_eq!(proof, BaselineProof::Proven);
        assert!(!proof.blocks());
    }

    /// A PARTIAL run is not proof. Some declared tests ran and failed, others never appeared,
    /// and there is no build failure to explain the absence. That mixture is exactly the
    /// "one strong test plus nine that did not compile" arrangement Grok named.
    /// RED IF: a partial run is promoted to Proven.
    #[test]
    fn bv_43_a_partial_run_is_not_proof() {
        let declared = vec!["strong".to_string(), "weak".to_string()];
        let out = "test blind::strong ... FAILED\n";
        let proof = classify_baseline_proof(&declared, out, &parse_cargo_test_output(out));
        assert_eq!(
            proof,
            BaselineProof::InconclusiveNoEvidence,
            "strong ran and failed, weak never appeared, and nothing explains why"
        );
    }

    /// A green name refutes the whole suite even when the rest genuinely failed. One tautology
    /// among nine real tests is still a tautology, and the report names WHICH.
    /// RED IF: refutation weakens to "all names were green".
    #[test]
    fn bv_44_one_green_name_refutes_the_suite_and_is_named() {
        let declared = vec!["real".to_string(), "tautology".to_string()];
        let out = "test blind::real ... FAILED\ntest blind::tautology ... ok\n";
        match classify_baseline_proof(&declared, out, &parse_cargo_test_output(out)) {
            BaselineProof::Refuted { already_green } => {
                assert_eq!(already_green, vec!["tautology".to_string()]);
            }
            other => panic!("expected Refuted, got {other:?}"),
        }
    }

    /// CODEX ROUND 5, FINDING 1. A name collision must FAIL CLOSED.
    ///
    /// Codex's attack: the validator's own `blind::x` FAILED, the worker added
    /// `worker_added::x` which passed, and suffix matching let the worker's test satisfy the
    /// validator's declared name. The self-validation family, re-entered through a collision
    /// rather than through the baseline diff I had just closed.
    ///
    /// When two tests answer to one name, nothing here can tell which is the validator's, so
    /// every one of them must pass.
    /// RED IF: the quantifier goes back to `any`.
    #[test]
    fn bv_35_a_colliding_name_cannot_be_satisfied_by_someone_elses_test() {
        let run = parse_cargo_test_output(
            "test blind::x ... FAILED\ntest worker_added::x ... ok\n",
        );
        assert!(
            !declared_tests_all_passed(&["x".to_string()], &run),
            "the validator's own x failed; a worker-added x of the same name must not cover it"
        );

        // And the proof must be equally unwilling: one green candidate at baseline is enough to
        // mean the name was not proven.
        let out = "test blind::x ... FAILED\ntest worker_added::x ... ok\n";
        let proof = classify_baseline_proof(&["x".to_string()], out, &parse_cargo_test_output(out));
        assert!(proof.blocks(), "a green candidate at baseline means this name proves nothing");
    }

    /// The control. A single, uncontested name still passes, and the `::` anchor still holds so
    /// `not_x` cannot satisfy `x`.
    /// RED IF: the anchoring breaks and every honest run starts failing, or a prefix match
    /// starts satisfying names it should not.
    #[test]
    fn bv_36_an_uncontested_name_still_passes_and_stays_anchored() {
        let run = parse_cargo_test_output("test blind::x ... ok\ntest blind::not_x ... FAILED\n");
        assert!(declared_tests_all_passed(&["x".to_string()], &run));
        assert!(
            !declared_tests_all_passed(&["y".to_string()], &run),
            "a name with no candidate at all is not a pass"
        );
    }

    /// CODEX ROUND 5, FINDING 2. A comment is not a test declaration.
    ///
    /// `// #[test] fn x() {}` declared `x`. On its own a false report; chained with the
    /// collision hole above, a validator that wrote nothing but a comment could have its
    /// "test" satisfied by one the worker added. Two independent defects composing into a
    /// false accept.
    /// RED IF: the comment and string stripping is removed.
    #[test]
    fn bv_37_comments_and_strings_do_not_declare_tests() {
        assert!(
            declared_test_names("// #[test] fn x() {}").is_empty(),
            "a line comment declares nothing"
        );
        assert!(
            declared_test_names("/* #[test] fn x() {} */").is_empty(),
            "a block comment declares nothing"
        );
        assert!(
            declared_test_names("/* outer /* nested #[test] fn x() {} */ still */").is_empty(),
            "rust block comments nest"
        );
        assert!(
            declared_test_names(r##"let s = "#[test] fn x() {}";"##).is_empty(),
            "a string literal declares nothing"
        );
        assert!(
            declared_test_names(r####"let s = r#"#[test] fn x() {}"#;"####).is_empty(),
            "a raw string declares nothing"
        );
    }

    /// The control for the stripper: real declarations survive it, including the shapes a
    /// validator actually writes.
    /// RED IF: stripping becomes over-eager and real tests stop being seen, which would make
    /// every honest validator look like it wrote nothing.
    #[test]
    fn bv_38_real_declarations_survive_the_stripper() {
        let src = r###"
            //! A module doc mentioning #[test] in prose.
            use std::fmt;

            /// Doc comment for the first test.
            #[test]
            fn checks_five_minutes() {
                assert_eq!(duration_secs("5m"), Some(300));
            }

            #[tokio::test]
            async fn checks_async() {
                let msg = "the string mentions #[test] fn decoy() {}";
                let _ = msg;
            }

            #[test]
            #[ignore = "slow"]
            fn checks_two_hours() {}
        "###;
        let names = declared_test_names(src);
        assert_eq!(
            names,
            vec![
                "checks_async".to_string(),
                "checks_five_minutes".to_string(),
                "checks_two_hours".to_string(),
            ],
            "got {names:?}"
        );
    }

    /// `#[cfg(test)]` does NOT arm the scanner, and the reason is one character.
    ///
    /// I predicted this was a bug before checking: the attribute looked like it contained the
    /// literal the scanner searches for, so a `#[cfg(test)] mod tests` would arm on it and then
    /// claim whatever function followed. It does not. The attribute is `test)]`, with a closing
    /// parenthesis between `test` and `]`, so the search for `test]` never matches it.
    ///
    /// Pinned because the near-miss is worth a test whichever way it went, and because a future
    /// change to the search literal (say, to `test`) would silently start claiming helpers
    /// inside every `#[cfg(test)]` module.
    /// RED IF: the attribute match widens and starts arming on cfg attributes.
    #[test]
    fn bv_39_a_cfg_test_module_declares_nothing() {
        let src = "#[cfg(test)]\nmod tests {\n    fn helper() {}\n}";
        assert!(
            declared_test_names(src).is_empty(),
            "`#[cfg(test)]` is `test)]`, not `test]`, so it must not arm the scanner"
        );

        // The real thing inside such a module still counts, so the rule above is not achieved by
        // ignoring test modules wholesale.
        let with_a_real_test = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn real() {}\n}";
        assert_eq!(declared_test_names(with_a_real_test), vec!["real".to_string()]);
    }

    /// GROK ROUND 4, FIND-BLIND-04. A tautology is caught by red/green, not by blindness.
    ///
    /// Grok's finding: the module claimed a tautology was "impossible" because the validator
    /// cannot see the code. Blindness stops a tautology built FROM the implementation. It does
    /// nothing about a WEAK test. `assert!(true)` passes against any body, and the prompt merely
    /// asked for better.
    ///
    /// The check is that a test proving a change works should FAIL before the change. That is
    /// decisive HERE, for a tautology that compiles against the old tree and comes back green.
    /// It is not a general answer to weak tests, which is what the module doc used to claim and
    /// what `bv_40` covers: a weak test against a NEW API never compiles at baseline, so it is
    /// never Refuted.
    /// RED IF: the red-at-baseline requirement is dropped from acceptance.
    #[test]
    fn bv_30_a_tautology_passes_the_tests_and_fails_the_proof() {
        let dir = scratch();
        let req = request(dir.path());
        let h = Harness {
            writes: "#[test] fn blind_a() { assert!(true); }".to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... ok\ntest blind::blind_a ... ok\n".to_string(),
            // The giveaway: it was ALREADY green before the worker changed anything.
            baseline_output: Some("test blind::blind_a ... ok\n".to_string()),
        };
        let report = run(&h, &req).unwrap();
        assert!(report.blind_tests_passed, "the tautology does pass, which is the problem");
        assert!(matches!(report.baseline_proof, BaselineProof::Refuted { .. }));
        assert!(!report.accepted(), "a test that was already green proves nothing");
        assert!(report.why_rejected().unwrap().contains("ALREADY GREEN"));
    }

    /// A caller that cannot build a pre-change tree gets a REFUSAL, not a pass. An unproven
    /// suite is indistinguishable from a tautological one.
    /// RED IF: a missing baseline run is treated as satisfied.
    #[test]
    fn bv_31_an_unprovable_suite_is_not_accepted() {
        let dir = scratch();
        let req = request(dir.path());
        let h = Harness {
            writes: "#[test] fn blind_a() { assert_eq!(1, 1); }".to_string(),
            tool_calls: vec![],
            test_output: "test existing::a ... ok\ntest blind::blind_a ... ok\n".to_string(),
            baseline_output: None,
        };
        let report = run(&h, &req).unwrap();
        assert!(!report.accepted());
        assert_eq!(report.baseline_proof, BaselineProof::InconclusiveNoEvidence);
    }

    /// EVERY declared test must have been red, not merely one. A validator writing nine
    /// tautologies and one real test would otherwise pass, and the nine are what this is for.
    /// RED IF: the check weakens to `any`.
    #[test]
    fn bv_32_one_real_test_does_not_excuse_the_tautologies() {
        let declared = vec!["real_one".to_string(), "tautology".to_string()];
        let out = "test blind::tautology ... ok\ntest blind::real_one ... FAILED\n";
        let proof = classify_baseline_proof(&declared, out, &parse_cargo_test_output(out));
        assert_eq!(
            proof,
            BaselineProof::Refuted { already_green: vec!["tautology".to_string()] },
            "one green name refutes the suite even though real_one was genuinely red"
        );
        assert!(proof.blocks());
    }

    /// GROK ROUND 4, FIND-BLIND-02. Deleting a FAILING test is not a silent pass.
    ///
    /// `newly_failing` filters the baseline to tests that were PASSING, so a red test that
    /// vanished was invisible. The doc named that exact cheat and then tested the other one:
    /// `bv_03` deletes a test that was green.
    /// RED IF: deletion of a previously-failing test stops being reported.
    #[test]
    fn bv_33_deleting_a_failing_test_is_reported() {
        let dir = scratch();
        let mut req = request(dir.path());
        req.baseline = parse_cargo_test_output("test existing::a ... ok\ntest existing::red ... FAILED\n");
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            // existing::red is simply gone. It was never green, so newly_failing cannot see it.
            test_output: "test existing::a ... ok\ntest blind::blind_a ... ok\n".to_string(),
            baseline_output: red_first(&["blind_a"]),
        };
        let report = run(&h, &req).unwrap();
        assert!(
            report.newly_failing.is_empty(),
            "it was never green, so it is correctly not in newly_failing"
        );
        assert_eq!(
            report.deleted,
            vec!["existing::red".to_string()],
            "but its DISAPPEARANCE must be reported, whatever state it was in"
        );
        assert!(!report.accepted());
        let why = report.why_rejected().unwrap();
        assert!(why.contains("did not appear in this run"), "got: {why}");
        assert!(
            why.contains("did not enumerate the same tests"),
            "the message must name the OTHER explanation too, or it blames an honest worker for \
             a harness mismatch; got: {why}"
        );
    }

    /// GROK ROUND 4, FIND-BLIND-03. The validator's directory must start EMPTY.
    ///
    /// "Contains the contract and nothing else" was a comment, not a fact: the orchestration
    /// never emptied the directory, so a pre-populated one kept whatever was there.
    /// RED IF: a non-empty validator directory is accepted.
    #[test]
    fn bv_34_a_prepopulated_validator_directory_is_refused() {
        let dir = scratch();
        let req = request(dir.path());
        std::fs::create_dir_all(&req.validator_dir).unwrap();
        std::fs::write(req.validator_dir.join("leftover.rs"), "the previous run").unwrap();
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            test_output: "test blind::blind_a ... ok\n".to_string(),
            baseline_output: red_first(&["blind_a"]),
        };
        let err = run(&h, &req).unwrap_err();
        assert!(err.contains("not empty"), "got: {err}");
        assert!(
            err.contains("Refusing rather than deleting"),
            "silently removing a caller's files is worse than declining; got: {err}"
        );
    }

    /// CODEX ROUND 4, FINDING 1. Containment is symmetric.
    ///
    /// Rejecting only "validator inside implementation" left the inverse open: an
    /// implementation INSIDE the validator's directory means the validator's allowed root
    /// contains the code, and every read of it passes the blindness check as legitimately
    /// inside.
    /// RED IF: the check goes back to testing one direction.
    #[test]
    fn bv_29_the_implementation_may_not_live_inside_the_validator() {
        let dir = scratch();
        let mut req = request(dir.path());
        req.validator_dir = dir.path().join("job");
        req.impl_worktree = dir.path().join("job").join("impl");
        std::fs::create_dir_all(&req.impl_worktree).unwrap();
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            test_output: "test blind_a ... ok\n".to_string(),
            baseline_output: red_first(&["blind_a", "checks_five_minutes", "checks_two_hours", "checks_async"]),
        };
        let err = run(&h, &req).unwrap_err();
        assert!(err.contains("overlap"), "got: {err}");
    }

    /// The containment check on the ARGUMENTS themselves. A caller that points the validator at
    /// the implementation worktree would otherwise get a confident, meaningless PASS.
    /// RED IF: the two directories are allowed to overlap.
    #[test]
    fn bv_25_the_validator_may_not_live_inside_the_implementation() {
        let dir = scratch();
        let mut req = request(dir.path());
        req.validator_dir = req.impl_worktree.join("validator");
        let h = Harness {
            writes: "#[test] fn blind_a() {}".to_string(),
            tool_calls: vec![],
            test_output: "test blind_a ... ok\n".to_string(),
            baseline_output: red_first(&["blind_a", "checks_five_minutes", "checks_two_hours", "checks_async"]),
        };
        let err = run(&h, &req).unwrap_err();
        assert!(err.contains("overlap"), "got: {err}");
    }
}
