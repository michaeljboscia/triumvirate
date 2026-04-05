use std::path::PathBuf;
use std::process::Command;

use crate::config;

#[derive(Debug, Clone)]
pub struct FleetMember {
    pub member_key: String,
    pub agent_type: String,
}

#[derive(Debug, Clone)]
pub struct WorktreeProvisioned {
    pub member_key: String,
    pub agent_type: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
}

/// Parse a fleet spec like "2 claude, 1 codex: build auth" into normalized members.
pub fn parse_fleet_members(spec: &str) -> Vec<FleetMember> {
    let mut counts: Vec<(String, usize)> = Vec::new();

    let normalized = spec.to_ascii_lowercase().replace(':', ",");
    for segment in normalized.split(',') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }
        let mut parts = segment.split_whitespace();
        let first = match parts.next() {
            Some(v) => v,
            None => continue,
        };
        let second = match parts.next() {
            Some(v) => v,
            None => continue,
        };

        let (count, raw_agent) = if let Ok(parsed) = first.parse::<usize>() {
            (parsed.max(1), second)
        } else {
            (1, first)
        };
        let agent = normalize_agent(raw_agent);
        if !matches!(agent.as_str(), "claude" | "gemini" | "codex") {
            continue;
        }
        counts.push((agent, count));
    }

    if counts.is_empty() {
        counts.push(("codex".to_string(), 1));
    }

    let mut members = Vec::new();
    for (agent, count) in counts {
        for idx in 1..=count {
            members.push(FleetMember {
                member_key: format!("{agent}-{idx}"),
                agent_type: agent.clone(),
            });
        }
    }
    members
}

/// Provision a git worktree for one fleet member.
pub fn provision_worktree(
    fleet_id: &str,
    member: &FleetMember,
    base_ref: &str,
) -> anyhow::Result<WorktreeProvisioned> {
    let repo_root = git_repo_root()?;
    let sanitized_fleet = sanitize_token(fleet_id);
    let sanitized_member = sanitize_token(&member.member_key);
    let branch_name = format!("fleet/{sanitized_fleet}/{sanitized_member}");
    let worktree_path = config::dirs()
        .join("worktrees")
        .join(sanitized_fleet)
        .join(sanitized_member);

    if let Some(parent) = worktree_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let status = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .arg("worktree")
        .arg("add")
        .arg("-b")
        .arg(&branch_name)
        .arg(&worktree_path)
        .arg(base_ref)
        .status()?;
    if !status.success() {
        anyhow::bail!(
            "git worktree add failed for member={} branch={}",
            member.member_key,
            branch_name
        );
    }

    Ok(WorktreeProvisioned {
        member_key: member.member_key.clone(),
        agent_type: member.agent_type.clone(),
        branch_name,
        worktree_path,
    })
}

/// Remove an existing worktree path and prune stale metadata.
pub fn remove_worktree(worktree_path: &str) -> anyhow::Result<()> {
    let repo_root = git_repo_root()?;
    let status = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(worktree_path)
        .status()?;
    if !status.success() {
        anyhow::bail!("git worktree remove failed for path={worktree_path}");
    }

    let prune_status = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .arg("worktree")
        .arg("prune")
        .status()?;
    if !prune_status.success() {
        anyhow::bail!("git worktree prune failed");
    }
    Ok(())
}

pub fn git_repo_root() -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()?;
    if !output.status.success() {
        anyhow::bail!("failed to discover git repo root");
    }
    let root = String::from_utf8(output.stdout)?.trim().to_string();
    if root.is_empty() {
        anyhow::bail!("git repo root is empty");
    }
    Ok(PathBuf::from(root))
}

fn sanitize_token(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

fn normalize_agent(raw: &str) -> String {
    let candidate = raw
        .trim()
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    if candidate.starts_with("claude") {
        "claude".to_string()
    } else if candidate.starts_with("gemini") {
        "gemini".to_string()
    } else if candidate.starts_with("codex") {
        "codex".to_string()
    } else {
        candidate
    }
}

#[cfg(test)]
mod tests {
    use super::parse_fleet_members;

    #[test]
    fn parse_defaults_to_single_codex() {
        let members = parse_fleet_members("build auth end-to-end");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].agent_type, "codex");
    }

    #[test]
    fn parse_counts_and_agents() {
        let members = parse_fleet_members("2 claude, 1 codex: implement API");
        assert_eq!(members.len(), 3);
        assert_eq!(members[0].member_key, "claude-1");
        assert_eq!(members[1].member_key, "claude-2");
        assert_eq!(members[2].member_key, "codex-1");
    }
}
