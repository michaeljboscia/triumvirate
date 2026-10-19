//! Seat containment (Seven Primes FEAT-005, REQ-030, REQ-031, REQ-033, REQ-035): seat homes and their daemon-written
//! files, the child environment (scrub, then add), the macOS seatbelt profile, and the post-run repo audit.
//! BACKEND_STRUCTURE.md section 5 is the contract.
//!
//! The macOS seatbelt is the PRIMARY write barrier and the repo audit the backstop: a mode label ("plan", "safe-mode")
//! is never trusted alone (Vibe #1267: `--agent plan` applied edits; LESSONS.md 3).

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::seat_profile::SeatProfile;

/// One read-only dispatch to a new seat, as the per-seat builders (kimi.rs, mistral.rs, qwen.rs) receive it.
#[derive(Debug, Clone)]
pub struct SeatRequest<'a> {
    pub prompt: &'a str,
    /// The repo the seat reads (never written: REQ-035).
    pub cwd: &'a Path,
    /// `$TRIUMVIRATE_HOME/homes/<seat>`, also the child's HOME (REQ-030).
    pub seat_home: &'a Path,
    /// Per-call directory the CLI may write its own output into (Mistral export.json).
    pub output_dir: &'a Path,
    pub model: Option<&'a str>,
    /// The vendor session id to resume, or `None` for a fresh session.
    pub resume_session_id: Option<&'a str>,
    /// Operator extra args; each builder rejects its forbidden list (BACKEND_STRUCTURE section 6).
    pub extra_args: &'a [String],
    pub max_turns: u32,
    pub time_limit: Duration,
}

/// What a builder returns: the program, its argv, and the env changes the runner applies on top of the scrubbed env.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatInvocation {
    pub program: String,
    pub args: Vec<String>,
    /// Applied after `env_remove`, so a value here always wins (Mistral `VIBE_CLI=python`).
    pub env_add: Vec<(String, String)>,
    /// Exact names or `PREFIX*` patterns removed from the inherited env.
    pub env_remove: Vec<String>,
    pub cwd: PathBuf,
}

/// The tracked and untracked state of a repo before or after a run (REQ-035).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoSnapshot {
    /// Path -> content hash (or a marker for a deletion), from `git status --porcelain=v1 -z --untracked-files=all`.
    pub entries: std::collections::BTreeMap<String, String>,
}

/// Inherited vendor credential and endpoint variables removed from every new seat's child env (REQ-033). A trailing
/// `*` is a prefix pattern. A seat's own credential is added back afterwards by `env_add`, so it is the only one left.
/// T-201's kimi, mistral and qwen rows use this as their `env_remove`.
pub const SEAT_CREDENTIAL_SCRUB: &[&str] = &[
    "MISTRAL_API_KEY",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "DASHSCOPE_*",
    "BAILIAN_*",
    "QWENCLOUD_*",
    "KIMI_*",
    "MOONSHOT_*",
];

fn matches_pattern(name: &str, pattern: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// A seat key becomes a path component, so it must not be able to address anything outside homes/.
fn check_seat_name(seat: &str) -> Result<(), String> {
    let ok = !seat.is_empty()
        && seat
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid_seat_name: {seat:?} (lowercase ascii, digits and '-' only)"
        ))
    }
}

/// `$TRIUMVIRATE_HOME/homes/<seat>`.
pub fn seat_home(triumvirate_home: &Path, seat: &str) -> Result<PathBuf, String> {
    check_seat_name(seat)?;
    Ok(triumvirate_home.join("homes").join(seat))
}

/// Kimi's read-only agent file (BACKEND_STRUCTURE section 5): tools limited to read and search, no subagents.
const KIMI_RO_AGENT: &str = "---\nname: triumvirate-readonly\ntools: [Read, Grep, Glob]\ndisallowedTools: [Agent, AgentSwarm]\nsubagents: []\n---\nRead-only reviewer: never write files.\n";

