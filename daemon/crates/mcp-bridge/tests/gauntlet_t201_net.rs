// GAUNTLET net suite for T-201, written blind from the spec (no implementation read).
//
// SPEC_AMBIGUITY: supported_agent_names() exact membership beyond the three new keys (kimi, mistral, qwen) is pinned by the
// T-004 census, not by this spec. This suite asserts membership of the new keys and uniqueness only.
// SPEC_AMBIGUITY: the spec does not say whether is_supported_agent_name accepts an alias (for example "vibe"). Only canonical
// keys and unknown names are asserted. Aliases are asserted through normalize_agent_name and then is_supported on the target.
// SPEC_AMBIGUITY: the spec does not say what display_agent_name returns for an alias or for an existing seat. Only the three
// new canonical keys are asserted (REQ-003).
// SPEC_AMBIGUITY: REQ-004 (tool description rendering, whitespace-normalized hand-written-list scan, daemon-target error text)
// lives in triumvirate/src/main.rs and mcp-tools, outside the mcp_bridge and shared_types public API named by the scope. Not tested here.
// SPEC_AMBIGUITY: the spec does not say how surrounding whitespace is handled by normalize_agent_name. Not tested.

use mcp_bridge::{display_agent_name, is_supported_agent_name, normalize_agent_name, supported_agent_names};
use shared_types::AGENT_ALIASES;

// REQ-001: the canonical key kimi is on the roster.
#[test]
fn roster_contains_kimi() {
    assert!(supported_agent_names().contains(&"kimi"));
}

// REQ-001: the canonical key mistral is on the roster.
#[test]
fn roster_contains_mistral() {
    assert!(supported_agent_names().contains(&"mistral"));
}

// REQ-001: the canonical key qwen is on the roster.
#[test]
fn roster_contains_qwen() {
    assert!(supported_agent_names().contains(&"qwen"));
}

// REQ-010 (roster keys unique): no canonical key appears twice on the roster.
#[test]
fn roster_has_no_duplicate_keys() {
    let names = supported_agent_names();
    let mut sorted: Vec<&str> = names.to_vec();
    sorted.sort_unstable();
    let before = sorted.len();
    sorted.dedup();
    assert_eq!(before, sorted.len(), "duplicate roster key in {:?}", names);
}

// REQ-001: is_supported_agent_name is true for each new canonical key.
#[test]
fn is_supported_true_for_new_canonical_keys() {
    assert!(is_supported_agent_name("kimi"));
    assert!(is_supported_agent_name("mistral"));
    assert!(is_supported_agent_name("qwen"));
}

// REQ-001 (validates against the roster): every roster entry is supported.
#[test]
fn is_supported_true_for_every_roster_entry() {
    for name in supported_agent_names() {
        assert!(is_supported_agent_name(name), "roster entry {name} not supported");
    }
}

// REQ-001: a name that is on no roster and in no alias table is not supported.
#[test]
fn is_supported_false_for_unknown_name() {
    assert!(!is_supported_agent_name("frobnicator"));
}

// REQ-002: the reality-test alias "vibe" normalizes to mistral.
#[test]
fn alias_vibe_normalizes_to_mistral() {
    assert_eq!(normalize_agent_name("vibe"), "mistral");
}

// REQ-002: the reality-test alias "moonshot" normalizes to kimi.
#[test]
fn alias_moonshot_normalizes_to_kimi() {
    assert_eq!(normalize_agent_name("moonshot"), "kimi");
}

// REQ-002: the reality-test alias "bailian" normalizes to qwen.
#[test]
fn alias_bailian_normalizes_to_qwen() {
    assert_eq!(normalize_agent_name("bailian"), "qwen");
}

// REQ-002: kimi-code normalizes to kimi (exact value from the REQ-002 alias list).
#[test]
fn alias_kimi_code_normalizes_to_kimi() {
    assert_eq!(normalize_agent_name("kimi-code"), "kimi");
}

// REQ-002: mistral-vibe normalizes to mistral (exact value from the REQ-002 alias list).
#[test]
fn alias_mistral_vibe_normalizes_to_mistral() {
    assert_eq!(normalize_agent_name("mistral-vibe"), "mistral");
}

// REQ-002: devstral normalizes to mistral (exact value from the REQ-002 alias list).
#[test]
fn alias_devstral_normalizes_to_mistral() {
    assert_eq!(normalize_agent_name("devstral"), "mistral");
}

// REQ-002: le-chat normalizes to mistral (exact value from the REQ-002 alias list).
#[test]
fn alias_le_chat_normalizes_to_mistral() {
    assert_eq!(normalize_agent_name("le-chat"), "mistral");
}

// REQ-002: qwen-code normalizes to qwen (exact value from the REQ-002 alias list).
#[test]
fn alias_qwen_code_normalizes_to_qwen() {
    assert_eq!(normalize_agent_name("qwen-code"), "qwen");
}

