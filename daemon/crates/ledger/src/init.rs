use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tracing::{instrument, warn};

const EXCLUDE_ENTRY: &[u8] = b".triumvirate/";

fn resolve_path(base: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn git_common_dir(project_root: &Path) -> anyhow::Result<Option<PathBuf>> {
    for dir in project_root.ancestors() {
        let dot_git = dir.join(".git");
        if !dot_git.exists() {
            continue;
        }

        let gitdir = if dot_git.is_dir() {
            dot_git
        } else {
            let contents = fs::read_to_string(&dot_git)?;
            let value = contents
                .lines()
                .find_map(|line| line.strip_prefix("gitdir:"))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("unparseable gitdir file: {}", dot_git.display()))?;
            resolve_path(dir, value)
        };

        let commondir_path = gitdir.join("commondir");
        if commondir_path.exists() {
            let value = fs::read_to_string(&commondir_path)?;
            let value = value.trim();
            if value.is_empty() {
                anyhow::bail!("empty commondir file: {}", commondir_path.display());
            }
            return Ok(Some(resolve_path(&gitdir, value)));
        }
        return Ok(Some(gitdir));
    }
    Ok(None)
}

fn write_exclude(project_root: &Path) -> anyhow::Result<()> {
    let Some(common) = git_common_dir(project_root)? else {
        return Ok(());
    };
    let common_metadata = fs::symlink_metadata(&common)?;
    if !common_metadata.is_dir() {
        anyhow::bail!("git common dir is not a directory: {}", common.display());
    }

    let info = common.join("info");
    match fs::symlink_metadata(&info) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                anyhow::bail!("git info path is a symlink: {}", info.display());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&info)?,
        Err(error) => return Err(error.into()),
    }

    let exclude = info.join("exclude");
    match fs::symlink_metadata(&exclude) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                anyhow::bail!("git exclude path is a symlink: {}", exclude.display());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&exclude)?;
    file.lock()?;

    let mut existing = Vec::new();
    file.read_to_end(&mut existing)?;
    if existing
        .split(|byte| *byte == b'\n')
        .any(|line| line.trim_ascii() == EXCLUDE_ENTRY)
    {
        return Ok(());
    }

    let mut addition = Vec::with_capacity(EXCLUDE_ENTRY.len() + 2);
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        addition.push(b'\n');
    }
    addition.extend_from_slice(EXCLUDE_ENTRY);
    addition.push(b'\n');
    file.write_all(&addition)?;
    Ok(())
}

#[instrument(
    skip_all,
    fields(
        event_type = "exclude_setup",
        spool_size = tracing::field::Empty,
        operation = "ensure_triumvirate_excluded"
    )
)]
pub(crate) fn ensure_triumvirate_excluded(project_root: &Path) {
    if let Err(error) = write_exclude(project_root) {
        warn!(project_root = %project_root.display(), %error, "failed to update git local exclude");
    }
}
