// GAUNTLET filter suite for T-104, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: the listed API names the `SeatProfile` type but not how a caller obtains one. The plan text for
// T-002 says `seat_profile("<key>")` in mcp_bridge::seat_profile; this suite assumes that accessor and that the
// kimi, mistral and qwen rows carry home_env_var (KIMI_CODE_HOME, VIBE_HOME, QWEN_HOME) and the REQ-033 env_remove list.
// SPEC_AMBIGUITY: no exact error text is given for seat_home, write_seat_files, child_env, render_seatbelt_profile,
// snapshot_repo or check_credentials_file, so error-path tests assert only `is_err()` (plus the one documented
// "cannot read" prefix for a non-regular daemon-owned file).
// SPEC_AMBIGUITY: render_seatbelt_profile for a path containing a double quote or newline (profile injection) is not
// specified as accepted or refused; not tested.
// SPEC_AMBIGUITY: scan_stray_keys does not list which names count as "metered-API keys" beyond the REQ-033 names
// MISTRAL_API_KEY and OPENAI_API_KEY; only those two are asserted.
// SPEC_AMBIGUITY: the pinned Qwen model id and plan baseUrl are not stated; only their presence, type and an https
// scheme are asserted. envKey is asserted as QWENCLOUD_TOKEN_PLAN_API_KEY from AMENDMENTS A-9.
// SPEC_AMBIGUITY: a planted symlink at Kimi's ro-agent.md may be refused or replaced; only "victim file untouched"
// is asserted for it.
// SPEC_AMBIGUITY: order of the Vec returned by audit_repo and scan_stray_keys is unspecified; compared sorted.

use mcp_bridge::containment::*;
use mcp_bridge::seat_profile::seat_profile;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

fn kv(k: &str, v: &str) -> (String, String) {
    (k.to_string(), v.to_string())
}

fn get<'a>(env: &'a [(String, String)], k: &str) -> Option<&'a str> {
    env.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
}

fn count(env: &[(String, String)], k: &str) -> usize {
    env.iter().filter(|(a, _)| a == k).count()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn mode(p: &Path) -> u32 {
    fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777
}

// ---------------------------------------------------------------- SEAT_CREDENTIAL_SCRUB (REQ-033)

#[test]
fn test_should_list_exactly_the_req033_scrub_names() {
    // REQ-033: the exact inherited credential and endpoint list.
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
        ][..]
    );
}

// ---------------------------------------------------------------- seat_home (REQ-030)

#[test]
fn test_should_join_homes_and_seat_under_triumvirate_home() {
    // REQ-030: $TRIUMVIRATE_HOME/homes/<seat>
    let sut = seat_home(Path::new("/base/tri"), "kimi").unwrap();
    assert_eq!(sut, PathBuf::from("/base/tri/homes/kimi"));
}

#[test]
fn test_should_accept_each_new_seat_key() {
    // REQ-030: the three new seat keys map to homes/<key>
    for seat in ["kimi", "mistral", "qwen"] {
        let sut = seat_home(Path::new("/base/tri"), seat).unwrap();
        assert_eq!(sut, PathBuf::from("/base/tri/homes").join(seat));
    }
}

#[test]
fn test_should_reject_seat_keys_that_escape_homes() {
    // seat_home doc: a seat key must not address anything outside homes/
    for bad in ["", ".", "..", "../x", "a/b", "/abs", "kimi/..", "../../etc", "a/../b"] {
        assert!(seat_home(Path::new("/base/tri"), bad).is_err(), "accepted {bad:?}");
    }
}

// ---------------------------------------------------------------- write_seat_files (REQ-031, REQ-032)

#[test]
fn test_should_write_kimi_ro_agent_file_with_0444_and_contract_text() {
    // BACKEND_STRUCTURE s5: ro-agent.md frontmatter and body, mode 0444
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("kimi");
    write_seat_files(&home, seat_profile("kimi").unwrap()).unwrap();
    let f = home.join("ro-agent.md");
    let text = fs::read_to_string(&f).unwrap();
    assert!(text.starts_with("---"), "frontmatter must open the file: {text}");
    for needle in [
        "name: triumvirate-readonly",
        "tools: [Read, Grep, Glob]",
        "disallowedTools: [Agent, AgentSwarm]",
        "subagents: []",
        "Read-only reviewer: never write files.",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in {text}");
    }
    assert_eq!(mode(&f), 0o444);
}

#[test]
fn test_should_rewrite_kimi_ro_agent_file_when_a_0444_copy_exists() {
    // write_seat_files doc: Kimi's file is rewritten; an existing 0444 copy needs no chmod
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("kimi");
    let p = seat_profile("kimi").unwrap();
    write_seat_files(&home, p).unwrap();
    let f = home.join("ro-agent.md");
    let first = fs::read_to_string(&f).unwrap();
    write_seat_files(&home, p).unwrap();
    assert_eq!(fs::read_to_string(&f).unwrap(), first);
    assert_eq!(mode(&f), 0o444);
}

