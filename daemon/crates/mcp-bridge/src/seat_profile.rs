//! The seat table: every per-agent fact the daemon dispatches on, in one typed row per seat.
//!
//! Seven Primes FEAT-001, REQ-011, REQ-012 (T-002). Today these facts are spread over match arms and string lists in
//! posthog.rs, stall.rs, agent_exec.rs, blind_validate.rs, the fleet crates, aliases.rs and abe.rs (the census in
//! docs/seven-primes/research/04-wiring-census.md). T-202 and T-204 move each consumer onto this table; until then
//! the table only DESCRIBES today's behavior, and the tests below pin it to the values T-001 characterized.
//!
//! Rules for this file:
//! - Typed values only. There is no `Unknown` or catch-all variant: a seat whose value is not known cannot get a row,
//!   which is the point. "Legacy" variants name a behavior that already exists in code and is not table-driven yet.
//! - Two rows are deliberately the D-01 TARGET, not today's value: grok's and claude's `provider` (today both report
//!   "unknown") and grok's `billing` (today "unknown"). provider_for/billing_for keep returning the old values until
//!   T-202 switches them to this table, which is the intended fix, and T-001's pins change in that commit.
//! - New seats (kimi, mistral, qwen) got rows in T-201 (REQ-001). Their surfaces follow amendment A-2: Mistral's launch
//!   set for all three. They stay unconfigured until `TRIUMVIRATE_ENABLE_<SEAT>=1`.
//!
//! Deviations from BACKEND_STRUCTURE.md section 2 (amendment A-5): `live_progress` is an enum, not a bool, and
//! `parser_mode` is `parser_modes`, a list. Both because gemini's value depends on its backend (agy buffers its
//! stdout and emits agy-stream-json; gemini-cli streams gemini-stream-json), which one bool or one string cannot say.

/// Every surface a seat can be dispatched on (REQ-090). Owned here, not in seat_gate.rs, because `surfaces_allowed`
/// needs it in T-002 and seat_gate.rs (T-003) imports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Surface {
    AskAgent,
    Session,
    Jury,
    BlindValidate,
    DoctorLiveProbe,
    PeerReviewRotation,
    FleetSpawn,
    FleetTemporal,
    AbeDispatch,
    Prewarm,
}

impl Surface {
    pub const ALL: &'static [Surface] = &[
        Surface::AskAgent,
        Surface::Session,
        Surface::Jury,
        Surface::BlindValidate,
        Surface::DoctorLiveProbe,
        Surface::PeerReviewRotation,
        Surface::FleetSpawn,
        Surface::FleetTemporal,
        Surface::AbeDispatch,
        Surface::Prewarm,
    ];
}

/// `$ai_provider` in telemetry (REQ-012, REQ-043). The strings are OpenRouter's provider slugs, because PostHog
/// matches `$ai_provider` + `$ai_model` against OpenRouter pricing first (c1526bf, D-022; amendment A-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Google,
    XAi,
    Anthropic,
    DeepSeek,
    Moonshot,
    Mistral,
    Alibaba,
}

impl Provider {
    /// The string PostHog expects.
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::Google => "google",
            Provider::XAi => "x-ai",
            Provider::Anthropic => "anthropic",
            Provider::DeepSeek => "deepseek",
            Provider::Moonshot => "moonshotai",
            Provider::Mistral => "mistralai",
            Provider::Alibaba => "qwen",
        }
    }
}

/// How a seat is paid for (REQ-012). `Metered` means "priced from DeepSeek's per-model table in posthog.rs", whose
/// unknown-model case stays UnknownPrice there: that is a property of a model, not of the seat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingClass {
    Subscription,
    Metered(DeepSeekPricing),
}

/// Marker for the DeepSeek per-model price table (posthog.rs `billing_for`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeepSeekPricing;

/// A stall window (REQ-051). `Disabled` means the seat is never stall-killed (claude today on the ask path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallWindow {
    Secs(u64),
    Disabled,
}

/// Retries for one dispatch (REQ-050). A sight-gated review is always one attempt, for every policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptPolicy {
    /// One attempt, no retry (grok, deepseek; every new seat).
    Single,
    /// gemini-cli walks GEMINI_MODEL_FAILDOWN; agy and strict calls are single.
    GeminiFaildown,
    /// Three attempts at 250 ms, 1 s, 2 s (codex, claude).
    Retry3,
}

/// What a quota or rate-limit failure does (REQ-050).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaPolicy {
    /// Terminal failure plus breaker trip, one attempt (new seats).
    TerminalTripBreaker,
    /// Each existing seat's current handling, not table-driven yet.
    Legacy,
}

/// Whether the seat's CLI emits progress while it works (REQ-051). A buffered seat would be stall-killed mid-work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveProgress {
    Streams,
    Buffered,
    /// gemini: gemini-cli streams, agy reads its whole stdout after exit.
    StreamsExceptAgy,
}

/// How a read-only run is contained (REQ-035).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Containment {
    /// macOS seatbelt plus a post-run repo audit (new seats).
    SeatbeltPlusAudit,
    /// The CLI's own sandbox flags (codex, grok, gemini, claude today).
    ExistingSandbox,
    /// No local process at all (deepseek is HTTP).
    NoLocalProcess,
}

/// Where usage numbers come from (REQ-041).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageSource {
    Exact,
    /// The seat does not report usage; persisted as `not_reported`, never as zero.
    NotReported,
    /// agy reports no token counts; the gemini seat's numbers are estimated on that backend.
    AgyEstimated,
}

/// Whether a seat is configured (REQ-091).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Configured {
    Always,
    EnvFlag(&'static str),
}

