// GAUNTLET tiebreaker suite for T-104, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: The spec does not provide the pinned Qwen model, endpoint, or plan environment variable values, so this suite verifies their required placement without inventing values.
// SPEC_AMBIGUITY: The public contract conflicts on planted daemon-file symlinks: one clause says rename replaces the symlink, while another says a planted symlink is refused.
// SPEC_AMBIGUITY: Exact error text is not specified for invalid seat keys, unsafe credentials permissions, filesystem failures, or stray-key detection.

use mcp_bridge::containment::{
    RepoSnapshot, SEAT_CREDENTIAL_SCRUB, audit_repo, check_credentials_file, child_env,
    render_seatbelt_profile, scan_stray_keys, seat_home, snapshot_repo, write_seat_files,
};
use mcp_bridge::seat_profile::seat_profile;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

fn profile(name: &str) -> &'static mcp_bridge::seat_profile::SeatProfile {
    seat_profile(name).unwrap_or_else(|| panic!("missing SeatProfile row for {name}"))
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git must execute");

    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn snapshot(entries: &[(&str, &str)]) -> RepoSnapshot {
    RepoSnapshot {
        entries: entries
            .iter()
            .map(|(path, hash)| ((*path).to_owned(), (*hash).to_owned()))
            .collect(),
    }
}

#[test]
fn req_030_seat_home_is_exactly_under_homes() {
    // REQ-030: a seat home is $TRIUMVIRATE_HOME/homes/<seat>.
    let root = Path::new("/tmp/triumvirate-contract-root");

    assert_eq!(
        seat_home(root, "kimi"),
        Ok(root.join("homes").join("kimi"))
    );
    assert_eq!(
        seat_home(root, "mistral"),
        Ok(root.join("homes").join("mistral"))
    );
    assert_eq!(
        seat_home(root, "qwen"),
        Ok(root.join("homes").join("qwen"))
    );
}

#[test]
fn req_030_seat_key_cannot_escape_homes() {
    // REQ-030: a seat key used as a path component cannot address anything outside homes.
    let root = Path::new("/tmp/triumvirate-contract-root");

    for invalid in [
        "",
        ".",
        "..",
        "../kimi",
        "kimi/..",
        "nested/kimi",
        "/absolute",
        "kimi\u{0}escape",
    ] {
        assert!(
            seat_home(root, invalid).is_err(),
            "unsafe seat key was accepted: {invalid:?}"
        );
    }
}

#[test]
fn req_033_credential_scrub_contract_is_exact() {
    // REQ-033: every seat removes the exact shared credential and endpoint denylist.
    assert_eq!(
        SEAT_CREDENTIAL_SCRUB,
        &[
            "MISTRAL_API_KEY",
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "DASHSCOPE_*",
            "BAILIAN_*",
            "QWENCLOUD_*",
            "KIMI_*",
            "MOONSHOT_*",
        ]
    );
}

#[test]
fn req_030_req_033_child_env_scrubs_then_adds_and_relocates_home() {
    // REQ-030 and REQ-033: scrub inherited credentials, set both homes, then apply env_add last.
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("mistral-home");
    let base = vec![
        ("KEEP".to_owned(), "yes".to_owned()),
        ("HOME".to_owned(), "/real/home".to_owned()),
        ("VIBE_HOME".to_owned(), "/real/vibe".to_owned()),
        ("MISTRAL_API_KEY".to_owned(), "inherited".to_owned()),
        ("OPENAI_API_KEY".to_owned(), "inherited".to_owned()),
        ("OPENAI_BASE_URL".to_owned(), "https://wrong.invalid".to_owned()),
        ("DASHSCOPE_API_KEY".to_owned(), "inherited".to_owned()),
        ("BAILIAN_TOKEN".to_owned(), "inherited".to_owned()),
        ("QWENCLOUD_PLAN_KEY".to_owned(), "inherited".to_owned()),
        ("KIMI_DISABLE_TELEMETRY".to_owned(), "0".to_owned()),
        ("MOONSHOT_API_KEY".to_owned(), "inherited".to_owned()),
    ];
    let env_add = vec![
        ("VIBE_CLI".to_owned(), "python".to_owned()),
        ("MISTRAL_API_KEY".to_owned(), "seat-owned".to_owned()),
    ];

    let actual = child_env(&base, profile("mistral"), &home, &env_add)
        .expect("child environment must be constructed");
    let actual: BTreeMap<_, _> = actual.into_iter().collect();

    let expected = BTreeMap::from([
        ("HOME".to_owned(), home.display().to_string()),
        ("KEEP".to_owned(), "yes".to_owned()),
        (
            "MISTRAL_API_KEY".to_owned(),
            "seat-owned".to_owned(),
        ),
        ("VIBE_CLI".to_owned(), "python".to_owned()),
        ("VIBE_HOME".to_owned(), home.display().to_string()),
    ]);

    assert_eq!(actual, expected);
}