#[test]
fn test_should_overwrite_tampered_kimi_ro_agent_file() {
    // write_seat_files doc: ro-agent.md is wholly the daemon's and is rewritten
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("kimi");
    fs::create_dir_all(&home).unwrap();
    let f = home.join("ro-agent.md");
    fs::write(&f, "tools: [Write, Shell]\n").unwrap();
    fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).unwrap();
    write_seat_files(&home, seat_profile("kimi").unwrap()).unwrap();
    let text = fs::read_to_string(&f).unwrap();
    assert!(!text.contains("Write"), "tampered tools survived: {text}");
    assert!(text.contains("tools: [Read, Grep, Glob]"));
    assert_eq!(mode(&f), 0o444);
}

#[test]
fn test_should_not_write_through_a_planted_symlink_for_kimi_file() {
    // write_seat_files doc: rename replaces a symlink itself, never its target
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("kimi");
    fs::create_dir_all(&home).unwrap();
    let victim = tmp.path().join("victim.txt");
    fs::write(&victim, "precious").unwrap();
    fs::set_permissions(&victim, fs::Permissions::from_mode(0o644)).unwrap();
    symlink(&victim, home.join("ro-agent.md")).unwrap();
    let _ = write_seat_files(&home, seat_profile("kimi").unwrap());
    assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
    assert_eq!(mode(&victim), 0o644);
}

#[test]
fn test_should_write_qwen_settings_json_with_plan_provider_and_no_cross_session() {
    // BACKEND_STRUCTURE s5: modelProviders.openai[0] and agents.crossSessionMessaging=false
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("qwen");
    write_seat_files(&home, seat_profile("qwen").unwrap()).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join("settings.json")).unwrap()).unwrap();
    let entry = &v["modelProviders"]["openai"][0];
    assert!(entry["id"].as_str().is_some_and(|s| !s.is_empty()), "id: {entry}");
    assert!(entry["baseUrl"].as_str().is_some_and(|s| s.starts_with("https://")), "baseUrl: {entry}");
    assert_eq!(entry["envKey"], "QWENCLOUD_TOKEN_PLAN_API_KEY");
    assert_eq!(v["agents"]["crossSessionMessaging"], serde_json::Value::Bool(false));
}

#[test]
fn test_should_merge_qwen_settings_and_keep_login_keys() {
    // write_seat_files doc: Qwen file is MERGED so login-recorded keys survive
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("qwen");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join("settings.json"),
        r#"{"loginMarker":"kept","agents":{"crossSessionMessaging":true}}"#,
    )
    .unwrap();
    write_seat_files(&home, seat_profile("qwen").unwrap()).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join("settings.json")).unwrap()).unwrap();
    assert_eq!(v["loginMarker"], "kept");
    assert_eq!(v["agents"]["crossSessionMessaging"], serde_json::Value::Bool(false));
    assert!(v["modelProviders"]["openai"][0]["id"].is_string());
}

#[test]
fn test_should_be_idempotent_for_qwen_settings() {
    // write_seat_files doc: only the daemon's keys are set, so a second call changes nothing
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("qwen");
    let p = seat_profile("qwen").unwrap();
    write_seat_files(&home, p).unwrap();
    let a: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join("settings.json")).unwrap()).unwrap();
    write_seat_files(&home, p).unwrap();
    let b: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(home.join("settings.json")).unwrap()).unwrap();
    assert_eq!(a, b);
    assert_eq!(b["modelProviders"]["openai"].as_array().unwrap().len(), 1);
}

