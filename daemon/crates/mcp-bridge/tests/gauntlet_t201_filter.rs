// GAUNTLET filter suite for T-201, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: the spec does not say whether alias lookup is case-insensitive ("Vibe" vs "vibe"); only that
//   non-aliases pass through lowercased. No test asserts uppercase alias resolution.
// SPEC_AMBIGUITY: the spec does not say whether surrounding whitespace is trimmed by normalize_agent_name.
// SPEC_AMBIGUITY: the spec does not say whether is_supported_agent_name accepts aliases or only canonical keys.
// SPEC_AMBIGUITY: display names of the five existing seats and of unknown agents are not given; only Kimi, Mistral, Qwen.
// SPEC_AMBIGUITY: tool description rendering, InvalidTarget text, the hand-written-list scan and the T-003 placeholder
//   grep are not reachable from the listed public API (they live in the triumvirate and mcp-tools crates).

use mcp_bridge::{
    display_agent_name, is_supported_agent_name, normalize_agent_name, supported_agent_names,
};
use std::collections::HashSet;

const NEW_ALIASES: &[(&str, &str)] = &[
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

const EXISTING_ALIASES: &[(&str, &str)] = &[
    ("antigravity", "gemini"),
    ("agy", "gemini"),
    ("grok-build", "grok"),
    ("xai", "grok"),
    ("supergrok", "grok"),
];

const EIGHT: &[&str] = &[
    "codex", "gemini", "grok", "claude", "deepseek", "kimi", "mistral", "qwen",
];

// REQ-001: the three new canonical keys are on the roster.
#[test]
fn roster_contains_the_three_new_keys() {
    let names = supported_agent_names();
    for k in ["kimi", "mistral", "qwen"] {
        assert!(names.contains(&k), "roster missing {k}: {names:?}");
    }
}

// REQ-001 / done_when: eight-seat roster, exactly the eight keys.
#[test]
fn roster_is_exactly_eight_seats() {
    let names = supported_agent_names();
    let got: HashSet<&str> = names.iter().copied().collect();
    let want: HashSet<&str> = EIGHT.iter().copied().collect();
    assert_eq!(got, want);
    assert_eq!(names.len(), 8);
}

// REQ-001: the roster has no duplicate entries.
#[test]
fn roster_has_no_duplicates() {
    let names = supported_agent_names();
    let set: HashSet<&&str> = names.iter().collect();
    assert_eq!(set.len(), names.len());
}

// REQ-001: roster entries are canonical keys (lowercase, no whitespace).
#[test]
fn roster_entries_are_lowercase_and_trimmed() {
    for n in supported_agent_names() {
        assert_eq!(*n, n.trim());
        assert_eq!(*n, n.to_lowercase());
        assert!(!n.is_empty());
    }
}

// REQ-GROK-003 via REQ-001: is_supported_agent_name validates against the roster, for every entry.
#[test]
fn every_roster_entry_is_supported() {
    for n in supported_agent_names() {
        assert!(is_supported_agent_name(n), "{n} on roster but not supported");
    }
}

// REQ-001: the new keys are supported.
#[test]
fn new_keys_are_supported() {
    assert!(is_supported_agent_name("kimi"));
    assert!(is_supported_agent_name("mistral"));
    assert!(is_supported_agent_name("qwen"));
}

// REQ-001: names not on the roster are rejected (a validator that accepts everything is caught).
#[test]
fn unknown_names_are_not_supported() {
    for n in ["", "nonsense", "llama", "kimi2", "mistral-large", "qwen3"] {
        assert!(!is_supported_agent_name(n), "{n:?} should not be supported");
    }
}

// REQ-002: every new alias normalizes to its canonical key via the mcp-bridge wrapper.
#[test]
fn bridge_normalizes_every_new_alias() {
    for (alias, canon) in NEW_ALIASES {
        assert_eq!(normalize_agent_name(alias), *canon, "alias {alias}");
    }
}

// REQ-002: every new alias normalizes to its canonical key via shared_types (the canonical normalizer).
#[test]
fn shared_types_normalizes_every_new_alias() {
    for (alias, canon) in NEW_ALIASES {
        assert_eq!(shared_types::normalize_agent_name(alias), *canon, "alias {alias}");
    }
}

// REQ-002 reality_test: the three named examples, exact.
#[test]
fn reality_test_examples() {
    assert_eq!(normalize_agent_name("vibe"), "mistral");
    assert_eq!(normalize_agent_name("moonshot"), "kimi");
    assert_eq!(normalize_agent_name("bailian"), "qwen");
    assert_eq!(shared_types::normalize_agent_name("vibe"), "mistral");
    assert_eq!(shared_types::normalize_agent_name("moonshot"), "kimi");
    assert_eq!(shared_types::normalize_agent_name("bailian"), "qwen");
}

// REQ-002 (both normalizers): the two normalizers agree on every alias and key.
#[test]
fn both_normalizers_agree() {
    let mut inputs: Vec<&str> = EIGHT.to_vec();
    inputs.extend(NEW_ALIASES.iter().map(|(a, _)| *a));
    inputs.extend(EXISTING_ALIASES.iter().map(|(a, _)| *a));
    inputs.extend(["nonsense", "", "unknown-agent"]);
    for i in inputs {
        assert_eq!(
            normalize_agent_name(i),
            shared_types::normalize_agent_name(i),
            "input {i:?}"
        );
    }
}

// Existing aliases keep working (Antigravity/agy -> gemini, grok aliases -> grok).
#[test]
fn existing_aliases_still_normalize() {
    for (alias, canon) in EXISTING_ALIASES {
        assert_eq!(normalize_agent_name(alias), *canon, "alias {alias}");
        assert_eq!(shared_types::normalize_agent_name(alias), *canon, "alias {alias}");
    }
}

// REQ-002: canonical keys normalize to themselves.
#[test]
fn canonical_keys_are_fixed_points() {
    for k in EIGHT {
        assert_eq!(normalize_agent_name(k), *k);
        assert_eq!(shared_types::normalize_agent_name(k), *k);
    }
}

// REQ-002: normalization is idempotent on every alias.
#[test]
fn normalization_is_idempotent() {
    for (alias, _) in NEW_ALIASES.iter().chain(EXISTING_ALIASES.iter()) {
        let once = normalize_agent_name(alias);
        assert_eq!(normalize_agent_name(&once), once);
    }
}

// Normalizer contract: everything else passes through lowercased.
#[test]
fn non_aliases_pass_through_lowercased() {
    assert_eq!(normalize_agent_name("CODEX"), "codex");
    assert_eq!(normalize_agent_name("some-unknown"), "some-unknown");
    assert_eq!(shared_types::normalize_agent_name("CODEX"), "codex");
    assert_eq!(shared_types::normalize_agent_name("Some-Unknown"), "some-unknown");
}

// REQ-002 test clause: AGENT_ALIASES holds every new alias with the exact target.
#[test]
fn alias_table_contains_every_new_pair() {
    for pair in NEW_ALIASES.iter().chain(EXISTING_ALIASES.iter()) {
        assert!(
            shared_types::AGENT_ALIASES.iter().any(|p| p == pair),
            "AGENT_ALIASES missing {pair:?}"
        );
    }
}

// REQ-002 test clause: no alias collides with a canonical key.
#[test]
fn no_alias_collides_with_a_canonical_key() {
    for (alias, _) in shared_types::AGENT_ALIASES {
        assert!(!EIGHT.contains(alias), "alias {alias} is a canonical key");
        assert!(!supported_agent_names().contains(alias), "alias {alias} is on the roster");
    }
}

// REQ-002 test clause: no alias appears twice (so none maps to two targets).
#[test]
fn no_alias_collides_with_another_alias() {
    let mut seen = HashSet::new();
    for (alias, _) in shared_types::AGENT_ALIASES {
        assert!(seen.insert(*alias), "duplicate alias {alias}");
    }
}

// REQ-002 / REQ-001: every alias target is a roster seat, and the table has exactly the 14 spec'd pairs.
#[test]
fn alias_targets_are_roster_seats_and_table_is_exact() {
    for (alias, target) in shared_types::AGENT_ALIASES {
        assert!(supported_agent_names().contains(target), "{alias} -> {target} not on roster");
    }
    assert_eq!(shared_types::AGENT_ALIASES.len(), 14);
}

// REQ-002: every alias in the table agrees with the normalizer (table and normalizer cannot disagree).
#[test]
fn table_and_normalizer_agree() {
    for (alias, target) in shared_types::AGENT_ALIASES {
        assert_eq!(shared_types::normalize_agent_name(alias), *target);
        assert_eq!(normalize_agent_name(alias), *target);
    }
}

// REQ-002: aliases resolve to a supported agent after normalization.
#[test]
fn normalized_aliases_are_supported() {
    for (alias, _) in NEW_ALIASES {
        assert!(is_supported_agent_name(&normalize_agent_name(alias)));
    }
}

// REQ-003: display names for the three new seats, exact.
#[test]
fn display_names_for_new_seats() {
    assert_eq!(display_agent_name("kimi"), "Kimi");
    assert_eq!(display_agent_name("mistral"), "Mistral");
    assert_eq!(display_agent_name("qwen"), "Qwen");
}

// REQ-003: display names are distinct and non-empty across the whole roster.
#[test]
fn display_names_are_distinct_and_non_empty() {
    let mut seen = HashSet::new();
    for n in supported_agent_names() {
        let d = display_agent_name(n);
        assert!(!d.trim().is_empty(), "empty display name for {n}");
        assert!(seen.insert(d.clone()), "duplicate display name {d}");
    }
}