// REQ-002: alibaba normalizes to qwen (exact value from the REQ-002 alias list).
#[test]
fn alias_alibaba_normalizes_to_qwen() {
    assert_eq!(normalize_agent_name("alibaba"), "qwen");
}

// REQ-002: every row of the shared alias table normalizes to its named canonical key.
#[test]
fn every_alias_table_row_normalizes_to_its_target() {
    for (alias, canonical) in AGENT_ALIASES {
        assert_eq!(normalize_agent_name(alias), *canonical, "alias {alias}");
    }
}

// REQ-002: every alias target is a roster key, so an alias can never name a seat that is not dispatchable.
#[test]
fn every_alias_target_is_on_the_roster() {
    for (alias, canonical) in AGENT_ALIASES {
        assert!(
            supported_agent_names().contains(canonical),
            "alias {alias} targets {canonical}, which is not on the roster"
        );
    }
}

// REQ-002 (no collision): no alias is itself a canonical roster key.
#[test]
fn no_alias_collides_with_a_canonical_key() {
    for (alias, _) in AGENT_ALIASES {
        assert!(
            !supported_agent_names().contains(alias),
            "alias {alias} collides with a canonical key"
        );
    }
}

// REQ-002 (no collision): no alias appears twice in the table.
#[test]
fn no_alias_is_listed_twice() {
    let mut seen: Vec<&str> = Vec::new();
    for (alias, _) in AGENT_ALIASES {
        assert!(!seen.contains(alias), "alias {alias} listed twice");
        seen.push(alias);
    }
}

// REQ-002: the nine Seven Primes aliases are present in the shared table with their exact targets.
#[test]
fn seven_primes_aliases_present_with_exact_targets() {
    let expected = [
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
    for (alias, target) in expected {
        assert!(
            AGENT_ALIASES.iter().any(|(a, c)| *a == alias && *c == target),
            "missing alias row {alias} -> {target}"
        );
    }
}

// REQ-002: the mcp_bridge wrapper delegates, so both normalizers agree on every alias row.
#[test]
fn bridge_wrapper_agrees_with_shared_types_normalizer_on_aliases() {
    for (alias, _) in AGENT_ALIASES {
        assert_eq!(
            normalize_agent_name(alias),
            shared_types::normalize_agent_name(alias),
            "wrapper disagrees on {alias}"
        );
    }
}

// REQ-002: the mcp_bridge wrapper and shared_types normalizer agree on canonical keys too.
#[test]
fn bridge_wrapper_agrees_with_shared_types_normalizer_on_roster() {
    for name in supported_agent_names() {
        assert_eq!(
            normalize_agent_name(name),
            shared_types::normalize_agent_name(name),
            "wrapper disagrees on {name}"
        );
    }
}

// REQ-002 (canonical normalizer is idempotent): a canonical key normalizes to itself.
#[test]
fn canonical_new_keys_normalize_to_themselves() {
    assert_eq!(normalize_agent_name("kimi"), "kimi");
    assert_eq!(normalize_agent_name("mistral"), "mistral");
    assert_eq!(normalize_agent_name("qwen"), "qwen");
}

// REQ-002 (idempotence over the alias table): normalizing a normalized name changes nothing.
#[test]
fn normalization_is_idempotent_on_every_alias() {
    for (alias, _) in AGENT_ALIASES {
        let once = normalize_agent_name(alias);
        assert_eq!(normalize_agent_name(&once), once, "not idempotent for {alias}");
    }
}

// REQ-002: the spec's normalizer rule says every name not in the alias table passes through lowercased.
#[test]
fn unknown_name_passes_through_lowercased() {
    assert_eq!(normalize_agent_name("Frobnicator"), "frobnicator");
}

// REQ-002: an uppercase canonical new key is lowercased to its canonical form.
#[test]
fn uppercase_canonical_key_is_lowercased() {
    assert_eq!(normalize_agent_name("KIMI"), "kimi");
    assert_eq!(normalize_agent_name("Qwen"), "qwen");
}

// REQ-002: composition. Every alias, after normalization, is accepted by is_supported_agent_name.
#[test]
fn every_alias_normalizes_to_a_supported_agent() {
    for (alias, _) in AGENT_ALIASES {
        let canonical = normalize_agent_name(alias);
        assert!(is_supported_agent_name(&canonical), "alias {alias} -> {canonical} not supported");
    }
}

// REQ-003: display_agent_name returns Kimi for the kimi key.
#[test]
fn display_name_for_kimi_is_kimi() {
    assert_eq!(display_agent_name("kimi"), "Kimi");
}

// REQ-003: display_agent_name returns Mistral for the mistral key.
#[test]
fn display_name_for_mistral_is_mistral() {
    assert_eq!(display_agent_name("mistral"), "Mistral");
}

// REQ-003: display_agent_name returns Qwen for the qwen key.
#[test]
fn display_name_for_qwen_is_qwen() {
    assert_eq!(display_agent_name("qwen"), "Qwen");
}