#[test]
fn test_should_refuse_qwen_settings_symlink_and_leave_target_untouched() {
    // write_seat_files doc: a symlinked daemon-owned file is an error, never read or written through
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("qwen");
    fs::create_dir_all(&home).unwrap();
    let victim = tmp.path().join("victim.json");
    fs::write(&victim, r#"{"a":1}"#).unwrap();
    symlink(&victim, home.join("settings.json")).unwrap();
    let r = write_seat_files(&home, seat_profile("qwen").unwrap());
    assert!(r.is_err());
    assert_eq!(fs::read_to_string(&victim).unwrap(), r#"{"a":1}"#);
}

#[test]
fn test_should_keep_cannot_read_prefix_for_non_regular_daemon_file() {
    // write_seat_files doc: a non-regular file keeps the "cannot read" prefix
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("qwen");
    fs::create_dir_all(home.join("settings.json")).unwrap();
    let e = write_seat_files(&home, seat_profile("qwen").unwrap()).unwrap_err();
    assert!(e.starts_with("cannot read"), "got {e}");
}

#[test]
fn test_should_write_mistral_config_with_telemetry_off() {
    // REQ-031: Mistral config.toml has enable_telemetry=false
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("mistral");
    write_seat_files(&home, seat_profile("mistral").unwrap()).unwrap();
    let text = fs::read_to_string(home.join("config.toml")).unwrap();
    let hits: Vec<&str> = text
        .lines()
        .filter(|l| l.split('=').next().map(str::trim) == Some("enable_telemetry"))
        .collect();
    assert_eq!(hits.len(), 1, "{text}");
    assert_eq!(hits[0].split('=').nth(1).unwrap().trim(), "false");
}

#[test]
fn test_should_merge_mistral_config_replacing_top_level_telemetry_and_keeping_tables() {
    // write_seat_files doc: set the key before the first [table], replace an existing assignment
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("mistral");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join("config.toml"),
        "active_model = \"devstral\"\nenable_telemetry = true\n\n[section]\nfoo = 1\nenable_telemetry = true\n",
    )
    .unwrap();
    write_seat_files(&home, seat_profile("mistral").unwrap()).unwrap();
    let text = fs::read_to_string(home.join("config.toml")).unwrap();
    let table_at = text.find("[section]").expect("table kept");
    let (top, rest) = text.split_at(table_at);
    assert!(top.contains("active_model = \"devstral\""), "{text}");
    assert!(rest.contains("foo = 1"), "{text}");
    let top_hits: Vec<&str> = top
        .lines()
        .filter(|l| l.split('=').next().map(str::trim) == Some("enable_telemetry"))
        .collect();
    assert_eq!(top_hits.len(), 1, "{text}");
    assert_eq!(top_hits[0].split('=').nth(1).unwrap().trim(), "false");
    // the same key inside a table is not the daemon's and is left alone
    assert!(rest.contains("enable_telemetry = true"), "{text}");
}

#[test]
fn test_should_add_telemetry_key_before_first_table_when_absent() {
    // write_seat_files doc: top-level table, before the first [table] header
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("mistral");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("config.toml"), "[ui]\ntheme = \"dark\"\n").unwrap();
    write_seat_files(&home, seat_profile("mistral").unwrap()).unwrap();
    let text = fs::read_to_string(home.join("config.toml")).unwrap();
    let key_at = text.find("enable_telemetry").expect("key added");
    assert!(key_at < text.find("[ui]").unwrap(), "{text}");
    assert!(text.contains("theme = \"dark\""));
}

#[test]
fn test_should_refuse_mistral_config_symlink_and_leave_target_untouched() {
    // write_seat_files doc: never reads or writes through a link
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("mistral");
    fs::create_dir_all(&home).unwrap();
    let victim = tmp.path().join("victim.toml");
    fs::write(&victim, "x = 1\n").unwrap();
    symlink(&victim, home.join("config.toml")).unwrap();
    let r = write_seat_files(&home, seat_profile("mistral").unwrap());
    assert!(r.is_err());
    assert_eq!(fs::read_to_string(&victim).unwrap(), "x = 1\n");
}

#[test]
fn test_should_refuse_a_seat_home_that_is_a_symlink() {
    // write_seat_files doc: refuse a seat home that is a symlink
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    fs::create_dir_all(&real).unwrap();
    let home = tmp.path().join("home");
    symlink(&real, &home).unwrap();
    assert!(write_seat_files(&home, seat_profile("qwen").unwrap()).is_err());
    assert!(!real.join("settings.json").exists());
}

#[test]
fn test_should_refuse_a_seat_home_that_is_a_file() {
    // write_seat_files doc: refuse a seat home that is not a directory
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    fs::write(&home, "not a dir").unwrap();
    assert!(write_seat_files(&home, seat_profile("kimi").unwrap()).is_err());
    assert_eq!(fs::read_to_string(&home).unwrap(), "not a dir");
}

#[test]
fn test_should_create_a_missing_seat_home() {
    // write_seat_files doc: a missing home is fine (it is created next)
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("deep").join("kimi");
    write_seat_files(&home, seat_profile("kimi").unwrap()).unwrap();
    assert!(home.is_dir());
}

#[test]
fn test_should_leave_no_non_empty_hook_plugin_or_skill_dirs_in_seat_home() {
    // REQ-031: the only skills/hooks/plugins dirs present are empty daemon-created ones
    for seat in ["kimi", "mistral", "qwen"] {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join(seat);
        write_seat_files(&home, seat_profile(seat).unwrap()).unwrap();
        for name in ["skills", "hooks", "plugins"] {
            let d = home.join(name);
            if d.exists() {
                assert_eq!(fs::read_dir(&d).unwrap().count(), 0, "{seat}/{name} not empty");
            }
        }
    }
}

#[test]
fn test_should_leave_no_stray_temp_files_after_write() {
    // write_seat_files doc: the temp file is removed; only daemon files remain
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("kimi");
    write_seat_files(&home, seat_profile("kimi").unwrap()).unwrap();
    let names: Vec<String> = fs::read_dir(&home)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["ro-agent.md".to_string()]);
}

// ---------------------------------------------------------------- child_env (REQ-030, REQ-033)

