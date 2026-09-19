//! `ask_jury`: one brief, N seats, and nobody ever answers for a seat that could not.
//!
//! 2026-09-19: the mneme jury ran 98 parts through `ask_agent`. When the gemini seat's backend
//! hit quota, the bridge did what it is built to do for Q&A and routed those calls to codex,
//! returning success. A "unanimous" vote would have been codex agreeing with itself.
//!
//! The defence is in two layers, because the second one is the only thing that survives version
//! skew. Every seat is dispatched with `strict_agent: true`, so a current daemon fails the seat
//! instead of substituting. And every reply is checked for who actually answered, so a daemon
//! too old to know `strict_agent` (serde ignores the unknown field) still cannot get a
//! substituted vote into the tally. It lands as `invalid`.
//!
//! The fan-out lives here in the bridge and rides the existing `ask_agent` path per seat. There
//! is no daemon-side jury endpoint, on purpose: one implementation, not two.

use crate::ProgressEmitter;
use mcp_bridge::{display_agent_name, is_supported_agent_name, normalize_agent_name};
use shared_types::{
    AskAgentRequest, AskAgentResponse, AskJuryRequest, JuryMajority, JuryOutputCheck, JurySeat,
    JuryTally,
};
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    path::Path,
    time::SystemTime,
};
use tokio::time::{Duration, Instant};

pub const DEFAULT_SEATS: [&str; 3] = ["codex", "grok", "gemini"];

pub const STATUS_ANSWERED: &str = "answered";
pub const STATUS_UNAVAILABLE: &str = "unavailable";
pub const STATUS_TIMEOUT: &str = "timeout";
pub const STATUS_INVALID: &str = "invalid";

/// The canonical seats for a request. Fails before anything is spent.
pub fn resolve_seats(requested: &[String]) -> Result<Vec<String>, String> {
    let raw: Vec<String> = if requested.is_empty() {
        DEFAULT_SEATS.iter().map(|s| s.to_string()).collect()
    } else {
        requested.to_vec()
    };
    let mut seats: Vec<String> = Vec::new();
    for name in &raw {
        let canonical = normalize_agent_name(name.trim());
        if !is_supported_agent_name(&canonical) {
            return Err(format!("ask_jury: '{name}' is not a dispatchable agent"));
        }
        if seats.contains(&canonical) {
            return Err(format!(
                "ask_jury: the {canonical} seat is named twice ('{name}' is an alias of it). \
                 One agent voting twice is the failure this tool exists to prevent."
            ));
        }
        seats.push(canonical);
    }
    if seats.len() < 2 {
        return Err("ask_jury: a jury needs at least two seats; use ask_agent for one".to_string());
    }
    Ok(seats)
}

/// `outputs` re-keyed by canonical seat. A key that names no seat is an error, not a skip:
/// a typo would otherwise silently drop the verification the caller asked for.
pub fn resolve_outputs(
    outputs: &BTreeMap<String, String>,
    seats: &[String],
) -> Result<BTreeMap<String, String>, String> {
    let mut resolved = BTreeMap::new();
    for (name, path) in outputs {
        let canonical = normalize_agent_name(name.trim());
        if !seats.contains(&canonical) {
            return Err(format!("ask_jury: outputs names '{name}', which is not one of the seats {seats:?}"));
        }
        if resolved.insert(canonical.clone(), path.clone()).is_some() {
            return Err(format!("ask_jury: outputs names the {canonical} seat twice"));
        }
    }
    Ok(resolved)
}

/// How a verdict is read out of a reply.
pub enum VerdictExtractor {
    JsonPointer(String),
    Regex(regex_lite::Regex),
    FirstLine,
}

impl VerdictExtractor {
    /// A bad regex or pointer fails here, before any seat is dispatched.
    pub fn from_request(req: &AskJuryRequest) -> Result<Self, String> {
        if let Some(pointer) = req.verdict_json_pointer.as_deref().filter(|p| !p.is_empty()) {
            if !pointer.starts_with('/') {
                return Err(format!("ask_jury: verdict_json_pointer '{pointer}' must start with '/' (RFC 6901)"));
            }
            return Ok(Self::JsonPointer(pointer.to_string()));
        }
        if let Some(pattern) = req.verdict_regex.as_deref().filter(|p| !p.is_empty()) {
            return regex_lite::Regex::new(pattern)
                .map(Self::Regex)
                .map_err(|e| format!("ask_jury: verdict_regex does not compile: {e}"));
        }
        Ok(Self::FirstLine)
    }