/// Whether a seat needs a passed `doctor --live` before it is available (REQ-091).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyPolicy {
    NotRequired,
    DoctorLive,
}

/// Whether provider responses are classified for account suspension (REQ-091).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspensionPolicy {
    NotClassified,
    Classified,
}

/// Concurrent processes per seat (REQ-053).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcurrencyLimit {
    /// One process per seat machine-wide, by advisory flock (new seats).
    One,
    /// No per-seat lock today (existing seats queue per project lane, not per seat).
    Unlimited,
}

/// How a named session resumes its CLI conversation (REQ-021..023).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeStrategy {
    KimiSessionFlagSameCwd,
    VibeResume,
    QwenResume,
    /// Each existing seat's current resume path.
    Legacy,
}

/// When a session id becomes known (REQ-061).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdPublication {
    AfterCompletedTurn,
    ExportJson,
    SessionStart,
    Legacy,
}

/// What a vendor rejecting a session id does (REQ-067).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invalidation {
    VendorRejectedIdMarksBroken,
    Legacy,
}

/// Where the served model name is read from (REQ-043).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelEvidence {
    /// The CLI does not say which model answered (kimi).
    NotExposed,
    /// agy reports the model it chose at runtime and the connector stashes it in `cli_version` (gemini).
    AgyRuntimeReport,
    ExportConfig,
    MessageModel,
    Legacy,
}

/// How exit code and stream together map to an outcome (REQ-052).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomePolicy {
    KimiStream,
    VibeExitCodes,
    QwenExitCodes,
    Legacy,
}

/// How the prompt reaches the CLI (REQ-024).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTransport {
    ArgvMax64KiB,
    Legacy,
}

/// Whether the daemon prewarms a worker for the seat at startup (REQ-090).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrewarmPolicy {
    Always,
    Never,
}

/// What the adapter's prompt-token counts mean (REQ-012). posthog.rs `normalize_prompt` depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptUsage {
    /// OpenAI convention: `input` is the whole prompt and includes `cached`.
    CachedSubsetOfInput,
    /// deepseek: `input` is the cache-miss count and `cached` the hit count; the prompt is their sum.
    CachedDisjoint,
}

/// One seat. Field order follows BACKEND_STRUCTURE.md section 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatProfile {
    pub key: &'static str,
    pub display_name: &'static str,
    pub provider: Provider,
    pub billing: BillingClass,
    pub stall_window: StallWindow,
    pub fleet_stall_window: StallWindow,
    pub attempt_policy: AttemptPolicy,
    pub quota_policy: QuotaPolicy,
    pub live_progress: LiveProgress,
    pub parser_modes: &'static [&'static str],
    pub records_tool_calls: bool,
    pub classifies_reads: bool,
    pub sight_repair: bool,
    pub can_write_test_file: bool,
    pub read_only_containment: Containment,
    pub usage_source: UsageSource,
    pub abe_buildable: bool,
    pub fleet_buildable: bool,
    pub daemon_target: bool,
    pub surfaces_allowed: &'static [Surface],
    pub configured: Configured,
    pub verify_policy: VerifyPolicy,
    pub suspension_policy: SuspensionPolicy,
    pub concurrency_limit: ConcurrencyLimit,
    pub resume_strategy: ResumeStrategy,
    pub session_id_publication: IdPublication,
    pub session_invalidation: Invalidation,
    pub home_env_var: Option<&'static str>,
    pub env_remove: &'static [&'static str],
    pub served_model_evidence: ModelEvidence,
    pub exit_outcome_policy: OutcomePolicy,
    pub prompt_transport: PromptTransport,
    pub prewarm_policy: PrewarmPolicy,
    pub prompt_usage: PromptUsage,
}

/// The fields every existing seat shares today. Each row below overrides what differs.
const EXISTING: SeatProfile = SeatProfile {
    key: "",
    display_name: "",
    provider: Provider::OpenAi,
    billing: BillingClass::Subscription,
    stall_window: StallWindow::Disabled,
    fleet_stall_window: StallWindow::Disabled,
    attempt_policy: AttemptPolicy::Single,
    quota_policy: QuotaPolicy::Legacy,
    live_progress: LiveProgress::Streams,
    parser_modes: &[],
    records_tool_calls: true,
    classifies_reads: true,
    sight_repair: false,
    can_write_test_file: true,
    read_only_containment: Containment::ExistingSandbox,
    usage_source: UsageSource::Exact,
    abe_buildable: false,
    fleet_buildable: false,
    daemon_target: false,
    surfaces_allowed: Surface::ALL,
    configured: Configured::Always,
    verify_policy: VerifyPolicy::NotRequired,
    suspension_policy: SuspensionPolicy::NotClassified,
    concurrency_limit: ConcurrencyLimit::Unlimited,
    resume_strategy: ResumeStrategy::Legacy,
    session_id_publication: IdPublication::Legacy,
    session_invalidation: Invalidation::Legacy,
    home_env_var: None,
    env_remove: &[],
    served_model_evidence: ModelEvidence::Legacy,
    exit_outcome_policy: OutcomePolicy::Legacy,
    prompt_transport: PromptTransport::Legacy,
    prewarm_policy: PrewarmPolicy::Never,
    prompt_usage: PromptUsage::CachedSubsetOfInput,
};

/// The launch surfaces of every new seat (amendment A-2): read-only asks, sessions, jury seats and the doctor probe.
pub const NEW_SEAT_LAUNCH_SURFACES: &[Surface] = &[
    Surface::AskAgent,
    Surface::Session,
    Surface::Jury,
    Surface::DoctorLiveProbe,
];

