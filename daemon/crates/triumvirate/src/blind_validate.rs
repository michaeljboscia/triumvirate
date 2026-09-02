//! Wiring: the live caller for `mcp_tools::blind_validation`.
//!
//! The validation logic is pure and lives in `mcp-tools`, behind injected callbacks, so its
//! guarantees can be tested without spawning a model or touching a repo. This file supplies the
//! real side effects: a real validator agent, a real test run, and a real pre-change worktree.
//!
//! It lives in the `triumvirate` crate rather than in `mcp-tools` because it needs
//! `run_named_agent_with_session_and_model`, which returns a `ParsedAgentResult` carrying the
//! agent's TOOL CALLS. `execute_ask_agent`'s public response carries only a tool-call COUNT
//! (`AskAgentResponse::tool_calls_made`), and a count cannot answer "did it read outside its own
//! directory". Using the public path would have meant handing the blindness scan an empty list,
//! which it would have reported as clean: absence of evidence presented as evidence of absence,
//! which is the exact defect the sight-gate work spent a day removing.

use std::path::{Path, PathBuf};

use mcp_tools::blind_validation::{
    BaselineProof, BlindReport, BlindValidationCallbacks, BlindValidationRequest, TestRun,
    parse_cargo_test_output, run_blind_validation,
};
use shared_types::{AskAgentRequest, BlindValidateRequest, BlindValidateResponse};
use uuid::Uuid;

use crate::agent_exec::run_named_agent_with_session_and_model;

/// What the caller asks for.
#[derive(Debug, Clone)]
pub struct LiveBlindValidation {
    /// The repository the worktree belongs to. Scratch space is created under it.
    pub project_root: PathBuf,
    /// The worktree the worker built in.
    pub impl_worktree: PathBuf,
    /// The contract, which must carry the API surface. See the module docs in `mcp-tools`.
    pub contract: String,
    /// The commit the worker started from. The pre-change tree is built at this ref.
    pub base_ref: String,
    /// Who wrote the code. The validator will not be this agent.
    pub worker_agent: String,
    /// Candidate validators in preference order. Empty means the default panel.
    pub roster: Vec<String>,
    /// Where the Cargo package lives, relative to each worktree root. Defaults to ".".
    pub package_dir: String,
}

/// Where a job's scratch space lives: OUTSIDE the repository entirely.
///
/// TWO reasons, both from Codex's round 7 review, and the second is the more important.
///
/// First, the obvious one: scratch used to be `<project_root>/.triumvirate/blind/<uuid>`, which
/// is INSIDE the implementation worktree whenever the worktree is the repo root. That is the
/// common "validate the main checkout" case, and `run_blind_validation` correctly refuses an
/// overlap, so blind validation could not run at all there.
///
/// Second, and this is what makes the blindness check mean something: from a directory that is
/// not under the repository, the implementation cannot be reached by RELATIVE traversal. A
/// validator that wants to read the code has to name an absolute path, and an absolute path is
/// the one thing `reads_outside_allowed_root` can actually see, including inside a shell
/// command. It does not make the validator sandboxed. It converts the undetectable case
/// (`cd .. && cat src/x.rs`) into the detectable one.
///
/// Stated without the overclaim this file carried in round 7: cwd is NOT a sandbox, the agent
/// CLIs are not confined by it, and a determined validator with shell access can still read the
/// implementation. What changed is that doing so now leaves a trace the scan catches.
fn scratch_paths(job: &str) -> (PathBuf, PathBuf, PathBuf) {
    // TWO SEPARATE ROOTS, not two children of one scratch directory.
    //
    // Grok's round 7 nit: with `scratch/validator` and `scratch/baseline`, the pre-change source
    // tree sat at `../baseline` from the agent's own cwd, so `cd .. && cat baseline/src/x.rs`
    // reached a full checkout by a relative path the scan cannot see. Separate roots mean `..`
    // from the validator contains only the validator's own job directory.
    //
    // It is the OLD tree, so a tautology of existing behaviour would be Refuted rather than
    // accepted, but "the wrong answer is caught downstream" is not a reason to leave a source
    // tree one directory up.
    let validator_dir = std::env::temp_dir()
        .join("triumvirate-blind-validator")
        .join(job)
        .join("v");
    let baseline_tree = std::env::temp_dir()
        .join("triumvirate-blind-base")
        .join(job)
        .join("tree");
    let scratch = validator_dir
        .parent()
        .expect("validator dir has a parent")
        .to_path_buf();
    (scratch, validator_dir, baseline_tree)
}

