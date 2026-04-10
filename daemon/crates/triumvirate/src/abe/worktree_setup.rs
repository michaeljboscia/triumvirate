use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Context;
use shared_types::ContractFields;
use tracing::instrument;

#[derive(Debug, Clone)]
pub struct WorktreeSetupRequest {
    pub project_root: PathBuf,
    pub sha: String,
    pub task_id: String,
    pub briefing_content: String,
    pub contract_fields: ContractFields,
}

#[derive(Debug, Clone)]
pub struct WorktreeSetupResult {
    pub worktree_path: PathBuf,
}

#[instrument(skip_all, fields(task_id = %req.task_id, wave = req.contract_fields.wave))]
pub fn setup_worktree(req: &WorktreeSetupRequest) -> anyhow::Result<WorktreeSetupResult> {
    let worktree_base = req.project_root.join(".triumvirate").join("abe-worktrees");
    fs::create_dir_all(&worktree_base)?;

    let worktree_path = worktree_base.join(format!("{}-{}", req.task_id, short_sha(&req.sha)));
    if worktree_path.exists() {
        rollback_worktree(&req.project_root, &worktree_path).ok();
    }

    run_git(
        &req.project_root,
        [
            "worktree",
            "add",
            worktree_path.to_string_lossy().as_ref(),
            req.sha.as_str(),
        ],
    )
    .with_context(|| format!("failed to create worktree for {}", req.task_id))?;

    let triumvirate_dir = worktree_path.join(".triumvirate");
    fs::create_dir_all(triumvirate_dir.join("hooks"))?;

    fs::write(
        triumvirate_dir.join("BRIEFING.md"),
        req.briefing_content.as_bytes(),
    )?;
    fs::write(
        triumvirate_dir.join("contract.json"),
        serde_json::to_vec_pretty(&req.contract_fields)?,
    )?;

    write_validate_script(&triumvirate_dir)?;
    write_pre_commit_hook(&triumvirate_dir)?;
    write_commit_msg_hook(&triumvirate_dir)?;

    ensure_exclude_entry(&worktree_path, ".triumvirate/")?;

    run_git(
        &worktree_path,
        ["config", "--worktree", "core.hooksPath", ".triumvirate/hooks"],
    )?;

    Ok(WorktreeSetupResult { worktree_path })
}

#[instrument(skip_all)]
pub fn rollback_worktree(project_root: &Path, worktree_path: &Path) -> anyhow::Result<()> {
    if worktree_path.exists() {
        let _ = run_git(
            project_root,
            [
                "worktree",
                "remove",
                "--force",
                worktree_path.to_string_lossy().as_ref(),
            ],
        );
        let _ = fs::remove_dir_all(worktree_path);
    }
    Ok(())
}

fn run_git<'a, I, S>(cwd: &Path, args: I) -> anyhow::Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str> + 'a,
{
    let args_vec = args.into_iter().map(|s| s.as_ref().to_string()).collect::<Vec<_>>();
    let out = Command::new("git").current_dir(cwd).args(&args_vec).output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("git {} failed: {}", args_vec.join(" "), stderr.trim());
    }
    Ok(())
}

fn ensure_exclude_entry(worktree_path: &Path, entry: &str) -> anyhow::Result<()> {
    let info_dir = resolve_git_dir(worktree_path).join("info");
    fs::create_dir_all(&info_dir)?;
    let exclude_path = info_dir.join("exclude");
    let mut contents = fs::read_to_string(&exclude_path).unwrap_or_default();
    if !contents.lines().any(|line| line.trim() == entry) {
        if !contents.ends_with('\n') && !contents.is_empty() {
            contents.push('\n');
        }
        contents.push_str(entry);
        contents.push('\n');
        fs::write(exclude_path, contents.as_bytes())?;
    }
    Ok(())
}

fn resolve_git_dir(worktree_path: &Path) -> PathBuf {
    let dot_git = worktree_path.join(".git");
    if dot_git.is_file() {
        let content = fs::read_to_string(&dot_git).unwrap_or_default();
        if let Some(gitdir) = content.lines().find_map(|line| line.strip_prefix("gitdir:")) {
            let raw = gitdir.trim();
            let parsed = PathBuf::from(raw);
            if parsed.is_absolute() {
                return parsed;
            }
            return worktree_path.join(parsed);
        }
    }
    dot_git
}

fn write_validate_script(triumvirate_dir: &Path) -> anyhow::Result<()> {
    let dst = triumvirate_dir.join("validate-task.sh");
    let source = dirs::home_dir()
        .map(|h| h.join(".claude/scripts/validate-task.sh"))
        .filter(|p| p.exists());

    if let Some(source) = source {
        fs::copy(source, &dst)?;
    } else {
        fs::write(
            &dst,
            b"#!/usr/bin/env bash\nset -euo pipefail\necho 'validate-task fallback: PASS'\n",
        )?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dst)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dst, perms)?;
    }
    Ok(())
}