#[test]
fn test_should_scrub_credentials_and_add_vibe_cli_for_mistral() {
    // REQ-033 reality test: MISTRAL_API_KEY and OPENAI_API_KEY removed, VIBE_CLI=python inserted
    let sk = format!("{}{}", "sk-", "fake1");
    let base = vec![
        kv("PATH", "/usr/bin"),
        kv("HOME", "/real/home"),
        kv("MISTRAL_API_KEY", &sk),
        kv("OPENAI_API_KEY", &sk),
    ];
    let sut = child_env(
        &base,
        seat_profile("mistral").unwrap(),
        Path::new("/seat/home"),
        &[kv("VIBE_CLI", "python")],
    )
    .unwrap();
    assert_eq!(get(&sut, "MISTRAL_API_KEY"), None);
    assert_eq!(get(&sut, "OPENAI_API_KEY"), None);
    assert_eq!(get(&sut, "VIBE_CLI"), Some("python"));
    assert_eq!(get(&sut, "PATH"), Some("/usr/bin"));
}

#[test]
fn test_should_set_home_and_vendor_home_var_for_each_seat() {
    // REQ-030: vendor home var AND HOME == seat home, real home absent
    for (seat, var) in [("kimi", "KIMI_CODE_HOME"), ("mistral", "VIBE_HOME"), ("qwen", "QWEN_HOME")] {
        let base = vec![kv("HOME", "/real/home"), kv("PATH", "/usr/bin")];
        let sut = child_env(&base, seat_profile(seat).unwrap(), Path::new("/seat/home"), &[]).unwrap();
        assert_eq!(get(&sut, "HOME"), Some("/seat/home"), "{seat}");
        assert_eq!(get(&sut, var), Some("/seat/home"), "{seat}");
        assert_eq!(count(&sut, "HOME"), 1, "{seat}");
        assert!(sut.iter().all(|(_, v)| v != "/real/home"), "{seat} leaks the real home");
    }
}

#[test]
fn test_should_set_home_even_when_base_has_no_home() {
    // REQ-030: HOME is set regardless of the inherited env
    let sut = child_env(&[], seat_profile("kimi").unwrap(), Path::new("/seat/home"), &[]).unwrap();
    assert_eq!(get(&sut, "HOME"), Some("/seat/home"));
    assert_eq!(get(&sut, "KIMI_CODE_HOME"), Some("/seat/home"));
}

#[test]
fn test_should_remove_prefix_pattern_vars_for_kimi_and_qwen() {
    // REQ-033: KIMI_*, MOONSHOT_*, DASHSCOPE_*, BAILIAN_*, QWENCLOUD_* prefix patterns scrubbed
    let base = vec![
        kv("KIMI_FOO", "a"),
        kv("MOONSHOT_API_KEY", "b"),
        kv("DASHSCOPE_API_KEY", "c"),
        kv("BAILIAN_X", "d"),
        kv("QWENCLOUD_TOKEN_PLAN_API_KEY", "e"),
        kv("OPENAI_BASE_URL", "http://x"),
        kv("KEEP_ME", "yes"),
    ];
    for seat in ["kimi", "qwen"] {
        let sut = child_env(&base, seat_profile(seat).unwrap(), Path::new("/s"), &[]).unwrap();
        for gone in [
            "KIMI_FOO",
            "MOONSHOT_API_KEY",
            "DASHSCOPE_API_KEY",
            "BAILIAN_X",
            "QWENCLOUD_TOKEN_PLAN_API_KEY",
            "OPENAI_BASE_URL",
        ] {
            assert_eq!(get(&sut, gone), None, "{seat} kept {gone}");
        }
        assert_eq!(get(&sut, "KEEP_ME"), Some("yes"), "{seat}");
    }
}

#[test]
fn test_should_let_env_add_win_over_scrub_for_the_plan_key() {
    // REQ-033: scrub runs before env_add; the plan key is the only credential var left, once
    let inherited = format!("{}{}", "sk-", "inherited");
    let plan = format!("{}{}", "sk-sp-", "plan");
    let base = vec![kv("QWENCLOUD_TOKEN_PLAN_API_KEY", &inherited), kv("DASHSCOPE_API_KEY", &inherited)];
    let sut = child_env(
        &base,
        seat_profile("qwen").unwrap(),
        Path::new("/s"),
        &[kv("QWENCLOUD_TOKEN_PLAN_API_KEY", &plan)],
    )
    .unwrap();
    assert_eq!(count(&sut, "QWENCLOUD_TOKEN_PLAN_API_KEY"), 1);
    assert_eq!(get(&sut, "QWENCLOUD_TOKEN_PLAN_API_KEY"), Some(plan.as_str()));
    assert_eq!(get(&sut, "DASHSCOPE_API_KEY"), None);
}

#[test]
fn test_should_let_env_add_win_for_a_seat_own_prefix_var() {
    // SEAT_CREDENTIAL_SCRUB doc: a seat's own variable is added back afterwards by env_add
    let base = vec![kv("KIMI_DISABLE_TELEMETRY", "0")];
    let sut = child_env(
        &base,
        seat_profile("kimi").unwrap(),
        Path::new("/s"),
        &[kv("KIMI_DISABLE_TELEMETRY", "1")],
    )
    .unwrap();
    assert_eq!(get(&sut, "KIMI_DISABLE_TELEMETRY"), Some("1"));
    assert_eq!(count(&sut, "KIMI_DISABLE_TELEMETRY"), 1);
}

