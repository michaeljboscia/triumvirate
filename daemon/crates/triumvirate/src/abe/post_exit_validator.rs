use std::path::Path;
use std::process::Command;

use shared_types::ContractFields;

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub passed: bool,
    pub violations: Vec<String>,
}

/// Validate a worker's commit against the ORIGINAL contract (daemon-side, untamperable).
/// This runs AFTER the worker exits, outside the worker's control.
/// The worker can --no-verify, rewrite hooks, modify contract.json — none of it matters
/// because this function uses the contract the DAEMON holds, not the worktree copy.
pub fn validate_commit(
    worktree_path: &Path,
    contract: &ContractFields,
    start_sha: &str,
) -> ValidationResult {
    let mut violations = Vec::new();

    // 1. Check modified files against allowed_files (default-deny)
    let modified = get_modified_files(worktree_path, start_sha);
    for file in &modified {
        if !contract.allowed_files.contains(file) {
            violations.push(format!(
                "FILE_SCOPE: '{}' modified but not in allowed_files {:?}",
                file, contract.allowed_files
            ));
        }
    }

    // 2. Check commit message format
    let commit_msg = get_commit_message(worktree_path);
    if !contract.commit_format.is_empty() {
        let re = regex_lite::Regex::new(&contract.commit_format);
        match re {
            Ok(re) => {
                if !re.is_match(&commit_msg) {
                    violations.push(format!(
                        "COMMIT_FORMAT: message '{}' does not match format '{}'",
                        commit_msg.lines().next().unwrap_or(""),
                        contract.commit_format
                    ));
                }
            }
            Err(_) => {
                // Fallback: simple starts-with check
                let prefix = contract.commit_format.trim_start_matches('^');
                if !commit_msg.starts_with(prefix) {
                    violations.push(format!(
                        "COMMIT_FORMAT: message does not start with '{}'",
                        prefix
                    ));
                }
            }
        }
    }

    // 3. Scan for stub markers in modified files
    let stub_patterns = [
        "todo!()", "unimplemented!()", "TODO", "FIXME", "XXX", "HACK",
        "NotImplementedError", "placeholder", "not implemented", "implement me",
    ];
    for file in &modified {
        let full_path = worktree_path.join(file);
        if let Ok(content) = std::fs::read_to_string(&full_path) {
            for pattern in &stub_patterns {
                if content.contains(pattern) {
                    violations.push(format!(
                        "STUB_MARKER: '{}' found in {}",
                        pattern, file
                    ));
                }
            }
        }
    }

    // 4. Run test command
    if !contract.test_command.is_empty() {
        let test_result = Command::new("sh")
            .arg("-c")
            .arg(&contract.test_command)
            .current_dir(worktree_path)
            .output();
        match test_result {
            Ok(output) => {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    violations.push(format!(
                        "TEST_FAILED: '{}' exited {}: {}",
                        contract.test_command,
                        output.status.code().unwrap_or(-1),
                        stderr.chars().take(200).collect::<String>()
                    ));
                }
            }
            Err(e) => {
                violations.push(format!(
                    "TEST_ERROR: failed to run '{}': {}",
                    contract.test_command, e
                ));
            }
        }
    }

    ValidationResult {
        passed: violations.is_empty(),
        violations,
    }
}

fn get_modified_files(worktree_path: &Path, start_sha: &str) -> Vec<String> {
    let output = Command::new("git")
        .args(["diff", "--name-only", start_sha, "HEAD"])
        .current_dir(worktree_path)
        .output();
    output
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn get_commit_message(worktree_path: &Path) -> String {
    let output = Command::new("git")
        .args(["log", "-1", "--pretty=%B"])
        .current_dir(worktree_path)
        .output();
    output
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::FilePolicy;

    fn test_contract() -> ContractFields {
        ContractFields {
            task_id: "T-TEST".to_string(),
            req_ids: vec!["REQ-TEST".to_string()],
            wave: 0,
            file_policy: FilePolicy::DefaultDeny,
            allowed_files: vec!["src/hello.rs".to_string()],
            forbidden_files: vec!["README.md".to_string()],
            allowed_commands: vec![],
            forbidden_commands: vec![],
            commit_format: "^T-TEST:".to_string(),
            test_command: "echo PASS".to_string(),
            task_timeout_sec: 60,
            done_when: "test".to_string(),
            reality_test: "test".to_string(),
        }
    }

    #[test]
    fn violations_detected_for_wrong_files() {
        // Simulate: worker modified README.md which is not in allowed_files
        let result = ValidationResult {
            passed: false,
            violations: vec!["FILE_SCOPE: 'README.md' modified but not in allowed_files".to_string()],
        };
        assert!(!result.passed);
        assert!(result.violations[0].contains("README.md"));
    }
}