    pub fn source(&self) -> &'static str {
        match self {
            Self::JsonPointer(_) => "json_pointer",
            Self::Regex(_) => "regex",
            Self::FirstLine => "first_line",
        }
    }

    /// The normalized verdict, or `None` when this reply does not carry one.
    pub fn extract(&self, reply: &str) -> Option<String> {
        let raw = match self {
            Self::JsonPointer(pointer) => {
                let value = parse_json_loosely(reply)?;
                match value.pointer(pointer)? {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Number(n) => n.to_string(),
                    _ => return None,
                }
            }
            Self::Regex(re) => {
                let caps = re.captures(reply)?;
                caps.get(1).or_else(|| caps.get(0))?.as_str().to_string()
            }
            Self::FirstLine => reply.lines().find(|l| !l.trim().is_empty())?.to_string(),
        };
        normalize_verdict(&raw)
    }
}

/// Lowercased, whitespace collapsed, markdown and quote wrapping and a trailing full stop
/// removed. `**APPROVE.**` and `approve` are the same vote; `approve` and `approved` are not,
/// and nothing here pretends otherwise.
pub fn normalize_verdict(raw: &str) -> Option<String> {
    let wrapping: &[char] = &['"', '\'', '`', '*', '_'];
    let trimmed = raw.trim().trim_matches(wrapping).trim_end_matches(['.', '!']).trim_matches(wrapping);
    let collapsed = trimmed.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    (!collapsed.is_empty()).then_some(collapsed)
}

/// Strict JSON first, then the outermost object in surrounding prose.
fn parse_json_loosely(text: &str) -> Option<serde_json::Value> {
    if let Ok(v) = serde_json::from_str(text.trim()) {
        return Some(v);
    }
    let (start, end) = (text.find('{')?, text.rfind('}')?);
    (start < end).then(|| serde_json::from_str(&text[start..=end]).ok()).flatten()
}

/// What an output path looked like BEFORE the seat ran.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OutputBaseline {
    existed: bool,
    modified: Option<SystemTime>,
    len: u64,
}

pub fn output_baseline(path: &Path) -> OutputBaseline {
    match std::fs::metadata(path) {
        Ok(meta) => OutputBaseline { existed: true, modified: meta.modified().ok(), len: meta.len() },
        Err(_) => OutputBaseline::default(),
    }
}

/// Counts and flags only. The contents never leave this function.
pub fn check_output(path: &str, baseline: &OutputBaseline, expect_json: bool) -> JuryOutputCheck {
    let mut check = JuryOutputCheck { path: path.to_string(), ..Default::default() };
    let now = output_baseline(Path::new(path));
    check.exists = now.existed;
    if !now.existed {
        check.error = Some("the seat did not write this file".to_string());
        return check;
    }
    check.written_this_call = &now != baseline;
    if !check.written_this_call {
        check.error = Some("the file predates this call and was not touched by it".to_string());
    }
    if !expect_json {
        return check;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            check.error = Some(format!("unreadable: {e}"));
            return check;
        }
    };
    match count_json_rows(&text) {
        Some((format, rows)) => {
            check.format = Some(format.to_string());
            check.rows = Some(rows);
        }
        None => check.error = Some("no parseable JSON in the file".to_string()),
    }
    check
}

/// `(format, rows)`. The mneme labelers write `.raw` files holding an array inside prose, which
/// is why `embedded_json` exists; it is reported as such so a caller can insist on strict JSON.
fn count_json_rows(text: &str) -> Option<(&'static str, u64)> {
    let rows_of = |v: &serde_json::Value| v.as_array().map_or(1, |a| a.len() as u64);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim()) {
        return Some(("json", rows_of(&v)));
    }
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if !lines.is_empty() && lines.iter().all(|l| serde_json::from_str::<serde_json::Value>(l).is_ok()) {
        return Some(("jsonl", lines.len() as u64));
    }
    for (open, close) in [('[', ']'), ('{', '}')] {
        if let (Some(start), Some(end)) = (text.find(open), text.rfind(close))
            && start < end
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[start..=end])
        {
            return Some(("embedded_json", rows_of(&v)));
        }
    }
    None
}