#[test]
fn test_should_not_mutate_the_base_env() {
    // child_env takes a slice and returns a new env
    let base = vec![kv("HOME", "/real/home"), kv("OPENAI_API_KEY", "k")];
    let before = base.clone();
    let _ = child_env(&base, seat_profile("mistral").unwrap(), Path::new("/s"), &[]).unwrap();
    assert_eq!(base, before);
}

// ---------------------------------------------------------------- render_seatbelt_profile (REQ-035)

fn canon(p: &Path) -> String {
    fs::canonicalize(p).unwrap().to_string_lossy().into_owned()
}

#[test]
fn test_should_render_the_section5_seatbelt_profile_with_canonical_paths() {
    // REQ-035 / BACKEND_STRUCTURE s5: template lines with canonicalized existing paths
    let tmp = tempfile::tempdir().unwrap();
    let (h, o, t) = (tmp.path().join("h"), tmp.path().join("o"), tmp.path().join("t"));
    for d in [&h, &o, &t] {
        fs::create_dir_all(d).unwrap();
    }
    let sut = render_seatbelt_profile(&h, &o, &t).unwrap();
    let lines: Vec<&str> = sut.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let expected = vec![
        "(version 1)".to_string(),
        "(allow default)".to_string(),
        "(deny file-write*)".to_string(),
        format!("(allow file-write* (subpath \"{}\"))", canon(&h)),
        format!("(allow file-write* (subpath \"{}\"))", canon(&o)),
        format!("(allow file-write* (subpath \"{}\"))", canon(&t)),
        "(allow file-write* (subpath \"/private/var/folders\"))".to_string(),
        "(allow file-write* (subpath \"/private/tmp\"))".to_string(),
        "(allow file-write* (literal \"/dev/null\") (literal \"/dev/stdout\") (literal \"/dev/stderr\") (literal \"/dev/dtracehelper\") (literal \"/dev/tty\"))".to_string(),
    ];
    assert_eq!(lines, expected);
}

#[test]
fn test_should_deny_before_any_allow_in_the_profile() {
    // REQ-035: (deny file-write*) then allow; later rules win in seatbelt
    let tmp = tempfile::tempdir().unwrap();
    let sut = render_seatbelt_profile(tmp.path(), tmp.path(), tmp.path()).unwrap();
    let deny = sut.find("(deny file-write*)").unwrap();
    let first_allow_write = sut.find("(allow file-write*").unwrap();
    assert!(deny < first_allow_write);
}

#[test]
fn test_should_use_given_path_text_when_a_path_does_not_exist() {
    // render_seatbelt_profile doc: canonicalized only when the path exists
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("not-yet-created");
    let sut = render_seatbelt_profile(tmp.path(), &missing, tmp.path()).unwrap();
    let want = format!("(subpath \"{}\")", missing.to_string_lossy());
    assert!(sut.contains(&want), "{sut}");
}

#[test]
fn test_should_leave_no_template_placeholders_in_the_profile() {
    // BACKEND_STRUCTURE s5: every @...@ slot is filled
    let tmp = tempfile::tempdir().unwrap();
    let sut = render_seatbelt_profile(tmp.path(), tmp.path(), tmp.path()).unwrap();
    assert!(!sut.contains('@'), "{sut}");
}

#[cfg(target_os = "macos")]
mod sandbox {
    use super::*;

    // REQ-035 acceptance (1): repo write denied, seat-home and output-dir writes allowed, under real sandbox-exec.
    // The scratch root lives under the crate dir (not /var/folders or /private/tmp, which the profile allows).
    fn run_under(profile: &str, script: &str, tmp: &Path) -> bool {
        let pf = tmp.join("p.sb");
        fs::write(&pf, profile).unwrap();
        Command::new("/usr/bin/sandbox-exec")
            .arg("-f")
            .arg(&pf)
            .args(["/bin/sh", "-c", script])
            .status()
            .unwrap()
            .success()
    }

    #[test]
    fn test_should_deny_repo_write_and_allow_seat_home_and_output_dir() {
        // REQ-035 (1)
        let root = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let (home, out, tmpd, repo) = (
            root.path().join("home"),
            root.path().join("out"),
            root.path().join("tmp"),
            root.path().join("repo"),
        );
        for d in [&home, &out, &tmpd, &repo] {
            fs::create_dir_all(d).unwrap();
        }
        let profile = render_seatbelt_profile(&home, &out, &tmpd).unwrap();
        let denied = run_under(&profile, &format!("echo x > '{}/f'", repo.display()), root.path());
        assert!(!denied, "repo write must fail");
        assert!(!repo.join("f").exists());
        assert!(run_under(&profile, &format!("echo x > '{}/f'", home.display()), root.path()));
        assert!(home.join("f").exists());
        assert!(run_under(&profile, &format!("echo x > '{}/f'", out.display()), root.path()));
        assert!(out.join("f").exists());
        assert!(run_under(&profile, &format!("echo x > '{}/f'", tmpd.display()), root.path()));
        assert!(run_under(&profile, "echo x > /dev/null", root.path()));
    }