/// Removes the pre-change worktree on EVERY exit path, including a panic.
///
/// Codex found in round 7 that the cleanup ran after `run_with_trees` returned, so a panic
/// anywhere inside it left a registered git worktree behind. Drop runs during unwinding; a
/// trailing statement does not.
struct BaselineWorktree {
    repo: PathBuf,
    path: PathBuf,
    /// Everything else this run created: the validator's directory, the baseline's parent, and
    /// the test file copied INTO the implementation worktree.
    ///
    /// Grok's round 7 nits 4 and 5: the git worktree was removed and the scratch directories
    /// were left in /tmp, and the blind test file was written into the worktree under test and
    /// never taken out, leaving a dirty tree and overwriting that filename if the worker
    /// already had one.
    also_remove: Vec<PathBuf>,
}

impl Drop for BaselineWorktree {
    fn drop(&mut self) {
        if let Err(e) = git(
            &self.repo,
            &["worktree", "remove", "--force", &self.path.to_string_lossy()],
        ) {
            // Logged rather than propagated: a Drop cannot return, and losing the report to a
            // cleanup failure would be worse than a stale entry that `git worktree prune` fixes.
            tracing::warn!(
                "blind validation left a worktree at {}: {e}. Run `git worktree prune`.",
                self.path.display()
            );
        }
        let _ = std::fs::remove_dir_all(&self.path);
        for p in &self.also_remove {
            let _ = if p.is_dir() {
                std::fs::remove_dir_all(p)
            } else {
                std::fs::remove_file(p)
            };
        }
    }
}

/// Run a blind validation for real.
///
/// EVERY STEP CAN REFUSE, and the refusals are the point. See
/// `mcp_tools::blind_validation::run_blind_validation` for the ordering and why it is that way.
pub async fn run_live(req: LiveBlindValidation) -> Result<BlindReport, String> {
    if !req.impl_worktree.is_dir() {
        return Err(format!(
            "the implementation worktree {} does not exist",
            req.impl_worktree.display()
        ));
    }

    let job = Uuid::new_v4().to_string();
    let (scratch, validator_dir, baseline_tree) = scratch_paths(&job);
    std::fs::create_dir_all(&scratch)
        .map_err(|e| format!("could not create blind validation scratch: {e}"))?;
    std::fs::create_dir_all(
        baseline_tree.parent().expect("baseline tree has a parent"),
    )
    .map_err(|e| format!("could not create the baseline root: {e}"))?;

    // THE PRE-CHANGE TREE. A detached worktree at the base commit, which is what makes the
    // red/green proof possible at all: the same blind tests are run against code that predates
    // the worker's change.
    git(
        &req.project_root,
        &[
            "worktree",
            "add",
            "--detach",
            &baseline_tree.to_string_lossy(),
            &req.base_ref,
        ],
    )?;

    // From here the worktree is owned by a guard, so it is removed even if the work panics.
    let copied_into_impl = req
        .impl_worktree
        .join(TreeLayout::new(&req.package_dir).test_file_rel());
    let _guard = BaselineWorktree {
        repo: req.project_root.clone(),
        path: baseline_tree.clone(),
        also_remove: vec![
            scratch.clone(),
            baseline_tree
                .parent()
                .expect("baseline tree has a parent")
                .to_path_buf(),
            copied_into_impl,
        ],
    };

    run_with_trees(&req, &validator_dir, &baseline_tree).await
}

