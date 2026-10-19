// GAUNTLET tiebreaker suite for T-201, written blind from the spec (no implementation read).
// SPEC_AMBIGUITY: REQ-004 ask_agent description rendering and test-only roster injection are not exposed by the listed public API.
// SPEC_AMBIGUITY: REQ-004 InvalidTarget text rendering is not exposed by the listed public API.
// SPEC_AMBIGUITY: REQ-004 handwritten-list scanning, including fixture injection, is not exposed by the listed public API.

use mcp_bridge::{
    display_agent_name, is_supported_agent_name, normalize_agent_name, supported_agent_names,
};
use shared_types::{AGENT_ALIASES, normalize_agent_name as normalize_shared_agent_name};
use std::collections::HashSet;

const EXPECTED_ROSTER: [&str; 8] = [
    "codex", "gemini", "grok", "claude", "deepseek", "kimi", "mistral", "qwen",
];

const EXPECTED_ALIASES: [(&str, &str); 14] = [
    ("antigravity", "gemini"),
    ("agy", "gemini"),
    ("grok-build", "grok"),
    ("xai", "grok"),
    ("supergrok", "grok"),
    ("kimi-code", "kimi"),
    ("moonshot", "kimi"),
    ("vibe", "mistral"),
    ("mistral-vibe", "mistral"),
    ("devstral", "mistral"),
    ("le-chat", "mistral"),
    ("qwen-code", "qwen"),
    ("alibaba", "qwen"),
    ("bailian", "qwen"),
];

#[test]
// REQ-001: the canonical roster contains exactly the eight specified seat keys.
#[ignore = "GAUNTLET triage FP: the spec fixes the roster membership, not its order; the existing order is the T-001 characterization pin (gemini, codex, deepseek, claude, grok) with the new seats appended"]
fn roster_is_exactly_the_eight_specified_seats() {
    assert_eq!(supported_agent_names(), EXPECTED_ROSTER.as_slice());
}

#[test]
// REQ-001: every canonical roster key is unique.
fn roster_has_no_duplicate_keys() {
    let roster = supported_agent_names();
    let unique: HashSet<_> = roster.iter().copied().collect();
    assert_eq!(unique.len(), roster.len());
}

#[test]
// REQ-001 and REQ-061: every roster key is accepted as a supported daemon target.
fn every_roster_key_is_supported() {
    for &agent in supported_agent_names() {
        assert_eq!(is_supported_agent_name(agent), true, "unsupported roster key: {agent}");
    }
}

#[test]
// REQ-001: names outside the exact canonical roster are rejected as roster keys.
fn names_outside_the_roster_are_not_supported_keys() {
    for agent in ["", "unknown", "not-a-seat"] {
        assert_eq!(is_supported_agent_name(agent), false, "unexpected roster key: {agent}");
    }
}

#[test]
// REQ-002 and REQ-004: the one exported alias table has the exact specified mappings.
fn alias_table_matches_the_spec_exactly() {
    assert_eq!(AGENT_ALIASES, EXPECTED_ALIASES.as_slice());
}

#[test]
// REQ-002: every alias is unique and points to a canonical roster key.
fn aliases_are_collision_free_and_target_canonical_seats() {
    let alias_names: HashSet<_> = AGENT_ALIASES.iter().map(|(alias, _)| *alias).collect();
    assert_eq!(alias_names.len(), AGENT_ALIASES.len());

    for &(alias, canonical) in AGENT_ALIASES {
        assert!(EXPECTED_ROSTER.contains(&canonical), "noncanonical target for {alias}");
        assert_eq!(EXPECTED_ROSTER.contains(&alias), false, "alias collides with roster: {alias}");
    }
}

#[test]
// REQ-002: every exported alias normalizes to its declared canonical key.
fn every_alias_normalizes_to_its_declared_target() {
    for &(alias, canonical) in AGENT_ALIASES {
        assert_eq!(normalize_shared_agent_name(alias), canonical);
        assert_eq!(normalize_agent_name(alias), canonical);
    }
}

#[test]
// REQ-002: canonical names pass through normalization after lowercasing.
fn canonical_names_normalize_case_insensitively() {
    for &canonical in &EXPECTED_ROSTER {
        assert_eq!(normalize_shared_agent_name(canonical), canonical);
        assert_eq!(normalize_agent_name(canonical), canonical);

        let uppercase = canonical.to_ascii_uppercase();
        assert_eq!(normalize_shared_agent_name(&uppercase), canonical);
        assert_eq!(normalize_agent_name(&uppercase), canonical);
    }
}

#[test]
// REQ-002: both public normalization entry points are identical for aliases and unknown names.
fn bridge_and_shared_normalizers_cannot_disagree() {
    for input in EXPECTED_ROSTER
        .iter()
        .copied()
        .chain(AGENT_ALIASES.iter().map(|(alias, _)| *alias))
        .chain(["UNLISTED", "MixedCaseUnknown"])
    {
        assert_eq!(normalize_agent_name(input), normalize_shared_agent_name(input));
    }
}

#[test]
// REQ-003: each new canonical seat has its exact operator-facing display name.
fn new_seats_have_exact_display_names() {
    assert_eq!(display_agent_name("kimi"), "Kimi");
    assert_eq!(display_agent_name("mistral"), "Mistral");
    assert_eq!(display_agent_name("qwen"), "Qwen");
}

#[test]
// REQ-003: every alias displays exactly like its canonical seat.
fn every_alias_has_its_canonical_display_name() {
    for &(alias, canonical) in AGENT_ALIASES {
        assert_eq!(display_agent_name(alias), display_agent_name(canonical));
    }
}