    #[test]
    fn test_should_deny_write_to_a_sibling_of_the_seat_home() {
        // REQ-035: only the seat home subtree is allowed, not its parent
        let root = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let (home, out, tmpd) = (root.path().join("home"), root.path().join("out"), root.path().join("tmp"));
        for d in [&home, &out, &tmpd] {
            fs::create_dir_all(d).unwrap();
        }
        let profile = render_seatbelt_profile(&home, &out, &tmpd).unwrap();
        assert!(!run_under(&profile, &format!("echo x > '{}/sibling'", root.path().display()), root.path()));
    }
}

// ---------------------------------------------------------------- snapshot_repo / audit_repo (REQ-035)

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn init_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q"]);
    for f in ["a.txt", "b.txt", "c.txt"] {
        fs::write(tmp.path().join(f), format!("base {f}\n")).unwrap();
    }
    git(tmp.path(), &["add", "."]);
    git(tmp.path(), &["commit", "-q", "-m", "base"]);
    tmp
}

fn dirty_repo() -> tempfile::TempDir {
    // pre-existing tracked edit (a.txt) and untracked file (u1.txt)
    let tmp = init_repo();
    fs::write(tmp.path().join("a.txt"), "edited before\n").unwrap();
    fs::write(tmp.path().join("u1.txt"), "untracked before\n").unwrap();
    tmp
}

#[test]
fn test_should_return_empty_snapshot_for_a_clean_repo() {
    // snapshot_repo doc: only changed, staged or untracked paths appear
    let repo = init_repo();
    let sut = snapshot_repo(repo.path()).unwrap();
    assert!(sut.entries.is_empty(), "{:?}", sut.entries);
}

#[test]
fn test_should_list_tracked_edit_and_untracked_file_in_snapshot() {
    // snapshot_repo doc: status plus content hash of every changed or untracked path
    let repo = dirty_repo();
    let sut = snapshot_repo(repo.path()).unwrap();
    let keys: Vec<&str> = sut.entries.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["a.txt", "u1.txt"]);
}

#[test]
fn test_should_report_both_new_paths_and_leave_all_four_untouched() {
    // REQ-035 (2) / reality test: new tracked edit + new untracked file on an already dirty repo
    let repo = dirty_repo();
    let before = snapshot_repo(repo.path()).unwrap();
    fs::write(repo.path().join("b.txt"), "edited during\n").unwrap();
    fs::write(repo.path().join("u2.txt"), "untracked during\n").unwrap();
    let after = snapshot_repo(repo.path()).unwrap();
    let sut = audit_repo(&before, &after).unwrap();
    assert_eq!(sorted(sut), vec!["b.txt".to_string(), "u2.txt".to_string()]);
    // nothing reverted
    assert_eq!(fs::read_to_string(repo.path().join("a.txt")).unwrap(), "edited before\n");
    assert_eq!(fs::read_to_string(repo.path().join("u1.txt")).unwrap(), "untracked before\n");
    assert_eq!(fs::read_to_string(repo.path().join("b.txt")).unwrap(), "edited during\n");
    assert_eq!(fs::read_to_string(repo.path().join("u2.txt")).unwrap(), "untracked during\n");
}

#[test]
fn test_should_report_nothing_when_a_dirty_repo_is_unchanged() {
    // BACKEND_STRUCTURE s9: repo already dirty -> audit compares deltas only
    let repo = dirty_repo();
    let before = snapshot_repo(repo.path()).unwrap();
    let after = snapshot_repo(repo.path()).unwrap();
    assert_eq!(audit_repo(&before, &after).unwrap(), Vec::<String>::new());
}

#[test]
fn test_should_report_a_pre_dirty_path_whose_content_changed_again() {
    // audit_repo doc: CHANGED in after compared with before (content hash differs)
    let repo = dirty_repo();
    let before = snapshot_repo(repo.path()).unwrap();
    fs::write(repo.path().join("a.txt"), "edited again\n").unwrap();
    let after = snapshot_repo(repo.path()).unwrap();
    assert_eq!(audit_repo(&before, &after).unwrap(), vec!["a.txt".to_string()]);
}

#[test]
fn test_should_report_an_untracked_file_whose_content_changed() {
    // audit_repo doc: content change of an existing untracked path counts
    let repo = dirty_repo();
    let before = snapshot_repo(repo.path()).unwrap();
    fs::write(repo.path().join("u1.txt"), "rewritten\n").unwrap();
    let after = snapshot_repo(repo.path()).unwrap();
    assert_eq!(audit_repo(&before, &after).unwrap(), vec!["u1.txt".to_string()]);
}