/// Write the daemon-owned files into a seat home: Kimi ro-agent.md (0444), Qwen settings.json, Mistral config.toml.
///
/// Qwen and Mistral files are MERGED, not replaced: only the daemon's own keys are set, so anything the CLI recorded
/// at login survives. Kimi's ro-agent.md is wholly the daemon's and is rewritten.
///
/// A seat can write inside its own home, so it could plant a symlink at any of these names. Every file is checked
/// with `symlink_metadata` before it is read, and is replaced by a fresh temp file renamed over it (rename replaces a
/// symlink itself, never its target), so the unsandboxed daemon never reads, chmods or writes through a link.
pub fn write_seat_files(seat_home: &Path, profile: &SeatProfile) -> Result<(), String> {
    check_seat_home_dir(seat_home)?;
    std::fs::create_dir_all(seat_home)
        .map_err(|e| format!("containment: cannot create {}: {e}", seat_home.display()))?;
    check_seat_home_dir(seat_home)?;
    match profile.key {
        "kimi" => write_readonly(&seat_home.join(crate::kimi::RO_AGENT_FILE), KIMI_RO_AGENT),
        "qwen" => {
            let path = seat_home.join("settings.json");
            // Only a missing file starts empty; any other read failure stops, so a file the daemon cannot read is
            // never replaced by one that has lost the CLI's login settings.
            let (mut doc, mode): (serde_json::Value, Option<u32>) = match read_regular(&path)? {
                Some((bytes, mode)) => (
                    serde_json::from_slice(&bytes)
                        .map_err(|e| format!("containment: {} is not JSON: {e}", path.display()))?,
                    Some(mode),
                ),
                None => (serde_json::json!({}), None),
            };
            let (env_key, base_url) = crate::qwen::plan_auth(None)?;
            let obj = doc
                .as_object_mut()
                .ok_or_else(|| format!("containment: {} is not a JSON object", path.display()))?;
            // Merge, not replace: only the openai entry whose id is the pinned model is the daemon's. Other
            // providers, other openai entries and any provider-level settings written at login are kept.
            let ours = serde_json::json!({
                "id": crate::qwen::DEFAULT_QWEN_MODEL,
                "baseUrl": base_url,
                "envKey": env_key,
            });
            let providers = obj
                .entry("modelProviders")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .ok_or_else(|| {
                    "containment: settings.json `modelProviders` is not an object".to_string()
                })?;
            let openai = providers
                .entry("openai")
                .or_insert_with(|| serde_json::json!([]))
                .as_array_mut()
                .ok_or_else(|| {
                    "containment: settings.json `modelProviders.openai` is not an array".to_string()
                })?;
            match openai.iter_mut().find(|e| {
                e.get("id").and_then(serde_json::Value::as_str)
                    == Some(crate::qwen::DEFAULT_QWEN_MODEL)
            }) {
                Some(entry) => *entry = ours,
                None => openai.push(ours),
            }
            let agents = obj.entry("agents").or_insert_with(|| serde_json::json!({}));
            agents
                .as_object_mut()
                .ok_or_else(|| "containment: settings.json `agents` is not an object".to_string())?
                .insert(
                    "crossSessionMessaging".into(),
                    serde_json::Value::Bool(false),
                );
            let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
            replace_file(&path, (text + "\n").as_bytes(), mode)
        }
        "mistral" => {
            let path = seat_home.join("config.toml");
            let (old, mode) = match read_regular(&path)? {
                Some((bytes, mode)) => (
                    String::from_utf8(bytes)
                        .map_err(|e| format!("containment: cannot read {}: {e}", path.display()))?,
                    Some(mode),
                ),
                None => (String::new(), None),
            };
            replace_file(
                &path,
                set_top_level_toml_bool(&old, "enable_telemetry", false).as_bytes(),
                mode,
            )
        }
        other => Err(format!(
            "containment: seat {other} has no daemon-written files"
        )),
    }
}