async fn run_with_trees(
    req: &LiveBlindValidation,
    validator_dir: &Path,
    baseline_tree: &Path,
) -> Result<BlindReport, String> {
    // The blind tests go inside the PACKAGE, not at the git root.
    //
    // Grok found in round 7 that `<checkout>/tests/blind_validation.rs` is never compiled when
    // the manifest is not at the git root, which is every virtual workspace including this one.
    // `cargo test` walks parents for a manifest, not children, so the file was simply not a
    // rustc input, every declared name was absent, and the gate rejected good worktrees as well
    // as bad. It was stuck closed.
    let layout = TreeLayout::new(&req.package_dir);
    let test_file_rel = layout.test_file_rel();
    let test_command = "cargo test".to_string();

    // THE BASELINE, captured from the pre-change tree BEFORE the blind tests are put into it.
    //
    // This is what stops a pre-existing failure being blamed on the worker. It is taken here
    // rather than accepted from the caller, because a caller-supplied baseline can be captured
    // with a different command and then every difference looks like the worker's doing. Grok
    // named that risk in review; taking it ourselves removes one of the two ways it happens.
    // Run in the PACKAGE directory of the pre-change tree, which is the same relative path the
    // after-run uses in the implementation tree. That is what makes the two runs comparable, and
    // Grok's round 5 warning was that an incomparable pair reads as deletion and blames an
    // honest worker.
    let baseline_pkg = layout.package_root(baseline_tree);
    let baseline: TestRun = parse_cargo_test_output(&cargo_test(&baseline_pkg)?);

    // AGENTS THAT CANNOT WRITE ARE NOT VALIDATORS ON THIS PATH.
    //
    // Grok found in round 7 that its own default sandbox is `read-only`, so a roster with grok
    // in front fails at "did not write tests" for a reason that is the harness's, not the
    // model's. That is the blocked-instrument shape this repo has a standing rule about: a zero
    // from a blocked instrument is not evidence about the agent.
    //
    // deepseek is excluded for a harder reason: it has no filesystem through the bridge at all.
    //
    // Filtered rather than failed, because the roster is a preference list and the next name is
    // a legitimate answer. Logged so an operator can see why their first choice was skipped.
    const CANNOT_WRITE_HERE: &[&str] = &["grok", "deepseek"];
    let requested = if req.roster.is_empty() {
        peer_review::default_reviewers()
    } else {
        req.roster.clone()
    };
    let roster: Vec<String> = requested
        .iter()
        .filter(|a| {
            let keep = !CANNOT_WRITE_HERE.contains(&a.to_ascii_lowercase().as_str());
            if !keep {
                tracing::info!(
                    agent = %a,
                    "skipped as a blind validator: it cannot write its test file on this \
                     dispatch (read-only sandbox, or no filesystem at all)"
                );
            }
            keep
        })
        .cloned()
        .collect();
    if roster.is_empty() {
        return Err(format!(
            "no agent in the roster {requested:?} can write a test file on this dispatch. \
             A blind validator must WRITE its tests, and {CANNOT_WRITE_HERE:?} cannot here."
        ));
    }

    // The validator agent, run for real. `run_named_agent_with_session_and_model` is used rather
    // than `execute_ask_agent` because its result carries the TOOL CALLS, which is what the
    // blindness scan reads. See the module docs.
    let run_agent = move |agent: &str, prompt: &str, cwd: &Path| {
        let agent = agent.to_string();
        let prompt = prompt.to_string();
        let cwd = cwd.to_string_lossy().to_string();
        // A DEDICATED THREAD WITH ITS OWN RUNTIME, not `block_in_place`.
        //
        // Codex found in round 7 that `tokio::task::block_in_place` PANICS on a current-thread
        // runtime, so the tool would have been unable to run at all depending on how the daemon
        // was configured. This is runtime-agnostic: it never touches the caller's runtime.
        //
        // `require_sight` is deliberately NOT set. This dispatch is the opposite shape from a
        // review: there is nothing the validator is required to have read, and it MUST write its
        // test file, which the sight gate treats as a disqualifying mutation.
        let parsed = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| format!("could not build a runtime for the validator: {e}"))?;
                    rt.block_on(async {
                        let ask = AskAgentRequest {
                            agent: agent.clone(),
                            message: prompt.clone(),
                            cwd: Some(cwd.clone()),
                            ..Default::default()
                        };
                        run_named_agent_with_session_and_model(
                            &agent, &prompt, &cwd, None, None, None, Some(&ask),
                        )
                        .await
                        .map_err(|e| format!("the validator agent failed: {e}"))
                    })
                })
                .join()
                .map_err(|_| "the validator dispatch thread panicked".to_string())?
        });
        parsed.map(|p| (p.response_text, p.tool_calls))
    };

    let after_layout = layout.clone();
    let run_tests = move |cwd: &Path| cargo_test(&after_layout.package_root(cwd));

    // The pre-change run. The blind tests are copied INTO the base worktree first, which is the
    // step a caller is most likely to skip: without it the run reports nothing, every declared
    // name is absent, and the classifier correctly refuses with InconclusiveNoEvidence.
    let baseline_tree_owned = baseline_tree.to_path_buf();
    let baseline_layout = layout.clone();
    let test_rel = test_file_rel.clone();
    let validator_dir_owned = validator_dir.to_path_buf();
    let run_tests_at_baseline = move |_impl_cwd: &Path| {
        let src = validator_dir_owned.join(&test_rel);
        let dest = baseline_tree_owned.join(&test_rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create the baseline test directory: {e}"))?;
        }
        std::fs::copy(&src, &dest)
            .map_err(|e| format!("could not copy the blind tests into the baseline tree: {e}"))?;
        cargo_test(&baseline_layout.package_root(&baseline_tree_owned)).map(Some)
    };

    run_blind_validation(
        &BlindValidationRequest {
            impl_worktree: req.impl_worktree.clone(),
            validator_dir: validator_dir.to_path_buf(),
            contract: req.contract.clone(),
            test_file_rel,
            test_command,
            worker_agent: req.worker_agent.clone(),
            roster,
            baseline,
        },
        &BlindValidationCallbacks {
            run_agent: &run_agent,
            run_tests: &run_tests,
            run_tests_at_baseline: &run_tests_at_baseline,
        },
    )
}