#[test]
fn test_should_report_a_pre_existing_edit_that_was_restored() {
    // audit_repo doc: a path in before that is gone from after changed too
    let repo = dirty_repo();
    let before = snapshot_repo(repo.path()).unwrap();
    git(repo.path(), &["checkout", "--", "a.txt"]);
    let after = snapshot_repo(repo.path()).unwrap();
    assert_eq!(audit_repo(&before, &after).unwrap(), vec!["a.txt".to_string()]);
}

#[test]
fn test_should_report_a_deleted_pre_existing_untracked_file() {
    // audit_repo doc: a path in before that is gone from after changed too
    let repo = dirty_repo();
    let before = snapshot_repo(repo.path()).unwrap();
    fs::remove_file(repo.path().join("u1.txt")).unwrap();
    let after = snapshot_repo(repo.path()).unwrap();
    assert_eq!(audit_repo(&before, &after).unwrap(), vec!["u1.txt".to_string()]);
}

#[test]
fn test_should_report_a_deleted_tracked_file() {
    // snapshot_repo doc: a deletion is a status entry (marker for a deletion)
    let repo = init_repo();
    let before = snapshot_repo(repo.path()).unwrap();
    fs::remove_file(repo.path().join("c.txt")).unwrap();
    let after = snapshot_repo(repo.path()).unwrap();
    assert_eq!(audit_repo(&before, &after).unwrap(), vec!["c.txt".to_string()]);
}

#[test]
fn test_should_list_untracked_files_inside_new_directories_individually() {
    // `--untracked-files=all`: files, not the collapsed directory
    let repo = init_repo();
    let before = snapshot_repo(repo.path()).unwrap();
    fs::create_dir_all(repo.path().join("d/e")).unwrap();
    fs::write(repo.path().join("d/e/deep.txt"), "x").unwrap();
    fs::write(repo.path().join("d/top.txt"), "y").unwrap();
    let after = snapshot_repo(repo.path()).unwrap();
    assert_eq!(
        sorted(audit_repo(&before, &after).unwrap()),
        vec!["d/e/deep.txt".to_string(), "d/top.txt".to_string()]
    );
}

#[test]
fn test_should_handle_paths_with_spaces_and_unicode() {
    // `-z` porcelain: no quoting of unusual names
    let repo = init_repo();
    let before = snapshot_repo(repo.path()).unwrap();
    fs::write(repo.path().join("with space.txt"), "x").unwrap();
    fs::write(repo.path().join("caf\u{e9}.txt"), "y").unwrap();
    let after = snapshot_repo(repo.path()).unwrap();
    assert_eq!(
        sorted(audit_repo(&before, &after).unwrap()),
        sorted(vec!["with space.txt".to_string(), "caf\u{e9}.txt".to_string()])
    );
}

#[test]
fn test_should_report_a_newly_staged_file() {
    // snapshot_repo doc: staged paths count
    let repo = init_repo();
    let before = snapshot_repo(repo.path()).unwrap();
    fs::write(repo.path().join("staged.txt"), "s").unwrap();
    git(repo.path(), &["add", "staged.txt"]);
    let after = snapshot_repo(repo.path()).unwrap();
    assert_eq!(audit_repo(&before, &after).unwrap(), vec!["staged.txt".to_string()]);
}

#[test]
fn test_should_not_trip_the_audit_for_writes_outside_the_repo() {
    // REQ-035 (3): writes under seat home, output dir and temp do not trip the audit
    let repo = init_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let before = snapshot_repo(repo.path()).unwrap();
    fs::write(elsewhere.path().join("out.json"), "{}").unwrap();
    let after = snapshot_repo(repo.path()).unwrap();
    assert_eq!(audit_repo(&before, &after).unwrap(), Vec::<String>::new());
}

#[test]
fn test_should_error_on_snapshot_of_a_non_repo() {
    // snapshot_repo returns Err when git cannot report status
    let tmp = tempfile::tempdir().unwrap();
    assert!(snapshot_repo(tmp.path()).is_err());
}

#[test]
fn test_should_error_on_snapshot_of_a_missing_path() {
    // snapshot_repo returns Err for a nonexistent cwd
    let tmp = tempfile::tempdir().unwrap();
    assert!(snapshot_repo(&tmp.path().join("nope")).is_err());
}

fn snap(items: &[(&str, &str)]) -> RepoSnapshot {
    let mut entries = BTreeMap::new();
    for (k, v) in items {
        entries.insert(k.to_string(), v.to_string());
    }
    RepoSnapshot { entries }
}

#[test]
fn test_should_audit_synthetic_snapshots_new_changed_gone_and_same() {
    // audit_repo doc: NEW or CHANGED in after, plus a before path gone from after; same entries are not reported
    let before = snap(&[("same", "h1"), ("changed", "h1"), ("gone", "h1")]);
    let after = snap(&[("same", "h1"), ("changed", "h2"), ("new", "h9")]);
    let sut = audit_repo(&before, &after).unwrap();
    assert_eq!(sorted(sut), vec!["changed".to_string(), "gone".to_string(), "new".to_string()]);
}