#[test]
fn req_033_prefix_patterns_remove_every_matching_variable() {
    // REQ-033: a trailing star removes every inherited variable with that prefix.
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("qwen-home");
    let base = vec![
        ("DASHSCOPE_ONE".to_owned(), "secret".to_owned()),
        ("DASHSCOPE_TWO".to_owned(), "secret".to_owned()),
        ("BAILIAN_ONE".to_owned(), "secret".to_owned()),
        ("QWENCLOUD_ONE".to_owned(), "secret".to_owned()),
        ("KIMI_ONE".to_owned(), "secret".to_owned()),
        ("MOONSHOT_ONE".to_owned(), "secret".to_owned()),
        ("UNRELATED".to_owned(), "kept".to_owned()),
    ];

    let actual =
        child_env(&base, profile("qwen"), &home, &[]).expect("child environment must be built");
    let names: BTreeSet<_> = actual.into_iter().map(|(name, _)| name).collect();

    assert_eq!(
        names,
        BTreeSet::from([
            "HOME".to_owned(),
            "QWEN_HOME".to_owned(),
            "UNRELATED".to_owned(),
        ])
    );
}

#[test]
fn req_021_req_031_kimi_agent_file_is_exact_and_read_only() {
    // REQ-021 and REQ-031: Kimi receives the exact daemon-owned read-only agent file with mode 0444.
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("kimi");

    write_seat_files(&home, profile("kimi")).expect("Kimi seat files must be written");

    let path = home.join("ro-agent.md");
    assert_eq!(
        fs::read_to_string(&path).expect("read ro-agent.md"),
        concat!(
            "---\n",
            "name: triumvirate-readonly\n",
            "description: Read-only reviewer that answers questions and never writes files.\n",
            "tools: [Read, Grep, Glob]\n",
            "disallowedTools: [Agent, AgentSwarm]\n",
            "subagents: []\n",
            "---\n",
            "Read-only reviewer: never write files.\n",
        )
    );

    #[cfg(unix)]
    assert_eq!(
        fs::metadata(path).expect("ro-agent.md metadata").mode() & 0o777,
        0o444
    );
}

#[test]
fn req_031_kimi_agent_file_is_rewritten_from_daemon_truth() {
    // REQ-031: Kimi ro-agent.md is wholly daemon-owned and is rewritten.
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("kimi");
    fs::create_dir_all(&home).expect("create seat home");
    fs::write(home.join("ro-agent.md"), "hostile override\n").expect("seed hostile file");

    write_seat_files(&home, profile("kimi")).expect("Kimi seat files must be rewritten");

    let contents = fs::read_to_string(home.join("ro-agent.md")).expect("read ro-agent.md");
    assert_eq!(
        contents,
        concat!(
            "---\n",
            "name: triumvirate-readonly\n",
            "description: Read-only reviewer that answers questions and never writes files.\n",
            "tools: [Read, Grep, Glob]\n",
            "disallowedTools: [Agent, AgentSwarm]\n",
            "subagents: []\n",
            "---\n",
            "Read-only reviewer: never write files.\n",
        )
    );
}

