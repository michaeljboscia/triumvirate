// GAUNTLET filter suite for T-202, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: row order of `daemon_target_agents` ("in row order") is not given in the listed material, so the set is asserted, not the order.
// SPEC_AMBIGUITY: the exact InvalidTarget message text is not pinned; only that it names every accepted target (REQ-004 "message and accepted set match").
// SPEC_AMBIGUITY: no row accessor is in the listed API, so "profile change moves the consumer" cannot be expressed here.

use mcp_tools::aliases::{
    AliasMappingError, SpawnDaemonParams, SpawnSessionRequestLike, daemon_target_agents,
    map_spawn_daemon_params,
};

fn spawn(target: &str) -> Result<SpawnSessionRequestLike, AliasMappingError> {
    map_spawn_daemon_params(SpawnDaemonParams {
        target: target.to_string(),
        session_name: None,
        cwd: None,
        timeout_ms: None,
    })
}

fn sorted_targets() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = daemon_target_agents().to_vec();
    v.sort();
    v
}

// REQ-004/061: targets are exactly the three existing CLI peers plus the three session-capable new seats.
#[test]
fn test_should_list_exactly_six_daemon_targets() {
    assert_eq!(
        sorted_targets(),
        vec!["codex", "gemini", "grok", "kimi", "mistral", "qwen"]
    );
}

// Property: no duplicate targets.
#[test]
fn test_should_have_no_duplicate_targets() {
    let t = sorted_targets();
    let mut d = t.clone();
    d.dedup();
    assert_eq!(t, d);
}

// Doc comment: claude is the orchestrator, not a daemon target.
#[test]
fn test_should_exclude_claude_from_targets() {
    assert!(!daemon_target_agents().contains(&"claude"));
}

// Doc comment: deepseek is HTTP, not a daemon target.
#[test]
fn test_should_exclude_deepseek_from_targets() {
    assert!(!daemon_target_agents().contains(&"deepseek"));
}

// REQ-061: every listed target is accepted by spawn mapping and maps to itself.
#[test]
fn test_should_accept_every_listed_target_as_itself() {
    for t in daemon_target_agents() {
        let r = spawn(t).unwrap_or_else(|e| panic!("{t} rejected: {e}"));
        assert_eq!(r.agent, *t);
    }
}

// REQ-061: new seats map through spawn with their canonical key.
#[test]
fn test_should_map_new_seats_for_spawn() {
    assert_eq!(spawn("kimi").unwrap().agent, "kimi");
    assert_eq!(spawn("mistral").unwrap().agent, "mistral");
    assert_eq!(spawn("qwen").unwrap().agent, "qwen");
}

// Doc comment: claude is rejected with InvalidTarget carrying the name.
#[test]
fn test_should_reject_claude_with_invalid_target() {
    match spawn("claude") {
        Err(AliasMappingError::InvalidTarget(s)) => assert!(s.contains("claude"), "got: {s}"),
        other => panic!("expected InvalidTarget, got ok={}", other.is_ok()),
    }
}

// Doc comment: deepseek is rejected with InvalidTarget.
#[test]
fn test_should_reject_deepseek_with_invalid_target() {
    assert!(matches!(
        spawn("deepseek"),
        Err(AliasMappingError::InvalidTarget(_))
    ));
}

// Edge: unknown target is rejected.
#[test]
fn test_should_reject_unknown_target() {
    assert!(matches!(
        spawn("not-a-real-seat"),
        Err(AliasMappingError::InvalidTarget(_))
    ));
}

// Edge: empty target is rejected.
#[test]
fn test_should_reject_empty_target() {
    assert!(spawn("").is_err());
}

// Doc comment: antigravity and agy normalize to gemini.
#[test]
fn test_should_normalize_gemini_aliases() {
    assert_eq!(spawn("antigravity").unwrap().agent, "gemini");
    assert_eq!(spawn("agy").unwrap().agent, "gemini");
}

// Doc comment: grok-build, xai, supergrok normalize to grok.
#[test]
fn test_should_normalize_grok_aliases() {
    for a in ["grok-build", "xai", "supergrok"] {
        assert_eq!(spawn(a).unwrap().agent, "grok", "alias {a}");
    }
}

// REQ-004: the error text renders from the same set the check accepts.
#[test]
fn test_should_name_every_accepted_target_in_error_text() {
    let Err(err) = spawn("not-a-real-seat") else {
        panic!("must be rejected");
    };
    let text = err.to_string();
    for t in daemon_target_agents() {
        assert!(text.contains(t), "error text missing {t}: {text}");
    }
}

// Spawn mapping carries through the other request fields unchanged.
#[test]
fn test_should_carry_name_cwd_and_timeout_through() {
    let r = map_spawn_daemon_params(SpawnDaemonParams {
        target: "kimi".to_string(),
        session_name: Some("daemon-x".to_string()),
        cwd: Some("/tmp/x".to_string()),
        timeout_ms: Some(1234),
    })
    .unwrap();
    assert_eq!(r.name.as_deref(), Some("daemon-x"));
    assert_eq!(r.cwd.as_deref(), Some("/tmp/x"));
    assert_eq!(r.timeout_ms, Some(1234));
}

// Purity: repeated calls return the same slice contents.
#[test]
fn test_should_be_stable_across_calls() {
    assert_eq!(daemon_target_agents(), daemon_target_agents());
}
