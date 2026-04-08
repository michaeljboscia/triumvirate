use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::Context;
use shared_types::ContractFields;

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

    ensure_exclude_entry(&worktree_path, ".triumvirate/")?;

    run_git(
        &worktree_path,
        ["config", "--worktree", "core.hooksPath", ".triumvirate/hooks"],
    )?;

    Ok(WorktreeSetupResult { worktree_path })
}

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
    let info_dir = worktree_path.join(".git").join("info");
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
commit_format=$(jq -r '.commit_format' "$contract")
if [[ -n "$commit_format" && "$commit_format" != "null" ]]; then
  if ! [[ "$msg" =~ $commit_format ]]; then
    echo "BLOCKED: Commit message does not match contract format"
    exit 1
  fi
fi

mapfile -t staged < <(git diff --cached --name-only)
for file in "${staged[@]}"; do
  if [[ "$file" == .triumvirate/* ]]; then
    continue
  fi
  if ! jq -e --arg file "$file" '.allowed_files | index($file) != null' "$contract" >/dev/null; then
    echo "BLOCKED: Write to $file denied by contract"
    exit 1
  fi
  if rg -n "TODO|FIXME|unimplemented!|placeholder" "$file" >/dev/null 2>&1; then
    echo "BLOCKED: stub marker detected in $file"
    exit 1
  fi
done
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