/// Write a file the CLI must never change: a fresh 0444 temp file renamed over the old one, so an existing 0444 copy
/// needs no chmod and a planted symlink is refused, never followed.
fn write_readonly(path: &Path, text: &str) -> Result<(), String> {
    lstat_regular(path)?;
    replace_file(path, text.as_bytes(), Some(0o444))
}

/// Refuse a seat home that is a symlink or not a directory. A missing home is fine (it is created next).
fn check_seat_home_dir(seat_home: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(seat_home) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => Err(format!(
            "containment: seat home {} is a symlink or not a directory",
            seat_home.display()
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "containment: cannot stat {}: {e}",
            seat_home.display()
        )),
    }
}

/// `symlink_metadata` of a daemon-owned file: `None` when missing, an error when it is a symlink or anything other
/// than a regular file. A non-regular file keeps the "cannot read" prefix (the daemon never replaces what it could
/// not read).
fn lstat_regular(path: &Path) -> Result<Option<std::fs::Metadata>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(format!(
            "containment: {} is not a regular file",
            path.display()
        )),
        Ok(m) if !m.is_file() => Err(format!(
            "containment: cannot read {}: {} is not a regular file",
            path.display(),
            path.display()
        )),
        Ok(m) => Ok(Some(m)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("containment: cannot read {}: {e}", path.display())),
    }
}