#[test]
fn req_031_qwen_settings_are_merged_and_containment_keys_are_set() {
    // REQ-031: Qwen settings preserve login state while setting provider and cross-session containment keys.
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("qwen");
    fs::create_dir_all(&home).expect("create seat home");
    fs::write(
        home.join("settings.json"),
        r#"{"login":{"refreshToken":"preserve-me"},"agents":{"operatorChoice":true}}"#,
    )
    .expect("seed settings.json");

    write_seat_files(&home, profile("qwen")).expect("Qwen seat files must be written");

    let document: serde_json::Value = serde_json::from_slice(
        &fs::read(home.join("settings.json")).expect("read settings.json"),
    )
    .expect("settings.json must remain valid JSON");

    assert_eq!(document["login"]["refreshToken"], "preserve-me");
    assert_eq!(document["agents"]["operatorChoice"], true);
    assert_eq!(document["agents"]["crossSessionMessaging"], false);

    let providers = document["modelProviders"]["openai"]
        .as_array()
        .expect("modelProviders.openai must be an array");
    assert_eq!(providers.len(), 1);

    let provider = providers[0]
        .as_object()
        .expect("the pinned provider must be an object");
    assert_eq!(
        provider.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "baseUrl".to_owned(),
            "envKey".to_owned(),
            "id".to_owned(),
        ])
    );

    for key in ["id", "baseUrl", "envKey"] {
        assert!(
            provider[key]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "provider field {key} must be a nonempty string"
        );
    }
}

#[test]
fn req_031_mistral_config_is_merged_and_disables_telemetry() {
    // REQ-031: Mistral config preserves unrelated login configuration and sets top-level enable_telemetry=false.
    let temp = tempfile::tempdir().expect("temporary directory");
    let home = temp.path().join("mistral");
    fs::create_dir_all(&home).expect("create seat home");
    fs::write(
        home.join("config.toml"),
        "theme = \"dark\"\nenable_telemetry = true\n\n[login]\nregion = \"us\"\n",
    )
    .expect("seed config.toml");

    write_seat_files(&home, profile("mistral")).expect("Mistral seat files must be written");

    assert_eq!(
        fs::read_to_string(home.join("config.toml")).expect("read config.toml"),
        "theme = \"dark\"\nenable_telemetry = false\n\n[login]\nregion = \"us\"\n"
    );
}

#[test]
fn req_035_seatbelt_profile_has_the_exact_write_policy() {
    // REQ-035: the rendered seatbelt denies writes globally and allows only the named writable locations.
    let temp = tempfile::tempdir().expect("temporary directory");
    let seat_home = temp.path().join("seat");
    let output_dir = temp.path().join("output");
    let tmpdir = temp.path().join("tmp");
    fs::create_dir_all(&seat_home).expect("create seat home");
    fs::create_dir_all(&output_dir).expect("create output directory");
    fs::create_dir_all(&tmpdir).expect("create temp directory");

    let actual = render_seatbelt_profile(&seat_home, &output_dir, &tmpdir)
        .expect("seatbelt profile must render");

    let expected = format!(
        concat!(
            "(version 1)\n",
            "(allow default)\n",
            "(deny file-write*)\n",
            "(allow file-write* (subpath \"{}\"))\n",
            "(allow file-write* (subpath \"{}\"))\n",
            "(allow file-write* (subpath \"{}\"))\n",
            "(allow file-write* (subpath \"/private/var/folders\"))\n",
            "(allow file-write* (subpath \"/private/tmp\"))\n",
            "(allow file-write* (literal \"/dev/null\") (literal \"/dev/stdout\") ",
            "(literal \"/dev/stderr\") (literal \"/dev/dtracehelper\") ",
            "(literal \"/dev/tty\"))\n",
        ),
        fs::canonicalize(&seat_home)
            .expect("canonical seat home")
            .display(),
        fs::canonicalize(&output_dir)
            .expect("canonical output directory")
            .display(),
        fs::canonicalize(&tmpdir)
            .expect("canonical temp directory")
            .display(),
    );

    assert_eq!(actual, expected);
}

#[test]
fn req_035_audit_reports_new_changed_and_removed_paths_exactly() {
    // REQ-035: audit returns every new, changed, and removed path in deterministic order.
    let before = snapshot(&[
        ("changed.txt", "old"),
        ("removed.txt", "present"),
        ("same.txt", "same"),
    ]);
    let after = snapshot(&[
        ("changed.txt", "new"),
        ("new.txt", "created"),
        ("same.txt", "same"),
    ]);

    assert_eq!(
        audit_repo(&before, &after),
        Ok(vec![
            "changed.txt".to_owned(),
            "new.txt".to_owned(),
            "removed.txt".to_owned(),
        ])
    );
}