/// How one seat's dispatch ended, before it is judged.
pub enum SeatOutcome {
    Replied(Box<AskAgentResponse>),
    Failed(String),
    TimedOut(u64),
}

/// THE PROVENANCE RULE. A reply counts only when the agent that was asked is the agent that
/// answered, on the backend it was asked on.
pub fn classify_seat(
    agent: &str,
    outcome: SeatOutcome,
    duration_ms: u64,
    extractor: &VerdictExtractor,
) -> JurySeat {
    let mut seat = JurySeat { agent: agent.to_string(), duration_ms, ..Default::default() };
    let resp = match outcome {
        SeatOutcome::Failed(reason) => {
            seat.status = STATUS_UNAVAILABLE.to_string();
            seat.reason = Some(reason);
            return seat;
        }
        SeatOutcome::TimedOut(secs) => {
            seat.status = STATUS_TIMEOUT.to_string();
            seat.reason = Some(format!("no reply within {secs}s"));
            return seat;
        }
        SeatOutcome::Replied(resp) => *resp,
    };

    let answered_by = resp
        .answered_by_agent
        .as_deref()
        .map(normalize_agent_name)
        .unwrap_or_else(|| agent.to_string());
    seat.request_id = Some(resp.request_id.clone());
    seat.answered_by_agent = Some(answered_by.clone());
    seat.answered_by_backend = resp.answered_by_backend.clone();
    seat.tool_calls_made = resp.tool_calls_made;

    let invalid = if answered_by != agent {
        Some(format!("asked {agent}, answered by {answered_by}"))
    } else if normalize_agent_name(&resp.agent) != agent {
        Some(format!("asked {agent}, the reply is labelled {}", resp.agent))
    } else {
        resp.degraded_from_backend.as_deref().map(|from| {
            format!(
                "degraded from the {from} backend to {}",
                resp.answered_by_backend.as_deref().unwrap_or("another backend")
            )
        })
    };
    if let Some(why) = invalid {
        // The reply is WITHHELD. A caller that reads `.response` without reading `.status`
        // must not be able to pick up a substituted vote.
        seat.status = STATUS_INVALID.to_string();
        seat.reason = Some(format!("{why}; the reply is withheld. Is the daemon older than strict_agent?"));
        return seat;
    }

    seat.status = STATUS_ANSWERED.to_string();
    seat.verdict = extractor.extract(&resp.response);
    seat.response = Some(resp.response);
    seat
}

/// Every input lands on exactly one outcome. Counts are against seats REQUESTED.
pub fn tally(seats_requested: usize, seats: &BTreeMap<String, JurySeat>, verdict_source: &str) -> JuryTally {
    let answered: Vec<&JurySeat> = seats.values().filter(|s| s.status == STATUS_ANSWERED).collect();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for verdict in answered.iter().filter_map(|s| s.verdict.as_deref()) {
        *counts.entry(verdict).or_default() += 1;
    }
    let verdicts_cast: usize = counts.values().sum();
    let majority = counts
        .iter()
        .find(|(_, count)| **count * 2 > seats_requested)
        .map(|(verdict, count)| JuryMajority { verdict: verdict.to_string(), count: *count });
    let unanimous = seats_requested >= 2 && verdicts_cast == seats_requested && counts.len() == 1;
    let split = verdicts_cast >= 2 && majority.is_none();
    let outcome = if unanimous {
        "unanimous"
    } else if majority.is_some() {
        "majority"
    } else if split {
        "split"
    } else {
        "no_quorum"
    };
    JuryTally {
        seats_requested,
        seats_answered: answered.len(),
        verdicts_cast,
        outcome: outcome.to_string(),
        unanimous,
        majority,
        split,
        verdict_source: verdict_source.to_string(),
    }
}