/// Read a daemon-owned file only after the symlink check passes: its bytes and permission bits, or `None` when it is
/// missing. The opened file must be the same inode `symlink_metadata` saw, so a link swapped in between is refused.
fn read_regular(path: &Path) -> Result<Option<(Vec<u8>, u32)>, String> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let Some(before) = lstat_regular(path)? else {
        return Ok(None);
    };
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("containment: cannot read {}: {e}", path.display()))?;
    let opened = file
        .metadata()
        .map_err(|e| format!("containment: cannot read {}: {e}", path.display()))?;
    if !opened.is_file() || opened.dev() != before.dev() || opened.ino() != before.ino() {
        return Err(format!(
            "containment: {} is not a regular file",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("containment: cannot read {}: {e}", path.display()))?;
    Ok(Some((bytes, before.permissions().mode() & 0o7777)))
}

/// Process-wide counter that, with the pid, makes each temp file name unique.
static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Replace `path` without writing in place: write a fresh temp file in the same directory (`create_new` refuses any
/// existing path, a symlink included), set `mode` on it, then rename it over `path`. The temp file is removed on any
/// error.
fn replace_file(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let dir = path
        .parent()
        .ok_or_else(|| format!("containment: {} has no parent", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("containment: {} has no file name", path.display()))?
        .to_string_lossy();
    let n = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.tmp.{}.{n}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| format!("containment: cannot write {}: {e}", tmp.display()))?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|e| format!("containment: cannot write {}: {e}", tmp.display()))?;
        if let Some(mode) = mode {
            file.set_permissions(std::fs::Permissions::from_mode(mode))
                .map_err(|e| format!("containment: chmod {}: {e}", tmp.display()))?;
        }
        drop(file);
        std::fs::rename(&tmp, path).map_err(|e| {
            format!(
                "containment: cannot write {}: rename {}: {e}",
                path.display(),
                tmp.display()
            )
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Set `key = <value>` in the top-level table of a TOML text (before the first `[table]` header), replacing an
/// existing top-level assignment. Line based on purpose: the build adds no TOML crate, and only one known key is set.
fn set_top_level_toml_bool(text: &str, key: &str, value: bool) -> String {
    let line = format!("{key} = {value}");
    let mut out: Vec<String> = Vec::new();
    let mut in_top = true;
    let mut done = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with('[') {
            if in_top && !done {
                out.push(line.clone());
                done = true;
            }
            in_top = false;
        }
        let is_key = t
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='));
        if in_top && is_key {
            if !done {
                out.push(line.clone());
                done = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !done {
        out.insert(0, line);
    }
    out.join("\n") + "\n"
}

/// The child env: `base` minus the profile's `env_remove`, with HOME and the vendor home var set to the seat home, then
/// `env_add` applied last.
pub fn child_env(
    base: &[(String, String)],
    profile: &SeatProfile,
    seat_home: &Path,
    env_add: &[(String, String)],
) -> Result<Vec<(String, String)>, String> {
    let home = seat_home
        .to_str()
        .ok_or_else(|| {
            format!(
                "containment: seat home {} is not UTF-8",
                seat_home.display()
            )
        })?
        .to_string();
    let mut overrides: Vec<(String, String)> = vec![("HOME".into(), home.clone())];
    if let Some(var) = profile.home_env_var {
        overrides.push((var.to_string(), home));
    }
    overrides.extend(env_add.iter().cloned());

    let mut env: Vec<(String, String)> = base
        .iter()
        .filter(|(k, _)| !profile.env_remove.iter().any(|p| matches_pattern(k, p)))
        .filter(|(k, _)| !overrides.iter().any(|(o, _)| o == k))
        .cloned()
        .collect();
    // Overrides last and deduplicated, so a later env_add entry wins over HOME or an earlier one.
    for (k, v) in overrides {
        env.retain(|(e, _)| *e != k);
        env.push((k, v));
    }
    Ok(env)
}

/// The seatbelt profile for one dispatch (deny file-write*, then allow the seat home, output dir, tmp and dev paths).
/// Generalized from mcp-bridge/src/agy.rs SANDBOX_PROFILE_TEMPLATE. Paths are canonicalized when they exist, because
/// seatbelt matches the resolved path (/var is /private/var on macOS).
pub fn render_seatbelt_profile(
    seat_home: &Path,
    output_dir: &Path,
    tmpdir: &Path,
) -> Result<String, String> {
    let quoted = |p: &Path| -> Result<String, String> {
        let real = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let s = real
            .to_str()
            .ok_or_else(|| format!("containment: {} is not UTF-8", p.display()))?;
        if s.contains('"') || s.contains('\\') || !s.starts_with('/') {
            return Err(format!(
                "containment: refusing to put {s:?} in a seatbelt profile"
            ));
        }
        Ok(s.to_string())
    };
    // Exactly the BACKEND_STRUCTURE section 5 profile text, line for line (GAUNTLET: a leading comment line here was a
    // deviation from the contract that two blind suites caught).
    Ok(format!(
        "(version 1)\n\
         (allow default)\n\
         (deny file-write*)\n\
         (allow file-write* (subpath \"{}\"))\n\
         (allow file-write* (subpath \"{}\"))\n\
         (allow file-write* (subpath \"{}\"))\n\
         (allow file-write* (subpath \"/private/var/folders\"))\n\
         (allow file-write* (subpath \"/private/tmp\"))\n\
         (allow file-write* (literal \"/dev/null\") (literal \"/dev/stdout\") (literal \"/dev/stderr\") (literal \"/dev/dtracehelper\") (literal \"/dev/tty\"))\n",
        quoted(seat_home)?,
        quoted(output_dir)?,
        quoted(tmpdir)?
    ))
}

fn git(cwd: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("containment: cannot run git: {e}"))?;
    if let Some(bytes) = stdin {
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(bytes)
            .map_err(|e| format!("containment: git stdin: {e}"))?;
    }
    drop(child.stdin.take());
    let out = child
        .wait_with_output()
        .map_err(|e| format!("containment: git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "containment: git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

/// Status plus content hash of every changed, staged or untracked path. Two snapshots differ wherever the run wrote.
pub fn snapshot_repo(cwd: &Path) -> Result<RepoSnapshot, String> {
    let raw = git(
        cwd,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        None,
    )?;
    // -z entries are "XY path\0"; a rename or copy carries its source as one more NUL-terminated field.
    let mut statuses: Vec<(String, String)> = Vec::new();
    let mut fields = raw.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        let text = String::from_utf8_lossy(field);
        if text.len() < 4 {
            continue;
        }
        let (xy, path) = (text[..2].to_string(), text[3..].to_string());
        if xy.starts_with('R') || xy.starts_with('C') {
            fields.next();
        }
        statuses.push((path, xy));
    }
    let present: Vec<&str> = statuses
        .iter()
        .filter(|(p, _)| cwd.join(p).is_file())
        .map(|(p, _)| p.as_str())
        .collect();
    let hashes_raw = if present.is_empty() {
        Vec::new()
    } else {
        git(
            cwd,
            &["hash-object", "--stdin-paths"],
            Some((present.join("\n") + "\n").as_bytes()),
        )?
    };
    let hashes_text = String::from_utf8_lossy(&hashes_raw);
    let mut hashes = hashes_text.lines();
    let mut entries = std::collections::BTreeMap::new();
    for (path, xy) in &statuses {
        let hash = if present.contains(&path.as_str()) {
            hashes.next().unwrap_or("?").to_string()
        } else {
            "absent".to_string()
        };
        entries.insert(path.clone(), format!("{xy}:{hash}"));
    }
    Ok(RepoSnapshot { entries })
}

/// Paths that are NEW or CHANGED in `after` compared with `before`. A non-empty result fails the call as
/// `read_only_violation: <paths>`; nothing is reverted.
///
/// A path in `before` that is gone from `after` changed too (the run deleted or restored a pre-existing edit).
pub fn audit_repo(before: &RepoSnapshot, after: &RepoSnapshot) -> Result<Vec<String>, String> {
    let mut changed: Vec<String> = after
        .entries
        .iter()
        .filter(|(p, v)| before.entries.get(*p) != Some(*v))
        .map(|(p, _)| p.clone())
        .collect();
    changed.extend(
        before
            .entries
            .keys()
            .filter(|p| !after.entries.contains_key(*p))
            .cloned(),
    );
    changed.sort();
    Ok(changed)
}

/// `homes/<seat>/credentials.env` must exist with mode 0600.
pub fn check_credentials_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("credentials_missing: {}: {e}", path.display()))?;
    let mode = meta.permissions().mode() & 0o777;
    if mode == 0o600 {
        Ok(())
    } else {
        Err(format!(
            "credentials_mode: {} is {mode:04o}, must be 0600",
            path.display()
        ))
    }
}

/// Names of metered-API keys present in an env that a seat must never see.
pub fn scan_stray_keys(env: &[(String, String)]) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = env
        .iter()
        .filter(|(k, _)| SEAT_CREDENTIAL_SCRUB.iter().any(|p| matches_pattern(k, p)))
        .map(|(k, _)| k.clone())
        .collect();
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A seat profile with the containment fields T-201 gives the new seats.
    fn seat(key: &'static str, home_var: &'static str) -> SeatProfile {
        SeatProfile {
            key,
            home_env_var: Some(home_var),
            env_remove: SEAT_CREDENTIAL_SCRUB,
            ..crate::seat_profile::seat_profile("codex").unwrap().clone()
        }
    }

    /// Scrub, then add: inherited vendor keys are gone, HOME and the vendor home point at the seat home, and the seat's
    /// own env_add entry survives even when it matches a scrub pattern.
    /// RED IF: a scrubbed key reaches the child, env_add runs before the scrub, or HOME is not relocated.
    #[test]
    fn child_env_scrubs_then_adds() {
        let base = kv(&[
            ("PATH", "/usr/bin"),
            ("HOME", "/Users/me"),
            ("MISTRAL_API_KEY", "metered"),
            ("OPENAI_API_KEY", "metered"),
            ("QWENCLOUD_TOKEN_PLAN_API_KEY", "inherited"),
            ("KIMI_FOO", "x"),
            ("OPENAI_BASE_URL", "https://elsewhere"),
            ("DASHSCOPE_API_KEY", "inherited"),
            ("BAILIAN_TOKEN", "inherited"),
            ("MOONSHOT_API_KEY", "inherited"),
        ]);
        let env = child_env(
            &base,
            &seat("mistral", "VIBE_HOME"),
            Path::new("/th/homes/mistral"),
            &kv(&[("VIBE_CLI", "python")]),
        )
        .unwrap();
        let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("MISTRAL_API_KEY"), None);
        assert_eq!(get("OPENAI_API_KEY"), None);
        assert_eq!(get("QWENCLOUD_TOKEN_PLAN_API_KEY"), None);
        assert_eq!(get("KIMI_FOO"), None);
        // Every family on the scrub list, so dropping any pattern from SEAT_CREDENTIAL_SCRUB turns this red.
        for k in [
            "OPENAI_BASE_URL",
            "DASHSCOPE_API_KEY",
            "BAILIAN_TOKEN",
            "MOONSHOT_API_KEY",
        ] {
            assert_eq!(get(k), None, "{k} reached the child");
        }
        assert_eq!(get("VIBE_CLI"), Some("python"));
        assert_eq!(get("HOME"), Some("/th/homes/mistral"));
        assert_eq!(get("VIBE_HOME"), Some("/th/homes/mistral"));
        assert_eq!(get("PATH"), Some("/usr/bin"));
        assert_eq!(env.iter().filter(|(k, _)| k == "HOME").count(), 1);

        // The Qwen plan key is added back AFTER the scrub, so it is the only credential left.
        let qwen = child_env(
            &base,
            &seat("qwen", "QWEN_HOME"),
            Path::new("/th/homes/qwen"),
            &kv(&[("QWENCLOUD_TOKEN_PLAN_API_KEY", "sk-sp-plan")]),
        )
        .unwrap();
        assert_eq!(
            scan_stray_keys(&qwen).unwrap(),
            ["QWENCLOUD_TOKEN_PLAN_API_KEY"]
        );
        assert!(qwen.contains(&("QWENCLOUD_TOKEN_PLAN_API_KEY".into(), "sk-sp-plan".into())));
    }

    /// RED IF: a mode other than 0600 passes, or a missing file passes.
    #[test]
    fn credentials_file_must_be_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("credentials.env");
        std::fs::write(&f, "QWENCLOUD_TOKEN_PLAN_API_KEY=sk-sp-x\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            check_credentials_file(&f).unwrap_err(),
            format!("credentials_mode: {} is 0644, must be 0600", f.display())
        );
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(check_credentials_file(&f), Ok(()));
        assert!(
            check_credentials_file(&dir.path().join("nope"))
                .unwrap_err()
                .starts_with("credentials_missing:")
        );
    }

    /// RED IF: an unreadable seat config is silently treated as absent and overwritten.
    #[test]
    fn seat_files_refuse_an_unreadable_config() {
        // A path that exists but cannot be read as a file (a directory here) is an error, never "start empty": the
        // daemon must not replace a config it could not read.
        let home = tempfile::tempdir().unwrap();
        for (key, var, file) in [
            ("qwen", "QWEN_HOME", "settings.json"),
            ("mistral", "VIBE_HOME", "config.toml"),
        ] {
            let dir = home.path().join(key);
            std::fs::create_dir_all(dir.join(file)).unwrap();
            let err = write_seat_files(&dir, &seat(key, var)).unwrap_err();
            assert!(err.starts_with("containment: cannot read"), "{key}: {err}");
        }
    }

    /// The daemon-written files have the shapes BACKEND_STRUCTURE section 5 fixes, and Qwen/Mistral files keep what
    /// the CLI had written.
    /// RED IF: ro-agent.md loses its tool limits or 0444 mode, Qwen settings stop pinning the plan endpoint and model,
    /// a merge drops a key or provider entry the CLI wrote, or a rewrite duplicates the daemon's entry.
    #[test]
    fn seat_files_are_written_and_merged() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let kimi = home.path().join("kimi");
        write_seat_files(&kimi, &seat("kimi", "KIMI_CODE_HOME")).unwrap();
        write_seat_files(&kimi, &seat("kimi", "KIMI_CODE_HOME")).unwrap(); // rewriting a 0444 file works
        let ro = kimi.join("ro-agent.md");
        let text = std::fs::read_to_string(&ro).unwrap();
        assert!(
            text.contains("tools: [Read, Grep, Glob]")
                && text.contains("disallowedTools: [Agent, AgentSwarm]")
        );
        assert_eq!(
            std::fs::metadata(&ro).unwrap().permissions().mode() & 0o777,
            0o444
        );

        let qwen = home.path().join("qwen");
        std::fs::create_dir_all(&qwen).unwrap();
        std::fs::write(
            qwen.join("settings.json"),
            r#"{"security":{"auth":{"selectedType":"openai"}},"modelProviders":{"anthropic":[{"id":"keep-me"}],"openai":[{"id":"other-model","baseUrl":"https://x"},{"id":"qwen-stale","envKey":"OLD"}]}}"#,
        )
        .unwrap();
        write_seat_files(&qwen, &seat("qwen", "QWEN_HOME")).unwrap();
        let doc: serde_json::Value =
            serde_json::from_slice(&std::fs::read(qwen.join("settings.json")).unwrap()).unwrap();
        // Login-written providers survive the merge; the daemon's entry is added once.
        assert_eq!(doc["modelProviders"]["anthropic"][0]["id"], "keep-me");
        assert_eq!(doc["modelProviders"]["openai"][0]["id"], "other-model");
        let openai = doc["modelProviders"]["openai"].as_array().unwrap();
        let provider = openai
            .iter()
            .find(|e| e["id"] == crate::qwen::DEFAULT_QWEN_MODEL)
            .expect("the daemon's entry");
        // Writing again replaces the daemon's entry in place, never duplicates it.
        write_seat_files(&qwen, &seat("qwen", "QWEN_HOME")).unwrap();
        let again: serde_json::Value =
            serde_json::from_slice(&std::fs::read(qwen.join("settings.json")).unwrap()).unwrap();
        assert_eq!(
            again["modelProviders"]["openai"].as_array().unwrap().len(),
            openai.len()
        );
        assert_eq!(
            provider["baseUrl"],
            "https://token-plan.maas.qwencloudapi.com/compatible-mode/v1"
        );
        assert_eq!(provider["envKey"], "QWENCLOUD_TOKEN_PLAN_API_KEY");
        assert_eq!(provider["id"], crate::qwen::DEFAULT_QWEN_MODEL);
        assert_eq!(doc["agents"]["crossSessionMessaging"], false);
        assert_eq!(
            doc["security"]["auth"]["selectedType"], "openai",
            "the CLI's own key survives"
        );

        let mistral = home.path().join("mistral");
        std::fs::create_dir_all(&mistral).unwrap();
        std::fs::write(
            mistral.join("config.toml"),
            "active_model = \"devstral\"\nenable_telemetry = true\n[ui]\nenable_telemetry = true\n",
        )
        .unwrap();
        write_seat_files(&mistral, &seat("mistral", "VIBE_HOME")).unwrap();
        let toml = std::fs::read_to_string(mistral.join("config.toml")).unwrap();
        assert_eq!(
            toml,
            "active_model = \"devstral\"\nenable_telemetry = false\n[ui]\nenable_telemetry = true\n"
        );
        assert_eq!(
            set_top_level_toml_bool("[a]\nx = 1\n", "enable_telemetry", false),
            "enable_telemetry = false\n[a]\nx = 1\n"
        );
    }

    /// Plant a symlink at `file` inside a seat home, pointing at a sentinel OUTSIDE it, and run write_seat_files.
    fn assert_symlink_refused(key: &'static str, var: &'static str, file: &str) {
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("repo-file.rs");
        std::fs::write(&sentinel, b"fn main() {}\n").unwrap();
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(key);
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&sentinel, dir.join(file)).unwrap();
        let err = write_seat_files(&dir, &seat(key, var)).unwrap_err();
        assert!(err.contains("is not a regular file"), "{key}: {err}");
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"fn main() {}\n");
        assert!(
            std::fs::symlink_metadata(dir.join(file))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    /// RED IF: a symlink planted at ro-agent.md makes the daemon chmod or overwrite the file it points to.
    #[test]
    fn kimi_ro_agent_symlink_is_refused() {
        assert_symlink_refused("kimi", "KIMI_CODE_HOME", crate::kimi::RO_AGENT_FILE);
    }

    /// RED IF: a symlink planted at settings.json is followed on read or write.
    #[test]
    fn qwen_settings_symlink_is_refused() {
        assert_symlink_refused("qwen", "QWEN_HOME", "settings.json");
    }

    /// RED IF: a symlink planted at config.toml is followed on read or write.
    #[test]
    fn mistral_config_symlink_is_refused() {
        assert_symlink_refused("mistral", "VIBE_HOME", "config.toml");
    }

    /// RED IF: a seat home that is itself a symlink is followed, so the daemon writes its files elsewhere.
    #[test]
    fn symlinked_seat_home_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        for (key, var) in [
            ("kimi", "KIMI_CODE_HOME"),
            ("qwen", "QWEN_HOME"),
            ("mistral", "VIBE_HOME"),
        ] {
            let link = home.path().join(key);
            std::os::unix::fs::symlink(outside.path(), &link).unwrap();
            let err = write_seat_files(&link, &seat(key, var)).unwrap_err();
            assert!(err.contains("is a symlink"), "{key}: {err}");
        }
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    /// RED IF: a rewrite writes in place (a hard link to the old inode sees the new bytes), leaves a temp file behind,
    /// or loses an existing file's mode.
    #[test]
    fn seat_files_replace_via_rename_and_keep_mode() {
        use std::os::unix::fs::PermissionsExt;
        let outside = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("mistral");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config.toml");
        std::fs::write(&cfg, "enable_telemetry = true\n").unwrap();
        std::fs::set_permissions(&cfg, std::fs::Permissions::from_mode(0o600)).unwrap();
        // A hard link to the old inode: an in-place write changes it, a rename leaves it alone.
        let old_inode = outside.path().join("old-inode");
        std::fs::hard_link(&cfg, &old_inode).unwrap();
        write_seat_files(&dir, &seat("mistral", "VIBE_HOME")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            "enable_telemetry = false\n"
        );
        assert_eq!(
            std::fs::read_to_string(&old_inode).unwrap(),
            "enable_telemetry = true\n"
        );
        assert_eq!(
            std::fs::metadata(&cfg).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["config.toml"]);
    }

    /// The profile allows exactly the seat home, the output dir and tmp, and refuses a path that could break out of
    /// its string.
    #[test]
    fn seatbelt_profile_shape() {
        let p = render_seatbelt_profile(
            Path::new("/th/homes/kimi"),
            Path::new("/th/out/1"),
            Path::new("/th/tmp"),
        )
        .unwrap();
        assert!(p.contains("(deny file-write*)"));
        for allowed in [
            "/th/homes/kimi",
            "/th/out/1",
            "/th/tmp",
            "/private/var/folders",
            "/private/tmp",
        ] {
            assert!(p.contains(&format!("(subpath \"{allowed}\")")), "{allowed}");
        }
        assert!(
            render_seatbelt_profile(
                Path::new("/x\")(allow default"),
                Path::new("/o"),
                Path::new("/t")
            )
            .is_err()
        );
        assert!(seat_home(Path::new("/th"), "../etc").is_err());
    }
}
