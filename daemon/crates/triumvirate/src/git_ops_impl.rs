use std::path::{Path, PathBuf};

use async_trait::async_trait;
use shared_types::{GitOps, MergeResult};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct RealGitOps {
    repo_root: PathBuf,
}

impl RealGitOps {
    pub fn new(repo_root: PathBuf) -> anyhow::Result<Self> {
        if !repo_root.is_absolute() {
            anyhow::bail!("repo_root must be absolute");
        }
        Ok(Self { repo_root })
    }

    async fn git(&self, args: &[&str]) -> anyhow::Result<std::process::Output> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repo_root)
            .args(args)
            .output()
            .await?;
        Ok(output)
    }

    fn ensure_success(output: &std::process::Output, context: &str) -> anyhow::Result<()> {
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stdout.is_empty() {
            stderr
        } else {
            format!("{stderr} | {stdout}")
        };
        anyhow::bail!("{context} failed: {detail}");
    }
}

#[async_trait]
impl GitOps for RealGitOps {
    async fn worktree_add(&self, path: &Path, branch: &str) -> anyhow::Result<()> {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("worktree path is not valid UTF-8"))?;
        let output = self
            .git(&["worktree", "add", "-b", branch, path_str, "HEAD"])
            .await?;
        Self::ensure_success(&output, "git worktree add")
    }

    async fn worktree_remove(&self, path: &Path) -> anyhow::Result<()> {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("worktree path is not valid UTF-8"))?;
        let output = self.git(&["worktree", "remove", "--force", path_str]).await?;
        Self::ensure_success(&output, "git worktree remove")
    }

    async fn is_clean(&self) -> anyhow::Result<bool> {
        // Check tracked files only — untracked files (.pythia, build artifacts) are not dirty.
        let staged = self.git(&["diff", "--quiet", "--cached"]).await?;
        let unstaged = self.git(&["diff", "--quiet"]).await?;
        let clean = staged.status.success() && unstaged.status.success();
        if !clean {
            tracing::warn!(
                repo = %self.repo_root.display(),
                staged_ok = staged.status.success(),
                unstaged_ok = unstaged.status.success(),
                staged_stderr = %String::from_utf8_lossy(&staged.stderr),
                unstaged_stderr = %String::from_utf8_lossy(&unstaged.stderr),
                "is_clean check failed"
            );
        }
        Ok(clean)
    }

    async fn current_head(&self) -> anyhow::Result<String> {
        let output = self.git(&["rev-parse", "HEAD"]).await?;
        Self::ensure_success(&output, "git rev-parse HEAD")?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    async fn merge(&self, branch: &str) -> anyhow::Result<MergeResult> {
        let output = self.git(&["merge", "--no-ff", "--no-edit", branch]).await?;
        if output.status.success() {
            return Ok(MergeResult::Success);
        }

        let conflicts = self.git(&["diff", "--name-only", "--diff-filter=U"]).await?;
        let files = String::from_utf8_lossy(&conflicts.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if !files.is_empty() {
            return Ok(MergeResult::Conflict { files });
        }

        Self::ensure_success(&output, "git merge")?;
        Ok(MergeResult::Success)
    }

    async fn diff(&self, branch: &str) -> anyhow::Result<String> {
        let output = self.git(&["diff", branch]).await?;
        Self::ensure_success(&output, "git diff")?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn rev_parse_toplevel(&self, cwd: &Path) -> anyhow::Result<PathBuf> {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .await?;
        Self::ensure_success(&output, "git rev-parse --show-toplevel")?;
        Ok(PathBuf::from(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::RealGitOps;
    use shared_types::GitOps;

    async fn init_repo(path: &Path) -> anyhow::Result<()> {
        tokio::process::Command::new("git")
            .arg("init")
            .arg(path)
            .output()
            .await?;
        tokio::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["config", "user.email", "test@example.com"])
            .output()
            .await?;
        tokio::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["config", "user.name", "Triumvirate Test"])
            .output()
            .await?;
        fs::write(path.join("README.md"), "init\n")?;
        tokio::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["add", "README.md"])
            .output()
            .await?;
        tokio::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["commit", "-m", "init"])
            .output()
            .await?;
        Ok(())
    }

    #[tokio::test]
    async fn worktree_add_clean_check_and_remove_are_real() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo)?;
        init_repo(&repo).await?;

        let git = RealGitOps::new(repo.clone())?;
        let worktree_path = temp.path().join("wt-feature");

        git.worktree_add(&worktree_path, "feature/test").await?;
        assert!(worktree_path.exists());
        assert!(worktree_path.join(".git").exists());

        assert!(git.is_clean().await?);
        fs::write(repo.join("dirty.txt"), "dirty\n")?;
        assert!(!git.is_clean().await?);

        git.worktree_remove(&worktree_path).await?;
        assert!(!worktree_path.exists());
        Ok(())
    }
}