/// The per-seat `ask_agent` request. `strict_agent` is not an option here.
pub fn seat_request(req: &AskJuryRequest, agent: &str) -> AskAgentRequest {
    AskAgentRequest {
        agent: agent.to_string(),
        message: req.message.clone(),
        cwd: req.cwd.clone(),
        require_sight: (req.require_sight.unwrap_or(false) || !req.required_sources.is_empty()).then_some(true),
        required_sources: req.required_sources.clone(),
        grok_depth: if agent == "grok" { req.grok_depth } else { None },
        strict_agent: Some(true),
        ..Default::default()
    }
}

pub struct JuryRun {
    pub seats: BTreeMap<String, JurySeat>,
    pub tally: JuryTally,
}

/// Dispatch every seat at once and judge each as it lands.
///
/// `on_seat` fires as each seat finishes, in completion order, BEFORE the slow seats are done.
/// That is the journal: if the caller's own client ceiling cancels this future, the seats that
/// had already reported are on disk rather than lost with the call.
pub async fn run_jury<F, Fut>(
    req: &AskJuryRequest,
    caller: Option<&str>,
    emitter: Option<&ProgressEmitter>,
    default_timeout: Duration,
    run_seat: F,
    mut on_seat: impl FnMut(&JurySeat),
) -> Result<JuryRun, String>
where
    F: Fn(AskAgentRequest) -> Fut,
    Fut: Future<Output = Result<AskAgentResponse, String>> + Send + 'static,
{
    if req.message.trim().is_empty() {
        return Err("ask_jury: message is empty".to_string());
    }
    let seat_names = resolve_seats(&req.seats)?;
    let outputs = resolve_outputs(&req.outputs, &seat_names)?;
    let extractor = VerdictExtractor::from_request(req)?;
    let expect_json = req.expect_json.unwrap_or(false);
    let seat_timeout = req.timeout_s.filter(|s| *s > 0).map(Duration::from_secs).unwrap_or(default_timeout);
    let caller = caller.map(normalize_agent_name);

    // Taken BEFORE dispatch, so a file left by an earlier run cannot pass for this one's.
    let baselines: BTreeMap<String, OutputBaseline> = outputs
        .iter()
        .map(|(seat, path)| (seat.clone(), output_baseline(Path::new(path))))
        .collect();

    let mut seats: BTreeMap<String, JurySeat> = BTreeMap::new();
    let mut tasks = tokio::task::JoinSet::new();
    let mut task_seat: HashMap<tokio::task::Id, String> = HashMap::new();

    let mut finish = |mut seat: JurySeat, seats: &mut BTreeMap<String, JurySeat>| {
        if let Some(path) = outputs.get(&seat.agent) {
            seat.output = Some(check_output(path, &baselines[&seat.agent], expect_json));
        }
        on_seat(&seat);
        seats.insert(seat.agent.clone(), seat);
    };

    for agent in &seat_names {
        if caller.as_deref() == Some(agent.as_str()) {
            let refused = SeatOutcome::Failed("the caller IS this seat; an agent cannot sit on its own jury".to_string());
            finish(classify_seat(agent, refused, 0, &extractor), &mut seats);
            continue;
        }
        let fut = run_seat(seat_request(req, agent));
        let handle = tasks.spawn(async move {
            let started = Instant::now();
            let result = tokio::time::timeout(seat_timeout, fut).await;
            (result, started.elapsed().as_millis() as u64)
        });
        task_seat.insert(handle.id(), agent.clone());
    }
    if let Some(e) = emitter {
        e.emit(format!("→ jury: {} seats dispatched", tasks.len())).await;
    }

    while let Some(joined) = tasks.join_next_with_id().await {
        let seat = match joined {
            Ok((id, (result, duration_ms))) => {
                let agent = task_seat.remove(&id).unwrap_or_default();
                let outcome = match result {
                    Ok(Ok(resp)) => SeatOutcome::Replied(Box::new(resp)),
                    Ok(Err(e)) => SeatOutcome::Failed(e),
                    Err(_) => SeatOutcome::TimedOut(seat_timeout.as_secs()),
                };
                classify_seat(&agent, outcome, duration_ms, &extractor)
            }
            Err(join_err) => {
                let agent = task_seat.remove(&join_err.id()).unwrap_or_default();
                classify_seat(&agent, SeatOutcome::Failed(format!("seat task died: {join_err}")), 0, &extractor)
            }
        };
        if let Some(e) = emitter {
            e.emit(format!("→ jury: {} {}", display_agent_name(&seat.agent), seat.status)).await;
        }
        finish(seat, &mut seats);
    }

    let tally = tally(seat_names.len(), &seats, extractor.source());
    Ok(JuryRun { seats, tally })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(agent: &str, text: &str) -> AskAgentResponse {
        AskAgentResponse::direct(format!("req-{agent}"), agent.to_string(), text.to_string(), Vec::new())
    }

    /// What a daemon WITHOUT `strict_agent` sends back when agy is over quota.
    fn substituted(asked: &str, by: &str, text: &str) -> AskAgentResponse {
        let mut r = reply(asked, text);
        r.answered_by_agent = Some(by.to_string());
        r.answered_by_backend = Some(by.to_string());
        r.degraded_from_backend = Some("agy".to_string());
        r
    }

    fn jury(seats: &[&str]) -> AskJuryRequest {
        AskJuryRequest {
            message: "label part 7".to_string(),
            seats: seats.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    async fn run(
        req: &AskJuryRequest,
        script: impl Fn(&AskAgentRequest) -> Result<AskAgentResponse, String> + Send + Sync + 'static,
    ) -> Result<JuryRun, String> {
        let script = std::sync::Arc::new(script);
        run_jury(
            req,
            None,
            None,
            Duration::from_secs(5),
            move |r: AskAgentRequest| {
                let script = script.clone();
                async move { script(&r) }
            },
            |_| {},
        )
        .await
    }

    /// BRIEF ACCEPTANCE 1. One seat's backend is forced unavailable.
    /// RED IF: the dead seat shows up as answered, `unanimous` is true on two of three, or the
    /// other two seats lose their provenance.
    #[tokio::test]
    async fn an_unavailable_seat_is_reported_unavailable_and_breaks_unanimity() {
        let out = run(&jury(&[]), |r| match r.agent.as_str() {
            "gemini" => Err("agy capacity/quota: circuit breaker open".to_string()),
            agent => Ok(reply(agent, "APPROVE")),
        })
        .await
        .expect("jury runs");

        let gemini = &out.seats["gemini"];
        assert_eq!(gemini.status, STATUS_UNAVAILABLE);
        assert!(gemini.reason.as_deref().unwrap_or_default().contains("quota"));
        assert_eq!(gemini.answered_by_agent, None, "nobody answered for it");
        assert_eq!(gemini.response, None);

        for agent in ["codex", "grok"] {
            let seat = &out.seats[agent];
            assert_eq!(seat.status, STATUS_ANSWERED);
            assert_eq!(seat.answered_by_agent.as_deref(), Some(agent));
            assert_eq!(seat.request_id.as_deref(), Some(format!("req-{agent}").as_str()));
        }
        assert!(!out.tally.unanimous, "two of three is never unanimous");
        assert_eq!(out.tally.outcome, "majority");
        assert_eq!(out.tally.majority, Some(JuryMajority { verdict: "approve".to_string(), count: 2 }));
        assert_eq!((out.tally.seats_requested, out.tally.seats_answered), (3, 2));
    }

    /// BRIEF ACCEPTANCE 2, and the version-skew defence. The runner here behaves like a daemon
    /// that ignores `strict_agent` and substitutes anyway: the 2026-09-19 response, verbatim
    /// in shape. RED IF: that reply is counted, or its text reaches the caller.
    #[tokio::test]
    async fn a_substituted_reply_is_invalid_withheld_and_never_counted() {
        let out = run(&jury(&[]), |r| match r.agent.as_str() {
            "gemini" => Ok(substituted("gemini", "codex", "APPROVE")),
            agent => Ok(reply(agent, "APPROVE")),
        })
        .await
        .expect("jury runs");

        let gemini = &out.seats["gemini"];
        assert_eq!(gemini.status, STATUS_INVALID);
        assert_eq!(gemini.answered_by_agent.as_deref(), Some("codex"), "provenance says who really answered");
        assert_eq!(gemini.response, None, "a substituted vote must not be readable as this seat's");
        assert_eq!(gemini.verdict, None);
        assert!(!out.tally.unanimous, "this is exactly the fake unanimity: codex agreeing with itself");
        assert_eq!(out.tally.seats_answered, 2);

        for seat in out.seats.values().filter(|s| s.status == STATUS_ANSWERED) {
            assert_eq!(seat.answered_by_agent.as_deref(), Some(seat.agent.as_str()));
        }
    }

    /// The three provenance checks are independent, so each is exercised ALONE. The test above
    /// trips two at once and would stay green with either one deleted.
    #[test]
    fn each_provenance_check_stands_on_its_own() {
        let first = VerdictExtractor::FirstLine;
        let judge = |resp: AskAgentResponse| classify_seat("gemini", SeatOutcome::Replied(Box::new(resp)), 1, &first);

        let mut only_answered_by = reply("gemini", "APPROVE");
        only_answered_by.answered_by_agent = Some("codex".to_string());
        assert_eq!(judge(only_answered_by).status, STATUS_INVALID);

        assert_eq!(judge(reply("codex", "APPROVE")).status, STATUS_INVALID, "a reply labelled for another agent");

        let mut only_degraded = reply("gemini", "APPROVE");
        only_degraded.degraded_from_backend = Some("agy".to_string());
        assert_eq!(judge(only_degraded).status, STATUS_INVALID);

        let mut alias = reply("gemini", "APPROVE");
        alias.answered_by_agent = Some("antigravity".to_string());
        assert_eq!(judge(alias).status, STATUS_ANSWERED, "an alias of the asked agent IS the asked agent");
    }

    /// A same-agent hop onto another backend is still not the seat that was asked.
    #[tokio::test]
    async fn a_same_agent_reply_from_a_degraded_backend_is_invalid() {
        let out = run(&jury(&["codex", "gemini"]), |r| match r.agent.as_str() {
            "gemini" => {
                let mut resp = reply("gemini", "APPROVE");
                resp.answered_by_agent = Some("gemini".to_string());
                resp.answered_by_backend = Some("gemini-cli".to_string());
                resp.degraded_from_backend = Some("agy".to_string());
                Ok(resp)
            }
            agent => Ok(reply(agent, "APPROVE")),
        })
        .await
        .expect("jury runs");
        assert_eq!(out.seats["gemini"].status, STATUS_INVALID);
        assert!(out.seats["gemini"].reason.as_deref().unwrap_or_default().contains("gemini-cli"));
    }

    /// RED IF: a seat stops being dispatched strict, which would put the whole defence on the
    /// after-the-fact check alone.
    #[tokio::test]
    async fn every_seat_is_dispatched_strict_with_the_same_brief() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        let mut req = jury(&[]);
        req.required_sources = vec!["/abs/part-7.md".to_string()];
        req.grok_depth = Some(shared_types::GrokDepthOverride::Deep);
        run(&req, move |r| {
            log.lock().expect("log").push(r.clone());
            Ok(reply(&r.agent, "APPROVE"))
        })
        .await
        .expect("jury runs");

        let seen = seen.lock().expect("log");
        assert_eq!(seen.len(), 3);
        for r in seen.iter() {
            assert_eq!(r.strict_agent, Some(true), "{} was not dispatched strict", r.agent);
            assert_eq!(r.message, "label part 7");
            assert_eq!(r.require_sight, Some(true), "named sources imply the sight gate");
            assert_eq!(r.required_sources, vec!["/abs/part-7.md".to_string()]);
            assert_eq!(r.grok_depth.is_some(), r.agent == "grok", "grok_depth is for the grok seat only");
        }
    }

    /// RED IF: one slow seat holds the others' results back, or is reported as anything but
    /// a timeout. The journal callback must have the fast seats before the slow one ends.
    #[tokio::test]
    async fn a_slow_seat_times_out_alone_and_the_others_are_journaled_first() {
        let mut req = jury(&[]);
        req.timeout_s = Some(1);
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let journal = order.clone();
        let out = run_jury(
            &req,
            None,
            None,
            Duration::from_secs(30),
            |r: AskAgentRequest| async move {
                if r.agent == "grok" {
                    tokio::time::sleep(Duration::from_secs(20)).await;
                }
                Ok(reply(&r.agent, "REJECT"))
            },
            |seat| journal.lock().expect("journal").push((seat.agent.clone(), seat.status.clone())),
        )
        .await
        .expect("jury runs");

        assert_eq!(out.seats["grok"].status, STATUS_TIMEOUT);
        assert_eq!(out.seats["codex"].status, STATUS_ANSWERED);
        assert_eq!(out.seats["gemini"].status, STATUS_ANSWERED);
        let order = order.lock().expect("journal");
        assert_eq!(order.last().map(|(a, _)| a.as_str()), Some("grok"), "the slow seat is journaled last: {order:?}");
        assert_eq!(order.len(), 3);
    }

    #[tokio::test]
    async fn the_caller_cannot_sit_on_its_own_jury() {
        let out = run_jury(
            &jury(&[]),
            Some("codex"),
            None,
            Duration::from_secs(5),
            |r: AskAgentRequest| async move {
                assert_ne!(r.agent, "codex", "the caller's own seat must never be dispatched");
                Ok(reply(&r.agent, "APPROVE"))
            },
            |_| {},
        )
        .await
        .expect("jury runs");
        assert_eq!(out.seats["codex"].status, STATUS_UNAVAILABLE);
        assert!(!out.tally.unanimous);
    }

    #[test]
    fn nothing_is_spent_on_a_malformed_request() {
        assert!(resolve_seats(&["gemini".into(), "antigravity".into()]).unwrap_err().contains("twice"));
        assert!(resolve_seats(&["codex".into()]).unwrap_err().contains("at least two"));
        assert!(resolve_seats(&["codex".into(), "gemnii".into()]).unwrap_err().contains("not a dispatchable"));
        assert_eq!(resolve_seats(&[]).expect("default"), vec!["codex", "grok", "gemini"]);

        let seats = vec!["codex".to_string(), "gemini".to_string()];
        let typo = BTreeMap::from([("grokk".to_string(), "/tmp/x".to_string())]);
        assert!(resolve_outputs(&typo, &seats).unwrap_err().contains("not one of the seats"));
        let alias = BTreeMap::from([("agy".to_string(), "/tmp/x".to_string())]);
        assert_eq!(resolve_outputs(&alias, &seats).expect("alias").keys().collect::<Vec<_>>(), vec!["gemini"]);

        let mut req = jury(&[]);
        req.verdict_regex = Some("(unclosed".to_string());
        assert!(VerdictExtractor::from_request(&req).is_err());
        req.verdict_regex = None;
        req.verdict_json_pointer = Some("verdict".to_string());
        assert!(VerdictExtractor::from_request(&req).is_err());
    }

    fn seat_with(agent: &str, status: &str, verdict: Option<&str>) -> (String, JurySeat) {
        let seat = JurySeat {
            agent: agent.to_string(),
            status: status.to_string(),
            verdict: verdict.map(str::to_string),
            ..Default::default()
        };
        (agent.to_string(), seat)
    }

    /// The brief defined `split` as "all different", which left two answered seats that
    /// disagree, and a lone answer, with no name at all. Every row here has exactly one.
    #[test]
    fn every_tally_lands_on_exactly_one_outcome() {
        let a = STATUS_ANSWERED;
        let cases: [(&[(&str, &str, Option<&str>)], usize, &str); 8] = [
            (&[("codex", a, Some("x")), ("grok", a, Some("x")), ("gemini", a, Some("x"))], 3, "unanimous"),
            (&[("codex", a, Some("x")), ("grok", a, Some("x")), ("gemini", a, Some("y"))], 3, "majority"),
            (&[("codex", a, Some("x")), ("grok", a, Some("x")), ("gemini", STATUS_UNAVAILABLE, None)], 3, "majority"),
            (&[("codex", a, Some("x")), ("grok", a, Some("y")), ("gemini", a, Some("z"))], 3, "split"),
            (&[("codex", a, Some("x")), ("grok", a, Some("y")), ("gemini", STATUS_TIMEOUT, None)], 3, "split"),
            (&[("codex", a, Some("x")), ("grok", STATUS_INVALID, None), ("gemini", STATUS_UNAVAILABLE, None)], 3, "no_quorum"),
            (&[("codex", a, Some("x")), ("grok", a, None), ("gemini", a, None)], 3, "no_quorum"),
            (&[("codex", a, Some("x")), ("grok", a, Some("x"))], 2, "unanimous"),
        ];
        for (rows, requested, expected) in cases {
            let seats: BTreeMap<String, JurySeat> = rows.iter().map(|(n, s, v)| seat_with(n, s, *v)).collect();
            let t = tally(requested, &seats, "first_line");
            assert_eq!(t.outcome, expected, "{rows:?}");
            assert_eq!(t.unanimous, expected == "unanimous", "{rows:?}");
            assert_eq!(t.split, expected == "split", "{rows:?}");
            assert_eq!(t.majority.is_some(), matches!(expected, "unanimous" | "majority"), "{rows:?}");
        }
    }

    #[test]
    fn verdicts_are_read_three_ways_and_normalized_once() {
        let first = VerdictExtractor::FirstLine;
        assert_eq!(first.extract("\n  **APPROVE.**\nbecause reasons").as_deref(), Some("approve"));
        assert_eq!(first.extract("   \n\n"), None);

        let mut req = jury(&[]);
        req.verdict_regex = Some(r"(?m)^VERDICT:\s*(\w+)".to_string());
        let re = VerdictExtractor::from_request(&req).expect("regex");
        assert_eq!(re.extract("I looked at it.\nVERDICT: Reject\n").as_deref(), Some("reject"));
        assert_eq!(re.extract("no verdict line here"), None);

        req.verdict_json_pointer = Some("/label".to_string());
        let ptr = VerdictExtractor::from_request(&req).expect("pointer wins over regex");
        assert_eq!(ptr.source(), "json_pointer");
        assert_eq!(ptr.extract(r#"{"label":"NONE","n":3}"#).as_deref(), Some("none"));
        assert_eq!(ptr.extract("Here you go:\n{\"label\": \"Person\"}\nthanks").as_deref(), Some("person"));
        assert_eq!(ptr.extract(r#"{"label":["a"]}"#), None, "a non-scalar is not a verdict");
    }

    /// BRIEF ACCEPTANCE 3: a resolver can consume the result from counts alone.
    /// RED IF: a file left over from an earlier run passes as this call's output.
    #[test]
    fn outputs_report_counts_never_contents_and_catch_a_stale_file() {
        let _env = crate::PROCESS_ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().expect("tempdir");
        let path = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
        let secret = "SECRET-LABEL-TEXT";

        let missing = check_output(&path("missing.json"), &OutputBaseline::default(), true);
        assert!(!missing.exists && !missing.written_this_call && missing.error.is_some());

        for (name, body, format, rows) in [
            ("a.json", format!(r#"[{{"n":1,"label":"{secret}"}},{{"n":2,"label":"NONE"}}]"#), "json", 2),
            ("b.jsonl", format!("{{\"n\":1,\"label\":\"{secret}\"}}\n{{\"n\":2}}\n{{\"n\":3}}\n"), "jsonl", 3),
            ("c.raw", format!("Labels follow.\n[{{\"n\":1,\"label\":\"{secret}\"}}]\nDONE 1"), "embedded_json", 1),
        ] {
            let p = path(name);
            let before = output_baseline(Path::new(&p));
            std::fs::write(&p, body).expect("write");
            let check = check_output(&p, &before, true);
            assert_eq!((check.format.as_deref(), check.rows), (Some(format), Some(rows)), "{name}");
            assert!(check.written_this_call && check.error.is_none(), "{name}: {check:?}");
            assert!(!serde_json::to_string(&check).expect("json").contains(secret), "{name} leaked a label");
        }

        let stale = path("a.json");
        let before = output_baseline(Path::new(&stale));
        let check = check_output(&stale, &before, true);
        assert!(check.exists && !check.written_this_call, "an untouched file is not this call's work");
        assert!(check.error.as_deref().unwrap_or_default().contains("predates"));

        let junk = path("junk.raw");
        std::fs::write(&junk, "I could not complete the task.").expect("write");
        let check = check_output(&junk, &OutputBaseline::default(), true);
        assert_eq!(check.rows, None);
        assert!(check.error.as_deref().unwrap_or_default().contains("no parseable JSON"));
    }
}