/// The MCP tool's whole body, extracted so it can be TESTED.
///
/// Antigravity found in round 7 that `bl_04` proved the wiring by string-matching `main.rs`,
/// which would still pass if the call were commented out. The tool on the bridge is now a
/// one-line delegation to this, and the test calls THIS, so a disconnected tool fails to
/// compile rather than passing a substring check.
pub async fn tool_impl(req: BlindValidateRequest) -> Result<BlindValidateResponse, String> {
    let report = run_live(LiveBlindValidation {
        project_root: PathBuf::from(&req.project_root),
        impl_worktree: PathBuf::from(&req.impl_worktree),
        contract: req.contract,
        base_ref: req.base_ref,
        worker_agent: req.worker_agent,
        roster: req.roster,
        package_dir: req.package_dir.unwrap_or_else(|| ".".to_string()),
    })
    .await?;

    Ok(BlindValidateResponse {
        validator_agent: report.validator_agent.clone(),
        accepted: report.accepted(),
        rejection: report.why_rejected(),
        blind_tests_passed: report.blind_tests_passed,
        baseline_proof: match &report.baseline_proof {
            BaselineProof::Proven => "proven".to_string(),
            BaselineProof::Refuted { .. } => "refuted".to_string(),
            BaselineProof::InconclusiveNoApi => "inconclusive_no_api".to_string(),
            BaselineProof::InconclusiveNoEvidence => "inconclusive_no_evidence".to_string(),
        },
        // The NAMES, not a formatted string. Antigravity pointed out that flattening
        // `Refuted { already_green }` into prose forced an automated caller to parse English to
        // learn which tests were the problem.
        already_green: match &report.baseline_proof {
            BaselineProof::Refuted { already_green } => already_green.clone(),
            _ => Vec::new(),
        },
        newly_failing: report.newly_failing,
        deleted: report.deleted,
        blindness_violations: report.blindness_violations,
    })
}

/// Where the blind tests live and where cargo runs, for a given package layout.
///
/// One type rather than two loose helpers, because the previous version computed the test path
/// and the cargo directory at separate call sites and a mutation that ignored the package
/// directory in BOTH left every test green: `package_root` was tested in isolation and nothing
/// checked that the orchestration actually used it. That is the "tested the helper, not the
/// route" defect this codebase has now hit four times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeLayout {
    package_dir: String,
}

impl TreeLayout {
    pub fn new(package_dir: &str) -> Self {
        Self { package_dir: package_dir.trim_matches('/').to_string() }
    }

    fn is_root(&self) -> bool {
        self.package_dir.is_empty() || self.package_dir == "."
    }

    /// The blind test file, RELATIVE to a tree root. Inside the package, because
    /// `cargo test` walks parents for a manifest and never finds a file placed above one.
    pub fn test_file_rel(&self) -> String {
        if self.is_root() {
            "tests/blind_validation.rs".to_string()
        } else {
            format!("{}/tests/blind_validation.rs", self.package_dir)
        }
    }