/// What every new seat shares at launch: off until enabled, verified by `doctor --live`, one process per seat,
/// seatbelt plus repo audit, inherited vendor credentials scrubbed, single attempt with a terminal quota breaker,
/// 780 s stall window (REQ-051 starting value), no fleet, ABE, prewarm or blind-validator role.
const NEW_SEAT: SeatProfile = SeatProfile {
    key: "",
    display_name: "",
    // A placeholder that is WRONG for every new seat, on purpose: each row must set its own provider, and a row that
    // forgets fails `new_rows_carry_the_launch_contract` instead of inheriting a plausible value.
    provider: Provider::OpenAi,
    billing: BillingClass::Subscription,
    stall_window: StallWindow::Secs(780),
    fleet_stall_window: StallWindow::Disabled,
    attempt_policy: AttemptPolicy::Single,
    quota_policy: QuotaPolicy::TerminalTripBreaker,
    live_progress: LiveProgress::Streams,
    parser_modes: &[],
    records_tool_calls: true,
    classifies_reads: true,
    sight_repair: false,
    can_write_test_file: false,
    read_only_containment: Containment::SeatbeltPlusAudit,
    usage_source: UsageSource::Exact,
    abe_buildable: false,
    fleet_buildable: false,
    daemon_target: true,
    surfaces_allowed: NEW_SEAT_LAUNCH_SURFACES,
    configured: Configured::EnvFlag(""),
    verify_policy: VerifyPolicy::DoctorLive,
    suspension_policy: SuspensionPolicy::Classified,
    concurrency_limit: ConcurrencyLimit::One,
    resume_strategy: ResumeStrategy::Legacy,
    session_id_publication: IdPublication::Legacy,
    session_invalidation: Invalidation::VendorRejectedIdMarksBroken,
    home_env_var: None,
    env_remove: crate::containment::SEAT_CREDENTIAL_SCRUB,
    served_model_evidence: ModelEvidence::Legacy,
    exit_outcome_policy: OutcomePolicy::Legacy,
    prompt_transport: PromptTransport::ArgvMax64KiB,
    prewarm_policy: PrewarmPolicy::Never,
    // Provisional for all three new seats: confirmed against live usage at T-302.
    prompt_usage: PromptUsage::CachedSubsetOfInput,
};

/// Every row, in roster order.
static SEATS: &[SeatProfile] = &[
    SeatProfile {
        key: "gemini",
        display_name: "Antigravity",
        provider: Provider::Google,
        stall_window: StallWindow::Secs(300),
        fleet_stall_window: StallWindow::Secs(300),
        attempt_policy: AttemptPolicy::GeminiFaildown,
        live_progress: LiveProgress::StreamsExceptAgy,
        parser_modes: &["agy-stream-json", "gemini-stream-json"],
        usage_source: UsageSource::AgyEstimated,
        served_model_evidence: ModelEvidence::AgyRuntimeReport,
        fleet_buildable: true,
        daemon_target: true,
        prewarm_policy: PrewarmPolicy::Always,
        ..EXISTING
    },
    SeatProfile {
        key: "codex",
        display_name: "Codex",
        provider: Provider::OpenAi,
        stall_window: StallWindow::Secs(240),
        fleet_stall_window: StallWindow::Secs(240),
        attempt_policy: AttemptPolicy::Retry3,
        parser_modes: &["codex-exec-json"],
        sight_repair: true,
        abe_buildable: true,
        fleet_buildable: true,
        daemon_target: true,
        prewarm_policy: PrewarmPolicy::Always,
        ..EXISTING
    },
    SeatProfile {
        key: "deepseek",
        display_name: "DeepSeek",
        provider: Provider::DeepSeek,
        billing: BillingClass::Metered(DeepSeekPricing),
        stall_window: StallWindow::Secs(420),
        live_progress: LiveProgress::Buffered,
        // HTTP API seat: no CLI stream, no tools, no filesystem through the bridge.
        records_tool_calls: false,
        classifies_reads: false,
        can_write_test_file: false,
        read_only_containment: Containment::NoLocalProcess,
        prompt_usage: PromptUsage::CachedDisjoint,
        ..EXISTING
    },
    SeatProfile {
        key: "claude",
        display_name: "Claude",
        // D-01 target; provider_for returns "unknown" until T-202.
        provider: Provider::Anthropic,
        attempt_policy: AttemptPolicy::Retry3,
        parser_modes: &["claude-stream-json"],
        abe_buildable: true,
        ..EXISTING
    },
    SeatProfile {
        key: "grok",
        display_name: "Grok",
        // D-01 target; provider_for and billing_for return "unknown" until T-202.
        provider: Provider::XAi,
        billing: BillingClass::Subscription,
        stall_window: StallWindow::Secs(780),
        fleet_stall_window: StallWindow::Secs(780),
        parser_modes: &["grok-streaming-json"],
        sight_repair: true,
        // grok's default consult sandbox is read-only, so it cannot write a blind validator's test file.
        can_write_test_file: false,
        abe_buildable: true,
        fleet_buildable: true,
        daemon_target: true,
        ..EXISTING
    },
    SeatProfile {
        key: "kimi",
        display_name: "Kimi",
        provider: Provider::Moonshot,
        parser_modes: &[agent_adapter::kimi::PARSER_MODE],
        // research/01: stdout carries no usage; the wire-file reader is not live-verified (REQ-041).
        usage_source: UsageSource::NotReported,
        configured: Configured::EnvFlag("TRIUMVIRATE_ENABLE_KIMI"),
        resume_strategy: ResumeStrategy::KimiSessionFlagSameCwd,
        session_id_publication: IdPublication::AfterCompletedTurn,
        home_env_var: Some("KIMI_CODE_HOME"),
        served_model_evidence: ModelEvidence::NotExposed,
        exit_outcome_policy: OutcomePolicy::KimiStream,
        ..NEW_SEAT
    },
    SeatProfile {
        key: "mistral",
        display_name: "Mistral",
        provider: Provider::Mistral,
        parser_modes: &[agent_adapter::mistral::PARSER_MODE],
        configured: Configured::EnvFlag("TRIUMVIRATE_ENABLE_MISTRAL"),
        resume_strategy: ResumeStrategy::VibeResume,
        session_id_publication: IdPublication::ExportJson,
        home_env_var: Some("VIBE_HOME"),
        served_model_evidence: ModelEvidence::ExportConfig,
        exit_outcome_policy: OutcomePolicy::VibeExitCodes,
        ..NEW_SEAT
    },
    SeatProfile {
        key: "qwen",
        display_name: "Qwen",
        provider: Provider::Alibaba,
        parser_modes: &[agent_adapter::qwen::PARSER_MODE],
        configured: Configured::EnvFlag("TRIUMVIRATE_ENABLE_QWEN"),
        resume_strategy: ResumeStrategy::QwenResume,
        session_id_publication: IdPublication::SessionStart,
        home_env_var: Some("QWEN_HOME"),
        served_model_evidence: ModelEvidence::MessageModel,
        exit_outcome_policy: OutcomePolicy::QwenExitCodes,
        ..NEW_SEAT
    },
];

