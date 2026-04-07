use std::{fs, path::Path};

const GITIGNORE_ENTRY: &str = ".triumvirate/";

fn is_git_repo(project_root: &Path) -> bool {
    project_root.join(".git").exists()
}

pub(crate) fn ensure_triumvirate_gitignore(project_root: &Path) -> anyhow::Result<()> {
    if !is_git_repo(project_root) {
        return Ok(());
    }

    let gitignore_path = project_root.join(".gitignore");
    if gitignore_path.exists() {
        let existing = fs::read_to_string(&gitignore_path)?;
        if existing.lines().any(|line| line.trim() == GITIGNORE_ENTRY) {
            return Ok(());
        }
        let mut updated = existing;
        if !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(GITIGNORE_ENTRY);
        updated.push('\n');
        fs::write(gitignore_path, updated)?;
        return Ok(());
    }

    fs::write(gitignore_path, format!("{GITIGNORE_ENTRY}\n"))?;
    Ok(())
}