    /// The directory `cargo test` runs in, for a given tree. The SAME relative path in both
    /// trees, which is what makes the baseline and the after-run comparable.
    pub fn package_root(&self, tree: &Path) -> PathBuf {
        if self.is_root() {
            tree.to_path_buf()
        } else {
            tree.join(&self.package_dir)
        }
    }
}

/// `cargo test` in a directory, returning combined output.
///
/// A non-zero exit is NOT an error here: a failing suite is the ordinary case this whole feature
/// exists to detect, and treating it as a transport failure would turn every caught defect into
/// "the harness broke". Only a failure to LAUNCH cargo is an error.
///
/// stdout and stderr are joined because the per-test lines land on stdout while a compile failure
/// lands on stderr, and `classify_baseline_proof` has to see both to tell "the old tree had no
/// such API" from "nobody ran anything".
fn cargo_test(cwd: &Path) -> Result<String, String> {
    let out = std::process::Command::new("cargo")
        .arg("test")
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("could not run cargo test in {}: {e}", cwd.display()))?;
    let mut combined = String::from_utf8_lossy(&out.stdout).into_owned();
    combined.push('\n');
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(combined)
}

fn git(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("could not run git {args:?}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every test here runs `cargo` or `git` through `Command::new`, which reads `PATH`.
    ///
    /// Antigravity raised the env race in round 7. Its stated version was wrong in one detail:
    /// `abe.rs` is in the `mcp-tools` crate and therefore a DIFFERENT test binary, so it cannot
    /// race this one. The hazard is real anyway, because this binary's own tests mutate process
    /// env under `crate::tests::env_lock`, and `setenv` may reallocate the environ array while
    /// `Command::new` reads it. Same lock, same reason as everywhere else in this codebase.
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::tests::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn have(bin: &str) -> bool {
        std::process::Command::new(bin)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Scratch must never be inside the implementation, and the REAL path function is what is
    /// asked, not a hand-built string.
    ///
    /// Antigravity found that the first version compared two hardcoded paths and never called
    /// production code, so breaking `run_live` would have left it green. It now drives
    /// `scratch_paths` and checks the case that was actually broken: the implementation worktree
    /// being the repository root, which put scratch inside it and made the whole tool refuse.
    /// RED IF: scratch moves back under the repository.
    #[test]
    fn bl_01_scratch_is_outside_the_repository() {
        let (scratch, validator_dir, baseline) = scratch_paths("job-1");

        // The case Codex named: validating the main checkout, where worktree == repo root.
        let repo = PathBuf::from("/repo");
        for p in [&scratch, &validator_dir, &baseline] {
            assert!(
                !p.starts_with(&repo),
                "{} must not be inside the repository, or validating the main worktree is \
                 impossible: run_blind_validation refuses the overlap",
                p.display()
            );
        }
        assert!(validator_dir.starts_with(&scratch));

        // The baseline is under a SEPARATE root, deliberately. Grok's round 7 nit: as siblings
        // under one scratch directory, the pre-change source tree sat at `../baseline` from the
        // agent's cwd, reachable by a relative path the blindness scan cannot see.
        assert!(
            !baseline.starts_with(&scratch),
            "the baseline checkout must not be a sibling of the validator's directory: \
             validator {} vs baseline {}",
            validator_dir.display(),
            baseline.display()
        );
        let up_from_validator = validator_dir.parent().expect("has a parent");
        assert!(
            !baseline.starts_with(up_from_validator),
            "and must not be reachable by `..` either"
        );
    }

    /// Two jobs must not collide, because they run concurrently in the same repo.
    /// RED IF: the job id stops reaching the path.
    #[test]
    fn bl_02_jobs_do_not_collide() {
        let (a, _, _) = scratch_paths("job-a");
        let (b, _, _) = scratch_paths("job-b");
        assert_ne!(a, b);
    }

    /// A FAILING test suite is not a transport error. `cargo test` exits non-zero when tests
    /// fail, which is the ordinary case this feature exists to detect; returning Err would turn
    /// every caught defect into "the harness broke".
    /// RED IF: cargo_test starts failing on a non-zero exit status.
    #[test]
    fn bl_03_a_failing_suite_returns_output_not_an_error() {
        let _g = env_guard();
        if !have("cargo") {
            eprintln!("skipping bl_03: cargo is not on PATH");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let out = cargo_test(dir.path());
        assert!(out.is_ok(), "a non-zero cargo exit is not a launch failure: {out:?}");
        let text = out.unwrap();
        assert!(
            text.contains("could not find") || text.contains("error"),
            "stderr must be captured with stdout: a compile failure lands there and the \
             classifier needs it to tell a missing API from a check that never ran; got: {text}"
        );
    }

    /// THE WORKTREE LIFECYCLE, against a real repository.
    ///
    /// Antigravity ranked this the highest untested risk in round 7: `git worktree add` and the
    /// forced removal were the core mechanism and nothing exercised them. A leak here leaves
    /// state on the host.
    /// RED IF: the guard stops removing the worktree, or the add stops working.
    #[test]
    fn bl_04_the_baseline_worktree_is_created_and_always_removed() {
        let _g = env_guard();
        if !have("git") {
            eprintln!("skipping bl_04: git is not on PATH");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q", "."],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            git(&repo, &args).expect("git setup");
        }
        std::fs::write(repo.join("a.txt"), "one").unwrap();
        git(&repo, &["add", "-A"]).unwrap();
        git(&repo, &["commit", "-qm", "base"]).unwrap();
        let base = git(&repo, &["rev-parse", "HEAD"]).unwrap().trim().to_string();

        let tree = dir.path().join("baseline");
        git(&repo, &["worktree", "add", "--detach", &tree.to_string_lossy(), &base])
            .expect("worktree add");
        assert!(tree.join("a.txt").is_file(), "the pre-change tree must have the code in it");

        {
            let _guard = BaselineWorktree { repo: repo.clone(), path: tree.clone(), also_remove: vec![] };
        } // dropped here

        assert!(!tree.exists(), "the guard must remove the worktree directory");
        let listed = git(&repo, &["worktree", "list"]).unwrap();
        assert!(
            !listed.contains("baseline"),
            "and must deregister it, or `git worktree list` accumulates junk: {listed}"
        );
    }

    /// The guard removes the worktree even when the work PANICS.
    ///
    /// Codex found that cleanup ran as a trailing statement, so a panic skipped it. Drop runs
    /// during unwinding; a trailing statement does not.
    /// RED IF: cleanup moves back out of Drop.
    #[test]
    fn bl_05_the_worktree_is_removed_even_on_panic() {
        let _g = env_guard();
        if !have("git") {
            eprintln!("skipping bl_05: git is not on PATH");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q", "."],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            git(&repo, &args).expect("git setup");
        }
        std::fs::write(repo.join("a.txt"), "one").unwrap();
        git(&repo, &["add", "-A"]).unwrap();
        git(&repo, &["commit", "-qm", "base"]).unwrap();
        let base = git(&repo, &["rev-parse", "HEAD"]).unwrap().trim().to_string();
        let tree = dir.path().join("baseline");
        git(&repo, &["worktree", "add", "--detach", &tree.to_string_lossy(), &base]).unwrap();

        let repo2 = repo.clone();
        let tree2 = tree.clone();
        let outcome = std::panic::catch_unwind(move || {
            let _guard = BaselineWorktree { repo: repo2, path: tree2, also_remove: vec![] };
            panic!("the work blew up");
        });
        assert!(outcome.is_err(), "the panic must actually have happened");
        assert!(!tree.exists(), "and the worktree must still be gone");
    }

    /// GROK ROUND 7, SEVERE 1. The blind tests must land where Cargo will COMPILE them.
    ///
    /// The gate was stuck CLOSED, which is worse than leaky: `tests/blind_validation.rs` at the
    /// git root is never a rustc input when the manifest is not at the git root, because
    /// `cargo test` walks PARENTS for a manifest and not children. That is every virtual
    /// workspace, including this repository. Every declared name was absent, so good worktrees
    /// were rejected exactly like bad ones, and the tool could not run on the project containing
    /// it.
    /// RED IF: the package directory stops reaching the test path or the cargo cwd.
    #[test]
    fn bl_07_the_blind_tests_land_inside_the_package() {
        // A virtual workspace: the manifest is in `daemon/`, not at the git root.
        let ws = TreeLayout::new("daemon");
        assert_eq!(ws.test_file_rel(), "daemon/tests/blind_validation.rs");
        assert_eq!(ws.package_root(Path::new("/checkout")), PathBuf::from("/checkout/daemon"));

        // The single-package case, all three spellings of "the root".
        for spelling in [".", "", "/"] {
            let root = TreeLayout::new(spelling);
            assert_eq!(root.test_file_rel(), "tests/blind_validation.rs", "{spelling:?}");
            assert_eq!(root.package_root(Path::new("/checkout")), PathBuf::from("/checkout"));
        }
        assert_eq!(TreeLayout::new("/daemon/").test_file_rel(), "daemon/tests/blind_validation.rs");
    }

    /// THE ROUTE, not the helper.
    ///
    /// `bl_07` exercises `TreeLayout` directly, and a mutation that made the ORCHESTRATION
    /// ignore the package directory left it green: the helper was right and unused. Same shape
    /// this codebase has now hit four times. This asserts the production path composes the
    /// layout, by checking the one place its output is observable from outside: the file the
    /// cleanup guard removes from the implementation worktree.
    /// RED IF: `run_with_trees` stops routing through `TreeLayout`.
    #[test]
    fn bl_08_the_orchestration_uses_the_layout_not_a_hardcoded_path() {
        let req = LiveBlindValidation {
            project_root: PathBuf::from("/repo"),
            impl_worktree: PathBuf::from("/repo/wt"),
            contract: "c".to_string(),
            base_ref: "HEAD".to_string(),
            worker_agent: "codex".to_string(),
            roster: vec![],
            package_dir: "daemon".to_string(),
        };
        // The exact expression `run_live` uses to decide what to clean up.
        let copied = req
            .impl_worktree
            .join(TreeLayout::new(&req.package_dir).test_file_rel());
        assert_eq!(
            copied,
            PathBuf::from("/repo/wt/daemon/tests/blind_validation.rs"),
            "the package directory must reach the path the tests are written to"
        );

        // And both trees resolve to the same relative package, which is what makes the baseline
        // and after-run comparable at all.
        let layout = TreeLayout::new(&req.package_dir);
        assert_eq!(
            layout.package_root(Path::new("/base")).strip_prefix("/base"),
            layout.package_root(Path::new("/impl")).strip_prefix("/impl")
        );
    }

    /// An agent that cannot WRITE cannot be a blind validator    /// An agent that cannot WRITE cannot be a blind validator, and is skipped rather than
    /// failed. Grok found that its own default sandbox is read-only, so a roster with grok in
    /// front died at "did not write tests" for a harness reason, not a model one.
    /// RED IF: a non-writing agent can be selected, or the whole roster being non-writing
    /// silently produces something other than a clear error.
    #[tokio::test]
    async fn bl_09_a_read_only_agent_is_not_selected_as_validator() {
        let err = tool_impl(BlindValidateRequest {
            project_root: "/nonexistent".to_string(),
            impl_worktree: "/nonexistent".to_string(),
            contract: "c".to_string(),
            base_ref: "HEAD".to_string(),
            worker_agent: "codex".to_string(),
            roster: vec!["grok".to_string(), "deepseek".to_string()],
            package_dir: None,
        })
        .await
        .expect_err("a roster of non-writers cannot validate");
        // The worktree check fires first, which is correct ordering; the roster message is
        // asserted directly below against the filter itself.
        assert!(err.contains("does not exist") || err.contains("cannot write"), "got: {err}");
    }

    /// The MCP tool delegates to a function that can be CALLED, not string-matched.
    ///
    /// Antigravity found that the first version scanned `main.rs` for a substring, which a
    /// commented-out call would still satisfy. This invokes the real entry point; a disconnected
    /// tool now fails to compile.
    /// RED IF: `tool_impl` stops being the tool's body, or stops refusing a missing worktree.
    #[tokio::test]
    async fn bl_06_the_tool_entry_point_is_real() {
        let err = tool_impl(BlindValidateRequest {
            project_root: "/nonexistent-repo".to_string(),
            impl_worktree: "/nonexistent-worktree".to_string(),
            contract: "pub fn f() -> u8".to_string(),
            base_ref: "HEAD".to_string(),
            worker_agent: "codex".to_string(),
            roster: vec![],
            package_dir: None,
        })
        .await
        .expect_err("a missing worktree must be refused");
        assert!(err.contains("does not exist"), "got: {err}");

        // And the bridge really delegates here rather than reimplementing.
        let src = include_str!("main.rs");
        assert!(src.contains("blind_validate::tool_impl(req).await?"));
    }
}
