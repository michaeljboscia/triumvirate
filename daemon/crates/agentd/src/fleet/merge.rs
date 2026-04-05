use std::path::Path;
use std::process::Command;

/// Merge a list of branches sequentially into the current branch.
///
/// Returns `(merged, failed_branch)` where `merged` is ordered by completion.
pub fn merge_branches_sequentially(
    repo_root: &Path,
    branches: &[String],
) -> anyhow::Result<(Vec<String>, Option<String>)> {
    ensure_clean_worktree(repo_root)?;

    let mut merged = Vec::new();
    for branch in branches {
        let status = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .arg("merge")
            .arg("--no-ff")
            .arg("--no-edit")
            .arg(branch)
            .status()?;

        if status.success() {
            merged.push(branch.clone());
            continue;
        }

        let _ = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .arg("merge")
            .arg("--abort")
            .status();
        return Ok((merged, Some(branch.clone())));
    }

    Ok((merged, None))
}

fn ensure_clean_worktree(repo_root: &Path) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("status")
        .arg("--porcelain")
        .output()?;
    if !output.status.success() {
        anyhow::bail!("failed to inspect git status");
    }
    if !output.stdout.is_empty() {
        anyhow::bail!("refusing merge: repo has local changes");
    }
    Ok(())
}