fn write_pre_commit_hook(triumvirate_dir: &Path) -> anyhow::Result<()> {
    let hook_path = triumvirate_dir.join("hooks").join("pre-commit");
    let mut file = fs::File::create(&hook_path)?;
    file.write_all(
        br#"#!/usr/bin/env bash
set -euo pipefail
contract=".triumvirate/contract.json"
if [[ ! -f "$contract" ]]; then
  echo "BLOCKED: missing .triumvirate/contract.json"
  exit 1
fi

msg=$(git log -1 --pretty=%B 2>/dev/null || true)
while IFS= read -r file; do
  [[ -z "$file" ]] && continue
  if [[ "$file" == .triumvirate/* ]]; then
    continue
  fi
  if ! jq -e --arg file "$file" '.allowed_files | index($file) != null' "$contract" >/dev/null; then
    echo "BLOCKED: Write to $file denied by contract"
    exit 1
  fi
  if grep -rnE "TODO|FIXME|unimplemented!|placeholder" "$file" >/dev/null 2>&1; then
    echo "BLOCKED: stub marker detected in $file"
    exit 1
  fi
done < <(git diff --cached --name-only)

test_cmd=$(jq -r '.test_command // empty' "$contract")
if [[ -n "$test_cmd" ]]; then
  if ! eval "$test_cmd" >/dev/null 2>&1; then
    echo "BLOCKED: test command failed: $test_cmd"
    exit 1
  fi
fi
"#,
    )?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms)?;
    }

    Ok(())
}

fn write_commit_msg_hook(triumvirate_dir: &Path) -> anyhow::Result<()> {
    let hook_path = triumvirate_dir.join("hooks").join("commit-msg");
    let mut file = fs::File::create(&hook_path)?;
    file.write_all(
        br#"#!/usr/bin/env bash
set -euo pipefail
contract=".triumvirate/contract.json"
if [[ ! -f "$contract" ]]; then
  echo "BLOCKED: missing .triumvirate/contract.json"
  exit 1
fi
message_file="${1:-}"
if [[ -z "$message_file" || ! -f "$message_file" ]]; then
  echo "BLOCKED: commit-msg hook requires message file path"
  exit 1
fi
msg=$(cat "$message_file")
commit_format=$(jq -r '.commit_format // empty' "$contract")
if [[ -n "$commit_format" ]]; then
  if ! [[ "$msg" =~ $commit_format ]]; then
    echo "BLOCKED: Commit message does not match contract format"
    exit 1
  fi
fi
"#,
    )?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&hook_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&hook_path, perms)?;
    }
    Ok(())
}

fn short_sha(sha: &str) -> &str {
    if sha.len() > 12 {
        &sha[..12]
    } else {
        sha
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_git_dir, setup_worktree, WorktreeSetupRequest};
    use shared_types::{ContractFields, FilePolicy};
    use std::path::PathBuf;
    use std::process::Command;

    #[test]
    fn setup_worktree_handles_dot_git_file_and_updates_exclude() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path();
        assert!(Command::new("git").arg("init").arg(repo).status().expect("git init").success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["config", "user.email", "abe@example.com"])
            .status()
            .expect("git config email")
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["config", "user.name", "ABE"])
            .status()
            .expect("git config name")
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["config", "extensions.worktreeConfig", "true"])
            .status()
            .expect("git config worktreeConfig")
            .success());
        std::fs::write(repo.join("README.md"), "hello\n").expect("write readme");
        assert!(Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["add", "README.md"])
            .status()
            .expect("git add")
            .success());
        assert!(Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-m", "init"])
            .status()
            .expect("git commit")
            .success());
        let sha = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("rev-parse")
                .stdout,
        )
        .expect("utf8")
        .trim()
        .to_string();

        let request = WorktreeSetupRequest {
            project_root: repo.to_path_buf(),
            sha,
            task_id: "T-003".to_string(),
            briefing_content: "briefing".to_string(),
            contract_fields: ContractFields {
                task_id: "T-003".to_string(),
                req_ids: vec!["REQ-A1.2".to_string()],
                wave: 1,
                file_policy: FilePolicy::DefaultDeny,
                allowed_files: vec!["README.md".to_string()],
                forbidden_files: vec![],
                allowed_commands: vec![vec!["cargo".to_string(), "check".to_string()]],
                forbidden_commands: vec![],
                commit_format: "^T-003:".to_string(),
                test_command: "true".to_string(),
                task_timeout_sec: 60,
                done_when: "done".to_string(),
                reality_test: "real".to_string(),
            },
        };

        let setup = setup_worktree(&request).expect("setup worktree");
        assert!(
            setup.worktree_path.join(".git").is_file(),
            ".git must be a file in worktree"
        );

        let git_dir = resolve_git_dir(&setup.worktree_path);
        let exclude = std::fs::read_to_string(git_dir.join("info").join("exclude"))
            .expect("read exclude");
        assert!(exclude.lines().any(|line| line.trim() == ".triumvirate/"));
        assert!(setup.worktree_path.join(".triumvirate/hooks/commit-msg").exists());
    }

    #[test]
    fn resolve_git_dir_supports_relative_pointer() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let wt = tmp.path().join("wt");
        std::fs::create_dir_all(&wt).expect("mkdir");
        std::fs::write(wt.join(".git"), "gitdir: ../.git/worktrees/demo\n").expect("write");
        let resolved = resolve_git_dir(&wt);
        assert_eq!(resolved, PathBuf::from(&wt).join("../.git/worktrees/demo"));
    }
}