#[test]
fn req_035_audit_ignores_preexisting_unchanged_dirt() {
    // REQ-035 edge case: identical dirty entries in both snapshots are not read-only violations.
    let before = snapshot(&[
        ("tracked-dirty.txt", "dirty-hash"),
        ("untracked.txt", "untracked-hash"),
    ]);
    let after = snapshot(&[
        ("tracked-dirty.txt", "dirty-hash"),
        ("untracked.txt", "untracked-hash"),
    ]);

    assert_eq!(audit_repo(&before, &after), Ok(Vec::new()));
}

#[test]
fn req_035_real_git_audit_finds_only_run_deltas_and_reverts_nothing() {
    // REQ-035 reality test: new tracked and untracked writes are reported while all prior and new changes remain.
    let temp = tempfile::tempdir().expect("temporary directory");
    let repo = temp.path();

    run_git(repo, &["init", "--quiet"]);
    run_git(repo, &["config", "user.name", "Gauntlet"]);
    run_git(repo, &["config", "user.email", "gauntlet@example.invalid"]);

    fs::write(repo.join("tracked.txt"), "committed\n").expect("write tracked file");
    run_git(repo, &["add", "tracked.txt"]);
    run_git(repo, &["commit", "--quiet", "-m", "fixture"]);

    fs::write(repo.join("tracked.txt"), "preexisting edit\n").expect("write preexisting edit");
    fs::write(repo.join("preexisting-untracked.txt"), "preexisting\n")
        .expect("write preexisting untracked file");

    let before = snapshot_repo(repo).expect("snapshot before run");

    fs::write(
        repo.join("tracked.txt"),
        "preexisting edit plus run edit\n",
    )
    .expect("write new tracked edit");
    fs::write(repo.join("new-untracked.txt"), "new\n").expect("write new untracked file");

    let after = snapshot_repo(repo).expect("snapshot after run");

    assert_eq!(
        audit_repo(&before, &after),
        Ok(vec![
            "new-untracked.txt".to_owned(),
            "tracked.txt".to_owned(),
        ])
    );

    assert_eq!(
        fs::read_to_string(repo.join("tracked.txt")).expect("read tracked file"),
        "preexisting edit plus run edit\n"
    );
    assert_eq!(
        fs::read_to_string(repo.join("preexisting-untracked.txt"))
            .expect("read preexisting untracked file"),
        "preexisting\n"
    );
    assert_eq!(
        fs::read_to_string(repo.join("new-untracked.txt")).expect("read new untracked file"),
        "new\n"
    );
}

#[cfg(unix)]
#[test]
fn req_033_credentials_file_requires_mode_0600() {
    // REQ-033: credentials.env succeeds at mode 0600 and fails at mode 0644.
    let temp = tempfile::tempdir().expect("temporary directory");
    let credentials = temp.path().join("credentials.env");
    fs::write(&credentials, "PLAN_KEY=value\n").expect("write credentials file");

    fs::set_permissions(&credentials, fs::Permissions::from_mode(0o600))
        .expect("set mode 0600");
    assert_eq!(check_credentials_file(&credentials), Ok(()));

    fs::set_permissions(&credentials, fs::Permissions::from_mode(0o644))
        .expect("set mode 0644");
    assert!(check_credentials_file(&credentials).is_err());
}

#[test]
fn req_033_credentials_file_must_exist() {
    // REQ-033: homes/<seat>/credentials.env must exist.
    let temp = tempfile::tempdir().expect("temporary directory");
    assert!(check_credentials_file(&temp.path().join("credentials.env")).is_err());
}

#[test]
fn req_033_stray_key_scan_reports_metered_keys_and_not_safe_values() {
    // REQ-033: the stray-key scan names metered API keys that a seat must never see.
    let clean = vec![
        ("HOME".to_owned(), "/seat/home".to_owned()),
        ("PATH".to_owned(), "/usr/bin".to_owned()),
        ("VIBE_CLI".to_owned(), "python".to_owned()),
    ];
    assert_eq!(scan_stray_keys(&clean), Ok(Vec::new()));

    let contaminated = vec![
        ("SAFE".to_owned(), "value".to_owned()),
        ("MISTRAL_API_KEY".to_owned(), "secret".to_owned()),
        ("OPENAI_API_KEY".to_owned(), "secret".to_owned()),
    ];
    assert_eq!(
        scan_stray_keys(&contaminated),
        Ok(vec![
            "MISTRAL_API_KEY".to_owned(),
            "OPENAI_API_KEY".to_owned(),
        ])
    );
}