/// The row for a canonical seat key, or `None` for anything else (aliases are normalized before lookup).
pub fn seat_profile(key: &str) -> Option<&'static SeatProfile> {
    SEATS.iter().find(|s| s.key == key)
}

/// Every row, in roster order.
pub fn all_seats() -> &'static [SeatProfile] {
    SEATS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str) -> &'static SeatProfile {
        seat_profile(key).unwrap_or_else(|| panic!("no row for {key}"))
    }

    /// The five existing seats, the only rows T-001's pins describe: the first five in roster order.
    fn existing_seats() -> &'static [SeatProfile] {
        let five = &all_seats()[..5];
        let keys: Vec<&str> = five.iter().map(|s| s.key).collect();
        assert_eq!(keys, ["gemini", "codex", "deepseek", "claude", "grok"]);
        five
    }

    /// RED IF: the roster gains, loses or reorders a seat (REQ-001: the five existing seats, then the three new ones).
    #[test]
    fn all_seats_is_exactly_the_eight_keys() {
        let keys: Vec<&str> = all_seats().iter().map(|s| s.key).collect();
        assert_eq!(
            keys,
            [
                "gemini", "codex", "deepseek", "claude", "grok", "kimi", "mistral", "qwen"
            ]
        );
        assert!(
            seat_profile("agy").is_none(),
            "aliases are normalized before lookup"
        );
    }

    /// The new rows carry the launch contract (REQ-001, REQ-030, REQ-041, REQ-050, REQ-051, REQ-053, REQ-091; A-2).
    /// RED IF: a new seat gains a writer, fleet, ABE or prewarm role, loses containment or the credential scrub, or
    /// becomes configured, verified or retried by default.
    #[test]
    fn new_rows_carry_the_launch_contract() {
        for (key, display, provider, home, flag) in [
            (
                "kimi",
                "Kimi",
                Provider::Moonshot,
                "KIMI_CODE_HOME",
                "TRIUMVIRATE_ENABLE_KIMI",
            ),
            (
                "mistral",
                "Mistral",
                Provider::Mistral,
                "VIBE_HOME",
                "TRIUMVIRATE_ENABLE_MISTRAL",
            ),
            (
                "qwen",
                "Qwen",
                Provider::Alibaba,
                "QWEN_HOME",
                "TRIUMVIRATE_ENABLE_QWEN",
            ),
        ] {
            let s = row(key);
            assert_eq!(
                (s.display_name, s.provider, s.home_env_var),
                (display, provider, Some(home)),
                "{key}"
            );
            assert_eq!(s.configured, Configured::EnvFlag(flag), "{key}");
            assert_eq!(s.surfaces_allowed, NEW_SEAT_LAUNCH_SURFACES, "{key}");
            assert_eq!(s.stall_window, StallWindow::Secs(780), "{key}");
            assert_eq!(s.fleet_stall_window, StallWindow::Disabled, "{key}");
            assert_eq!(s.attempt_policy, AttemptPolicy::Single, "{key}");
            assert_eq!(s.quota_policy, QuotaPolicy::TerminalTripBreaker, "{key}");
            assert_eq!(s.verify_policy, VerifyPolicy::DoctorLive, "{key}");
            assert_eq!(s.suspension_policy, SuspensionPolicy::Classified, "{key}");
            assert_eq!(s.concurrency_limit, ConcurrencyLimit::One, "{key}");
            assert_eq!(
                s.read_only_containment,
                Containment::SeatbeltPlusAudit,
                "{key}"
            );
            assert_eq!(
                s.env_remove,
                crate::containment::SEAT_CREDENTIAL_SCRUB,
                "{key}"
            );
            assert_eq!(s.prompt_transport, PromptTransport::ArgvMax64KiB, "{key}");
            assert!(
                !s.can_write_test_file && !s.abe_buildable && !s.fleet_buildable,
                "{key}"
            );
            assert_eq!(s.prewarm_policy, PrewarmPolicy::Never, "{key}");
        }
        assert_eq!(row("kimi").usage_source, UsageSource::NotReported);
        assert_eq!(row("mistral").usage_source, UsageSource::Exact);
        assert_eq!(row("qwen").usage_source, UsageSource::Exact);
        let mode = |k| row(k).parser_modes;
        assert_eq!(
            [mode("kimi"), mode("mistral"), mode("qwen")],
            [
                &["kimi-stream-json"][..],
                &["mistral-streaming"][..],
                &["qwen-stream-json"][..]
            ]
        );
    }

    /// Every row agrees field by field with what T-001 pinned at the census sites (posthog.rs, stall.rs, agent_exec.rs,
    /// blind_validate.rs, fleet-temporal worker.rs, fleet orchestrator.rs, aliases.rs, jury and fleet defaults), and
    /// with abe.rs's worker arms and agent_exec.rs's prewarm list.
    /// RED IF: any row value drifts from today's behavior (apart from the three D-01 targets in the module doc).
    #[test]
    fn rows_reproduce_the_t001_pins() {
        use StallWindow::{Disabled, Secs};
        let stall = |k| row(k).stall_window;
        assert_eq!(
            [
                stall("codex"),
                stall("gemini"),
                stall("deepseek"),
                stall("grok"),
                stall("claude")
            ],
            [Secs(240), Secs(300), Secs(420), Secs(780), Disabled]
        );
        let fleet = |k| row(k).fleet_stall_window;
        assert_eq!(
            [
                fleet("codex"),
                fleet("gemini"),
                fleet("grok"),
                fleet("deepseek"),
                fleet("claude")
            ],
            [Secs(240), Secs(300), Secs(780), Disabled, Disabled]
        );

        assert_eq!(row("gemini").provider.as_str(), "google");
        assert_eq!(row("codex").provider.as_str(), "openai");
        assert_eq!(row("deepseek").provider.as_str(), "deepseek");
        assert_eq!(row("grok").provider.as_str(), "x-ai", "D-01 target");
        assert_eq!(row("claude").provider.as_str(), "anthropic", "D-01 target");

        assert_eq!(
            row("deepseek").billing,
            BillingClass::Metered(DeepSeekPricing)
        );
        for sub in ["gemini", "codex", "claude", "grok"] {
            assert_eq!(row(sub).billing, BillingClass::Subscription, "{sub}");
        }

        assert_eq!(row("deepseek").prompt_usage, PromptUsage::CachedDisjoint);
        for subset in ["gemini", "codex", "claude", "grok"] {
            assert_eq!(
                row(subset).prompt_usage,
                PromptUsage::CachedSubsetOfInput,
                "{subset}"
            );
        }

        assert_eq!(row("gemini").attempt_policy, AttemptPolicy::GeminiFaildown);
        assert_eq!(row("codex").attempt_policy, AttemptPolicy::Retry3);
        assert_eq!(row("claude").attempt_policy, AttemptPolicy::Retry3);
        assert_eq!(row("deepseek").attempt_policy, AttemptPolicy::Single);
        assert_eq!(row("grok").attempt_policy, AttemptPolicy::Single);

        assert_eq!(row("gemini").live_progress, LiveProgress::StreamsExceptAgy);
        assert_eq!(row("deepseek").live_progress, LiveProgress::Buffered);
        for live in ["codex", "grok", "claude"] {
            assert_eq!(row(live).live_progress, LiveProgress::Streams, "{live}");
        }

        // The parser-mode allowlists in agent_exec.rs are exactly the modes of seats that record tool calls and
        // classify reads (set equality: the consts' order is pinned by T-001 itself).
        let modes = |pick: fn(&SeatProfile) -> bool| {
            let mut m: Vec<&str> = existing_seats()
                .iter()
                .filter(|s| pick(s))
                .flat_map(|s| s.parser_modes.iter().copied())
                .collect();
            m.sort_unstable();
            m
        };
        let want = [
            "agy-stream-json",
            "claude-stream-json",
            "codex-exec-json",
            "gemini-stream-json",
            "grok-streaming-json",
        ];
        assert_eq!(modes(|s| s.records_tool_calls), want);
        assert_eq!(modes(|s| s.classifies_reads), want);

        let keys = |pick: fn(&SeatProfile) -> bool| -> Vec<&str> {
            existing_seats()
                .iter()
                .filter(|s| pick(s))
                .map(|s| s.key)
                .collect()
        };
        assert_eq!(keys(|s| s.sight_repair), ["codex", "grok"]);
        assert_eq!(keys(|s| !s.can_write_test_file), ["deepseek", "grok"]);
        assert_eq!(keys(|s| s.daemon_target), ["gemini", "codex", "grok"]);
        assert_eq!(keys(|s| s.fleet_buildable), ["gemini", "codex", "grok"]);
        assert_eq!(keys(|s| s.abe_buildable), ["codex", "claude", "grok"]);
        assert_eq!(
            keys(|s| s.prewarm_policy == PrewarmPolicy::Always),
            ["gemini", "codex"]
        );

        let display: Vec<&str> = existing_seats().iter().map(|s| s.display_name).collect();
        assert_eq!(
            display,
            ["Antigravity", "Codex", "DeepSeek", "Claude", "Grok"]
        );

        // Existing seats run everywhere, need no verification, and are not locked per seat today.
        for s in existing_seats() {
            assert_eq!(s.surfaces_allowed, Surface::ALL, "{}", s.key);
            assert_eq!(s.configured, Configured::Always, "{}", s.key);
            assert_eq!(s.verify_policy, VerifyPolicy::NotRequired, "{}", s.key);
            assert_eq!(
                s.concurrency_limit,
                ConcurrencyLimit::Unlimited,
                "{}",
                s.key
            );
            assert_eq!(s.home_env_var, None, "{}", s.key);
        }
    }

    /// Every row pinned WHOLE (retroactive P5.3, T-002 and T-004): each expected row names every field with no `..`
    /// spread, so adding a SeatProfile field fails to compile here until a value is decided for every seat (the
    /// "every field explicit" contract), and changing any value, including one inherited from EXISTING or NEW_SEAT,
    /// fails at runtime. Parser modes are written as literals, independent of the adapters' PARSER_MODE consts.
    /// RED IF: any field of any row changes, a row is added or removed, or the roster order changes.
    #[test]
    fn every_row_is_pinned_whole() {
        let expected = [
            SeatProfile {
                key: "gemini",
                display_name: "Antigravity",
                provider: Provider::Google,
                billing: BillingClass::Subscription,
                stall_window: StallWindow::Secs(300),
                fleet_stall_window: StallWindow::Secs(300),
                attempt_policy: AttemptPolicy::GeminiFaildown,
                quota_policy: QuotaPolicy::Legacy,
                live_progress: LiveProgress::StreamsExceptAgy,
                parser_modes: &["agy-stream-json", "gemini-stream-json"],
                records_tool_calls: true,
                classifies_reads: true,
                sight_repair: false,
                can_write_test_file: true,
                read_only_containment: Containment::ExistingSandbox,
                usage_source: UsageSource::AgyEstimated,
                abe_buildable: false,
                fleet_buildable: true,
                daemon_target: true,
                surfaces_allowed: Surface::ALL,
                configured: Configured::Always,
                verify_policy: VerifyPolicy::NotRequired,
                suspension_policy: SuspensionPolicy::NotClassified,
                concurrency_limit: ConcurrencyLimit::Unlimited,
                resume_strategy: ResumeStrategy::Legacy,
                session_id_publication: IdPublication::Legacy,
                session_invalidation: Invalidation::Legacy,
                home_env_var: None,
                env_remove: &[],
                served_model_evidence: ModelEvidence::AgyRuntimeReport,
                exit_outcome_policy: OutcomePolicy::Legacy,
                prompt_transport: PromptTransport::Legacy,
                prewarm_policy: PrewarmPolicy::Always,
                prompt_usage: PromptUsage::CachedSubsetOfInput,
            },
            SeatProfile {
                key: "codex",
                display_name: "Codex",
                provider: Provider::OpenAi,
                billing: BillingClass::Subscription,
                stall_window: StallWindow::Secs(240),
                fleet_stall_window: StallWindow::Secs(240),
                attempt_policy: AttemptPolicy::Retry3,
                quota_policy: QuotaPolicy::Legacy,
                live_progress: LiveProgress::Streams,
                parser_modes: &["codex-exec-json"],
                records_tool_calls: true,
                classifies_reads: true,
                sight_repair: true,
                can_write_test_file: true,
                read_only_containment: Containment::ExistingSandbox,
                usage_source: UsageSource::Exact,
                abe_buildable: true,
                fleet_buildable: true,
                daemon_target: true,
                surfaces_allowed: Surface::ALL,
                configured: Configured::Always,
                verify_policy: VerifyPolicy::NotRequired,
                suspension_policy: SuspensionPolicy::NotClassified,
                concurrency_limit: ConcurrencyLimit::Unlimited,
                resume_strategy: ResumeStrategy::Legacy,
                session_id_publication: IdPublication::Legacy,
                session_invalidation: Invalidation::Legacy,
                home_env_var: None,
                env_remove: &[],
                served_model_evidence: ModelEvidence::Legacy,
                exit_outcome_policy: OutcomePolicy::Legacy,
                prompt_transport: PromptTransport::Legacy,
                prewarm_policy: PrewarmPolicy::Always,
                prompt_usage: PromptUsage::CachedSubsetOfInput,
            },
            SeatProfile {
                key: "deepseek",
                display_name: "DeepSeek",
                provider: Provider::DeepSeek,
                billing: BillingClass::Metered(DeepSeekPricing),
                stall_window: StallWindow::Secs(420),
                fleet_stall_window: StallWindow::Disabled,
                attempt_policy: AttemptPolicy::Single,
                quota_policy: QuotaPolicy::Legacy,
                live_progress: LiveProgress::Buffered,
                parser_modes: &[],
                records_tool_calls: false,
                classifies_reads: false,
                sight_repair: false,
                can_write_test_file: false,
                read_only_containment: Containment::NoLocalProcess,
                usage_source: UsageSource::Exact,
                abe_buildable: false,
                fleet_buildable: false,
                daemon_target: false,
                surfaces_allowed: Surface::ALL,
                configured: Configured::Always,
                verify_policy: VerifyPolicy::NotRequired,
                suspension_policy: SuspensionPolicy::NotClassified,
                concurrency_limit: ConcurrencyLimit::Unlimited,
                resume_strategy: ResumeStrategy::Legacy,
                session_id_publication: IdPublication::Legacy,
                session_invalidation: Invalidation::Legacy,
                home_env_var: None,
                env_remove: &[],
                served_model_evidence: ModelEvidence::Legacy,
                exit_outcome_policy: OutcomePolicy::Legacy,
                prompt_transport: PromptTransport::Legacy,
                prewarm_policy: PrewarmPolicy::Never,
                prompt_usage: PromptUsage::CachedDisjoint,
            },
            SeatProfile {
                key: "claude",
                display_name: "Claude",
                provider: Provider::Anthropic,
                billing: BillingClass::Subscription,
                stall_window: StallWindow::Disabled,
                fleet_stall_window: StallWindow::Disabled,
                attempt_policy: AttemptPolicy::Retry3,
                quota_policy: QuotaPolicy::Legacy,
                live_progress: LiveProgress::Streams,
                parser_modes: &["claude-stream-json"],
                records_tool_calls: true,
                classifies_reads: true,
                sight_repair: false,
                can_write_test_file: true,
                read_only_containment: Containment::ExistingSandbox,
                usage_source: UsageSource::Exact,
                abe_buildable: true,
                fleet_buildable: false,
                daemon_target: false,
                surfaces_allowed: Surface::ALL,
                configured: Configured::Always,
                verify_policy: VerifyPolicy::NotRequired,
                suspension_policy: SuspensionPolicy::NotClassified,
                concurrency_limit: ConcurrencyLimit::Unlimited,
                resume_strategy: ResumeStrategy::Legacy,
                session_id_publication: IdPublication::Legacy,
                session_invalidation: Invalidation::Legacy,
                home_env_var: None,
                env_remove: &[],
                served_model_evidence: ModelEvidence::Legacy,
                exit_outcome_policy: OutcomePolicy::Legacy,
                prompt_transport: PromptTransport::Legacy,
                prewarm_policy: PrewarmPolicy::Never,
                prompt_usage: PromptUsage::CachedSubsetOfInput,
            },
            SeatProfile {
                key: "grok",
                display_name: "Grok",
                provider: Provider::XAi,
                billing: BillingClass::Subscription,
                stall_window: StallWindow::Secs(780),
                fleet_stall_window: StallWindow::Secs(780),
                attempt_policy: AttemptPolicy::Single,
                quota_policy: QuotaPolicy::Legacy,
                live_progress: LiveProgress::Streams,
                parser_modes: &["grok-streaming-json"],
                records_tool_calls: true,
                classifies_reads: true,
                sight_repair: true,
                can_write_test_file: false,
                read_only_containment: Containment::ExistingSandbox,
                usage_source: UsageSource::Exact,
                abe_buildable: true,
                fleet_buildable: true,
                daemon_target: true,
                surfaces_allowed: Surface::ALL,
                configured: Configured::Always,
                verify_policy: VerifyPolicy::NotRequired,
                suspension_policy: SuspensionPolicy::NotClassified,
                concurrency_limit: ConcurrencyLimit::Unlimited,
                resume_strategy: ResumeStrategy::Legacy,
                session_id_publication: IdPublication::Legacy,
                session_invalidation: Invalidation::Legacy,
                home_env_var: None,
                env_remove: &[],
                served_model_evidence: ModelEvidence::Legacy,
                exit_outcome_policy: OutcomePolicy::Legacy,
                prompt_transport: PromptTransport::Legacy,
                prewarm_policy: PrewarmPolicy::Never,
                prompt_usage: PromptUsage::CachedSubsetOfInput,
            },
            SeatProfile {
                key: "kimi",
                display_name: "Kimi",
                provider: Provider::Moonshot,
                billing: BillingClass::Subscription,
                stall_window: StallWindow::Secs(780),
                fleet_stall_window: StallWindow::Disabled,
                attempt_policy: AttemptPolicy::Single,
                quota_policy: QuotaPolicy::TerminalTripBreaker,
                live_progress: LiveProgress::Streams,
                parser_modes: &["kimi-stream-json"],
                records_tool_calls: true,
                classifies_reads: true,
                sight_repair: false,
                can_write_test_file: false,
                read_only_containment: Containment::SeatbeltPlusAudit,
                usage_source: UsageSource::NotReported,
                abe_buildable: false,
                fleet_buildable: false,
                daemon_target: true,
                surfaces_allowed: NEW_SEAT_LAUNCH_SURFACES,
                configured: Configured::EnvFlag("TRIUMVIRATE_ENABLE_KIMI"),
                verify_policy: VerifyPolicy::DoctorLive,
                suspension_policy: SuspensionPolicy::Classified,
                concurrency_limit: ConcurrencyLimit::One,
                resume_strategy: ResumeStrategy::KimiSessionFlagSameCwd,
                session_id_publication: IdPublication::AfterCompletedTurn,
                session_invalidation: Invalidation::VendorRejectedIdMarksBroken,
                home_env_var: Some("KIMI_CODE_HOME"),
                env_remove: crate::containment::SEAT_CREDENTIAL_SCRUB,
                served_model_evidence: ModelEvidence::NotExposed,
                exit_outcome_policy: OutcomePolicy::KimiStream,
                prompt_transport: PromptTransport::ArgvMax64KiB,
                prewarm_policy: PrewarmPolicy::Never,
                prompt_usage: PromptUsage::CachedSubsetOfInput,
            },
            SeatProfile {
                key: "mistral",
                display_name: "Mistral",
                provider: Provider::Mistral,
                billing: BillingClass::Subscription,
                stall_window: StallWindow::Secs(780),
                fleet_stall_window: StallWindow::Disabled,
                attempt_policy: AttemptPolicy::Single,
                quota_policy: QuotaPolicy::TerminalTripBreaker,
                live_progress: LiveProgress::Streams,
                parser_modes: &["mistral-streaming"],
                records_tool_calls: true,
                classifies_reads: true,
                sight_repair: false,
                can_write_test_file: false,
                read_only_containment: Containment::SeatbeltPlusAudit,
                usage_source: UsageSource::Exact,
                abe_buildable: false,
                fleet_buildable: false,
                daemon_target: true,
                surfaces_allowed: NEW_SEAT_LAUNCH_SURFACES,
                configured: Configured::EnvFlag("TRIUMVIRATE_ENABLE_MISTRAL"),
                verify_policy: VerifyPolicy::DoctorLive,
                suspension_policy: SuspensionPolicy::Classified,
                concurrency_limit: ConcurrencyLimit::One,
                resume_strategy: ResumeStrategy::VibeResume,
                session_id_publication: IdPublication::ExportJson,
                session_invalidation: Invalidation::VendorRejectedIdMarksBroken,
                home_env_var: Some("VIBE_HOME"),
                env_remove: crate::containment::SEAT_CREDENTIAL_SCRUB,
                served_model_evidence: ModelEvidence::ExportConfig,
                exit_outcome_policy: OutcomePolicy::VibeExitCodes,
                prompt_transport: PromptTransport::ArgvMax64KiB,
                prewarm_policy: PrewarmPolicy::Never,
                prompt_usage: PromptUsage::CachedSubsetOfInput,
            },
            SeatProfile {
                key: "qwen",
                display_name: "Qwen",
                provider: Provider::Alibaba,
                billing: BillingClass::Subscription,
                stall_window: StallWindow::Secs(780),
                fleet_stall_window: StallWindow::Disabled,
                attempt_policy: AttemptPolicy::Single,
                quota_policy: QuotaPolicy::TerminalTripBreaker,
                live_progress: LiveProgress::Streams,
                parser_modes: &["qwen-stream-json"],
                records_tool_calls: true,
                classifies_reads: true,
                sight_repair: false,
                can_write_test_file: false,
                read_only_containment: Containment::SeatbeltPlusAudit,
                usage_source: UsageSource::Exact,
                abe_buildable: false,
                fleet_buildable: false,
                daemon_target: true,
                surfaces_allowed: NEW_SEAT_LAUNCH_SURFACES,
                configured: Configured::EnvFlag("TRIUMVIRATE_ENABLE_QWEN"),
                verify_policy: VerifyPolicy::DoctorLive,
                suspension_policy: SuspensionPolicy::Classified,
                concurrency_limit: ConcurrencyLimit::One,
                resume_strategy: ResumeStrategy::QwenResume,
                session_id_publication: IdPublication::SessionStart,
                session_invalidation: Invalidation::VendorRejectedIdMarksBroken,
                home_env_var: Some("QWEN_HOME"),
                env_remove: crate::containment::SEAT_CREDENTIAL_SCRUB,
                served_model_evidence: ModelEvidence::MessageModel,
                exit_outcome_policy: OutcomePolicy::QwenExitCodes,
                prompt_transport: PromptTransport::ArgvMax64KiB,
                prewarm_policy: PrewarmPolicy::Never,
                prompt_usage: PromptUsage::CachedSubsetOfInput,
            },
        ];
        assert_eq!(all_seats().len(), expected.len());
        for (got, want) in all_seats().iter().zip(expected.iter()) {
            assert_eq!(got, want, "row {}", want.key);
        }
    }

    /// Every provider string is the one PostHog charts.
    #[test]
    fn provider_strings() {
        let all = [
            (Provider::OpenAi, "openai"),
            (Provider::Google, "google"),
            (Provider::XAi, "x-ai"),
            (Provider::Anthropic, "anthropic"),
            (Provider::DeepSeek, "deepseek"),
            (Provider::Moonshot, "moonshotai"),
            (Provider::Mistral, "mistralai"),
            (Provider::Alibaba, "qwen"),
        ];
        for (p, s) in all {
            assert_eq!(p.as_str(), s);
        }
    }

    /// The census (REQ-010, T-004): every roster key has exactly one row, every row is on the roster, keys are unique.
    /// Takes the roster as an argument, which is the test-only injection hook the meta-test below uses.
    fn census(roster: &[&str]) -> Vec<String> {
        let mut problems = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for key in roster {
            if !seen.insert(*key) {
                problems.push(format!("duplicate roster key: {key}"));
            }
            match all_seats().iter().filter(|s| s.key == *key).count() {
                0 => problems.push(format!("missing SeatProfile row: {key}")),
                1 => {}
                n => problems.push(format!("duplicate SeatProfile rows ({n}): {key}")),
            }
        }
        for s in all_seats() {
            if !roster.contains(&s.key) {
                problems.push(format!("SeatProfile row not on the roster: {}", s.key));
            }
        }
        problems
    }

    /// RED IF: the roster (`supported_agent_names`) and the table disagree in either direction.
    #[test]
    fn census_roster_equals_profile_keys() {
        assert_eq!(census(crate::supported_agent_names()), Vec::<String>::new());
    }

    /// The census must catch a missing row for ANY name, not a fixed one: the injected key is random per run.
    /// RED IF: the census special-cases names, or stops checking the roster against the table.
    #[test]
    fn census_meta_test_catches_a_random_missing_row() {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        );
        let injected = format!("zzz-{:016x}", h.finish());
        let mut roster: Vec<&str> = crate::supported_agent_names().to_vec();
        roster.push(&injected);
        assert_eq!(
            census(&roster),
            vec![format!("missing SeatProfile row: {injected}")]
        );
        // And the other direction: a row whose key left the roster is reported too. The dropped key is chosen at
        // run time from the same hash, so a census that special-cases one name fails.
        let names = crate::supported_agent_names();
        let dropped = names[(h.finish() as usize) % names.len()];
        let short: Vec<&str> = names.iter().copied().filter(|k| *k != dropped).collect();
        assert_eq!(
            census(&short),
            vec![format!("SeatProfile row not on the roster: {dropped}")]
        );
        // And a roster key listed twice is reported, though each occurrence finds exactly one row.
        let mut doubled: Vec<&str> = names.to_vec();
        doubled.push(dropped);
        assert_eq!(
            census(&doubled),
            vec![format!("duplicate roster key: {dropped}")]
        );
    }

    /// Every row's key is canonical (normalizing it is a no-op) and unique, so no alias can name two seats.
    /// RED IF: a row is keyed by an alias, or two rows share a key.
    #[test]
    fn row_keys_are_canonical_and_unique() {
        let keys: Vec<&str> = all_seats().iter().map(|s| s.key).collect();
        for k in &keys {
            assert_eq!(crate::normalize_agent_name(k), *k, "{k} must be canonical");
        }
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len(), "duplicate row keys: {keys:?}");
    }
}