#[test]
fn test_should_audit_two_empty_snapshots_as_clean() {
    // audit_repo doc: a non-empty result fails the call; empty means pass
    assert_eq!(audit_repo(&snap(&[]), &snap(&[])).unwrap(), Vec::<String>::new());
}

#[test]
fn test_should_report_everything_when_before_is_empty() {
    // audit_repo doc: every entry in after is NEW
    let sut = audit_repo(&snap(&[]), &snap(&[("x", "1"), ("y", "2")])).unwrap();
    assert_eq!(sorted(sut), vec!["x".to_string(), "y".to_string()]);
}

#[test]
fn test_should_not_report_duplicates() {
    // audit_repo returns each path once
    let before = snap(&[("p", "1")]);
    let after = snap(&[("p", "2")]);
    assert_eq!(audit_repo(&before, &after).unwrap(), vec!["p".to_string()]);
}

// ---------------------------------------------------------------- check_credentials_file (REQ-033)

fn cred(mode_bits: u32) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("credentials.env");
    fs::write(&f, "QWENCLOUD_TOKEN_PLAN_API_KEY=placeholder").unwrap();
    fs::set_permissions(&f, fs::Permissions::from_mode(mode_bits)).unwrap();
    (tmp, f)
}

#[test]
fn test_should_accept_credentials_file_with_mode_0600() {
    // REQ-033 / check_credentials_file doc: exists with mode 0600
    let (_t, f) = cred(0o600);
    assert!(check_credentials_file(&f).is_ok());
}

#[test]
fn test_should_reject_credentials_file_with_mode_0644() {
    // REQ-033 reality test: 0644 fails
    let (_t, f) = cred(0o644);
    assert!(check_credentials_file(&f).is_err());
}

#[test]
fn test_should_reject_every_other_mode() {
    // check_credentials_file doc: 0600 exactly
    for m in [0o400, 0o640, 0o660, 0o666, 0o700, 0o604, 0o000] {
        let (_t, f) = cred(m);
        assert!(check_credentials_file(&f).is_err(), "mode {m:o} accepted");
        let _ = fs::set_permissions(&f, fs::Permissions::from_mode(0o600));
    }
}

#[test]
fn test_should_reject_a_missing_credentials_file() {
    // check_credentials_file doc: must exist
    let tmp = tempfile::tempdir().unwrap();
    assert!(check_credentials_file(&tmp.path().join("credentials.env")).is_err());
}

// ---------------------------------------------------------------- scan_stray_keys (REQ-033)

#[test]
fn test_should_name_metered_keys_present_in_the_env() {
    // scan_stray_keys doc: names of metered-API keys present
    let secret = format!("{}{}", "sk-", "notreal");
    let env = vec![
        kv("PATH", "/usr/bin"),
        kv("MISTRAL_API_KEY", &secret),
        kv("OPENAI_API_KEY", &secret),
    ];
    let sut = scan_stray_keys(&env).unwrap();
    let names = sorted(sut.clone());
    assert!(names.contains(&"MISTRAL_API_KEY".to_string()), "{names:?}");
    assert!(names.contains(&"OPENAI_API_KEY".to_string()), "{names:?}");
    assert!(!names.contains(&"PATH".to_string()), "{names:?}");
    assert!(sut.iter().all(|n| !n.contains(&secret)), "a key value leaked into the result");
}

#[test]
fn test_should_return_empty_for_an_env_without_metered_keys() {
    // scan_stray_keys doc: clean env yields nothing
    let env = vec![kv("PATH", "/usr/bin"), kv("HOME", "/seat/home"), kv("VIBE_CLI", "python")];
    assert_eq!(scan_stray_keys(&env).unwrap(), Vec::<String>::new());
}

#[test]
fn test_should_return_empty_for_an_empty_env() {
    // scan_stray_keys edge: no env at all
    assert_eq!(scan_stray_keys(&[]).unwrap(), Vec::<String>::new());
}

#[test]
fn test_should_report_a_single_key_once() {
    // scan_stray_keys returns names, one per key present
    let env = vec![kv("OPENAI_API_KEY", "x")];
    let sut = scan_stray_keys(&env).unwrap();
    assert_eq!(sut.iter().filter(|n| *n == "OPENAI_API_KEY").count(), 1);
}

#[test]
fn test_should_find_no_stray_keys_in_a_scrubbed_child_env() {
    // REQ-033: after the scrub, no credential variable remains for the seat to see
    let secret = format!("{}{}", "sk-", "notreal");
    let base = vec![
        kv("PATH", "/usr/bin"),
        kv("MISTRAL_API_KEY", &secret),
        kv("OPENAI_API_KEY", &secret),
    ];
    let env = child_env(
        &base,
        seat_profile("mistral").unwrap(),
        Path::new("/seat/home"),
        &[kv("VIBE_CLI", "python")],
    )
    .unwrap();
    assert_eq!(scan_stray_keys(&env).unwrap(), Vec::<String>::new());
}
