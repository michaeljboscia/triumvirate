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

use crate::{ProgressEmitter, inter_agent::{ExecuteAskAgentFn, describe_ask_agent_failure}};
use daemon_http::{daemon_ask_timeout_secs, fetch_daemon_ask_agent, fetch_daemon_ledger_record};
use mcp_bridge::{caller_driver_identity, display_agent_name, is_supported_agent_name, normalize_agent_name};
use rmcp::{
    Json,
    service::{RequestContext, RoleServer},
};
use shared_types::{
    AskAgentRequest, AskAgentResponse, AskJuryRequest, AskJuryResponse, JuryMajority,
    JuryOutputCheck, JurySeat, JuryTally, ManualRecord, OutboxEvent,
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
        // Two seats, one file: each would read as "exists, parses, changed during the call" on
        // the strength of the OTHER seat's write, and the later writer erases the earlier vote.
        if let Some((other, _)) = resolved.iter().find(|(_, existing): &(&String, &String)| is_same_file(existing, path)) {
            return Err(format!("ask_jury: the {other} and {canonical} seats are both told to write {path}"));
        }
        if resolved.insert(canonical.clone(), path.clone()).is_some() {
            return Err(format!("ask_jury: outputs names the {canonical} seat twice"));
        }
    }
    Ok(resolved)
}

/// Do two paths name one file? Compared by identity, not by spelling.
///
/// String equality rejected `/tmp/x` twice and accepted `/tmp/x` beside `/tmp/./x`, which is
/// the same file and the same collision. Grok found it: the fix had closed the exact input I
/// had thought of.
///
/// `.` and `..` are resolved lexically FIRST, then the parent directory is canonicalized (it
/// exists; the output file usually does not yet). Canonicalizing first is not an option: a
/// parent that does not exist falls back to its literal spelling, and on macOS that compares
/// `/private/tmp` against `/tmp` and calls one file two.
///
/// Limit: lexical `..` is not symlink-aware, so two paths through a symlinked directory can be
/// called the same file. That direction is safe here. It rejects the configuration and says
/// why, rather than accepting a collision and losing a vote to it.
fn is_same_file(a: &str, b: &str) -> bool {
    fn key(p: &str) -> (std::path::PathBuf, Option<std::ffi::OsString>) {
        use std::path::Component;
        let mut lexical = std::path::PathBuf::new();
        for part in Path::new(p).components() {
            match part {
                Component::CurDir => {}
                Component::ParentDir => {
                    if !lexical.pop() {
                        lexical.push("..");
                    }
                }
                other => lexical.push(other),
            }
        }
        let parent = lexical.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let dir = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        (dir, lexical.file_name().map(|n| n.to_os_string()))
    }
    key(a) == key(b)
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

    /// The verdict, or why there is not one.
    ///
    /// AMBIGUITY IS NOT A VERDICT. Every candidate the reader finds is normalized and they must
    /// agree. Picking one of several was tried both ways and both are wrong:
    ///
    /// - First match: a seat that restates the question ("verdict: approve or reject... I
    ///   choose REJECT") votes for the echo. Antigravity found it.
    /// - Last match: a seat that answers correctly and then explains itself votes for a word in
    ///   its own prose. Grok answered "YES\n17 is a prime number because it has **no** positive
    ///   divisors", and last-match recorded `no`. Found on the first live run, in the fix for
    ///   the first bug.
    ///
    /// So the reader reports what it can prove. One distinct value is the vote; two is a reply
    /// that supports two readings, and the seat casts nothing, visibly, with the reason
    /// attached. The caller's answer is a regex that matches one thing, such as
    /// `(?m)^VERDICT:\s*(\w+)$`.
    pub fn extract(&self, reply: &str) -> (Option<String>, Option<String>) {
        let candidates: Vec<String> = match self {
            Self::JsonPointer(pointer) => json_objects_in(reply)
                .iter()
                .filter_map(|value| match value.pointer(pointer)? {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .collect(),
            Self::Regex(re) => re
                .captures_iter(reply)
                .filter_map(|caps| Some(caps.get(1).or_else(|| caps.get(0))?.as_str().to_string()))
                .collect(),
            Self::FirstLine => reply.lines().find(|l| !l.trim().is_empty()).map(str::to_string).into_iter().collect(),
        };

        let mut distinct: Vec<String> = Vec::new();
        for value in candidates.iter().filter_map(|c| normalize_verdict(c)) {
            if !distinct.contains(&value) {
                distinct.push(value);
            }
        }
        match distinct.len() {
            1 => (distinct.pop(), None),
            0 => (
                None,
                Some(format!("the {} reader found no verdict in this reply", self.source())),
            ),
            _ => (
                None,
                Some(format!(
                    "ambiguous: the {} reader found {} different verdicts in one reply ({}). \
                     Nothing is counted. Narrow the pattern so it matches exactly one.",
                    self.source(),
                    distinct.len(),
                    distinct.join(", ")
                )),
            ),
        }
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

/// Every JSON object in a reply, in order: the whole reply if it is one, else each object
/// found by trying to parse from every `{`.
///
/// NOT "first brace to last brace". A reply of two objects, thinking then verdict, makes that
/// span invalid JSON, and a valid vote vanished into a false split. Antigravity found it.
fn json_objects_in(text: &str) -> Vec<serde_json::Value> {
    if let Ok(v @ serde_json::Value::Object(_)) = serde_json::from_str(text.trim()) {
        return vec![v];
    }
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find('{') {
        let start = from + offset;
        let mut stream = serde_json::Deserializer::from_str(&text[start..]).into_iter::<serde_json::Value>();
        match stream.next() {
            Some(Ok(v @ serde_json::Value::Object(_))) => {
                found.push(v);
                from = start + stream.byte_offset();
            }
            _ => from = start + 1,
        }
    }
    found
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
    check.changed_during_call = &now != baseline;
    if !check.changed_during_call {
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
///
/// `rows` counts JSON VALUES. It is not a claim that any of them is a well-formed anything:
/// `["nonsense","garbage"]` is honestly two rows. A caller that needs structure must check the
/// structure. What the count does catch is the common case, a seat that wrote nothing, wrote
/// prose, or wrote fewer rows than it was given parts.
fn count_json_rows(text: &str) -> Option<(&'static str, u64)> {
    // An empty object is not a row. It parses, and reporting `rows: 1` for it told a caller
    // that something had been written when the file carried nothing (Antigravity).
    let rows_of = |v: &serde_json::Value| match v {
        serde_json::Value::Array(a) => a.len() as u64,
        serde_json::Value::Object(o) if o.is_empty() => 0,
        _ => 1,
    };
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

    seat.model = resp.model.clone();
    let invalid = if resp.strict_agent_honored != Some(true) {
        // Silence is not proof. Every seat is dispatched strict, so a daemon that took the
        // strict path says so; one that ignored the field says nothing, and a substituted vote
        // from it would otherwise have to be caught by fields it may also omit (Codex).
        Some("this daemon did not acknowledge strict_agent, so no substitution check it reports \
              can be trusted. Install the daemon that ships with ask_jury".to_string())
    } else if answered_by != agent {
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
    (seat.verdict, seat.verdict_note) = extractor.extract(&resp.response);
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
        // Without this the daemon runs the seats one at a time: they share a `cwd`.
        own_lane: Some(true),
        ..Default::default()
    }
}

pub struct JuryRun {
    pub seats: BTreeMap<String, JurySeat>,
    pub tally: JuryTally,
    /// The probe this run attempted, if any, and which seats it bought back.
    pub breaker_probe: Option<shared_types::BreakerProbeResponse>,
    pub seats_retried_after_probe: Vec<String>,
}

/// Would a breaker probe plausibly fix this seat's failure?
///
/// Only the agy-backed `gemini` seat has a breaker. The 2026-09-19 addendum to the brief: the
/// breaker outlived the outage, so `agy --print` worked by hand while the bridge still refused,
/// and a jury would have reported a reachable seat as unavailable. A probe is one small live
/// call, so it is spent only on a failure a probe could actually clear.
fn a_probe_could_fix(seat: &JurySeat) -> bool {
    if seat.agent != "gemini" || seat.status != STATUS_UNAVAILABLE {
        return false;
    }
    let reason = seat.reason.as_deref().unwrap_or_default().to_lowercase();
    ["circuit breaker", "capacity/quota", "quota", "resource_exhausted", "429"]
        .iter()
        .any(|needle| reason.contains(needle))
}

/// Dispatch every seat at once and judge each as it lands.
///
/// `on_seat` fires as each seat finishes, in completion order, BEFORE the slow seats are done.
/// That is the journal: if the caller's own client ceiling cancels this future, the seats that
/// had already reported are on disk rather than lost with the call.
pub async fn run_jury<F, Fut, P, PFut>(
    req: &AskJuryRequest,
    caller: Option<&str>,
    emitter: Option<&ProgressEmitter>,
    default_timeout: Duration,
    run_seat: F,
    probe_breaker: P,
    mut on_seat: impl FnMut(&JurySeat),
) -> Result<JuryRun, String>
where
    F: Fn(AskAgentRequest) -> Fut,
    Fut: Future<Output = Result<AskAgentResponse, String>> + Send + 'static,
    P: Fn() -> PFut,
    PFut: Future<Output = Result<shared_types::BreakerProbeResponse, String>>,
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
            let mut check = check_output(path, &baselines[&seat.agent], expect_json);
            // A timeout ends the WAIT, not the work. The daemon does not cancel on client
            // disconnect, so the seat may still be running and may write this file afterwards
            // (Codex). Reporting "the seat did not write it" would be a claim about a race.
            if seat.status == STATUS_TIMEOUT {
                check.error = Some(format!(
                    "{} (the seat may still be running in the daemon and may write this file later)",
                    check.error.as_deref().unwrap_or("checked while the seat was still running")
                ));
            }
            seat.output = Some(check);
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

    // THE BRIEF'S 2026-09-19 ADDENDUM. A seat is not called unavailable until a probe has had
    // one chance to disprove it. Once per RUN, not once per seat: three seats failing on the
    // same breaker is one outage, and three probes would spend three live calls to learn it.
    let mut breaker_probe = None;
    let mut seats_retried_after_probe: Vec<String> = Vec::new();
    let probe_candidates: Vec<String> =
        seats.values().filter(|s| a_probe_could_fix(s)).map(|s| s.agent.clone()).collect();
    if !probe_candidates.is_empty() {
        if let Some(e) = emitter {
            e.emit("→ jury: a seat failed on the breaker; probing before calling it unavailable").await;
        }
        match probe_breaker().await {
            Ok(probe) => {
                if probe.closed_by_probe {
                    for agent in probe_candidates {
                        let started = Instant::now();
                        let outcome = match run_seat(seat_request(req, &agent)).await {
                            Ok(resp) => SeatOutcome::Replied(Box::new(resp)),
                            Err(e) => SeatOutcome::Failed(e),
                        };
                        let seat = classify_seat(&agent, outcome, started.elapsed().as_millis() as u64, &extractor);
                        seats_retried_after_probe.push(agent);
                        finish(seat, &mut seats);
                    }
                }
                breaker_probe = Some(probe);
            }
            // A probe that cannot run leaves the seat exactly as it was. The jury still returns.
            Err(e) => tracing::warn!(error = %e, "ask_jury: breaker probe failed; seat stays unavailable"),
        }
    }

    let tally = tally(seat_names.len(), &seats, extractor.source());
    Ok(JuryRun { seats, tally, breaker_probe, seats_retried_after_probe })
}

/// The MCP tool. Seats ride the same path `ask_agent` does: the daemon when proxying, in
/// process otherwise.
pub async fn ask_jury(
    req: &AskJuryRequest,
    context: &RequestContext<RoleServer>,
    local_test_execution_allowed: bool,
    execute_ask_agent: ExecuteAskAgentFn,
) -> Result<Json<AskJuryResponse>, String> {
    let jury_id = format!("jury-{}", uuid::Uuid::new_v4());
    let emitter = ProgressEmitter::from_context(context);
    let caller = caller_driver_identity();
    let cwd = req.cwd.clone();

    let run = run_jury(
        req,
        caller.as_deref(),
        Some(&emitter),
        Duration::from_secs(daemon_ask_timeout_secs()),
        move |seat_req: AskAgentRequest| async move {
            if local_test_execution_allowed {
                execute_ask_agent(&seat_req, None).await
            } else {
                fetch_daemon_ask_agent(&seat_req).await.map_err(|e| describe_ask_agent_failure(&e))
            }
        },
        || async {
            if local_test_execution_allowed {
                Err("breaker probe runs in the daemon".to_string())
            } else {
                daemon_http::fetch_daemon_breaker_probe(&shared_types::BreakerProbeRequest::default())
                    .await
                    .map_err(|e| format!("{e:#}"))
            }
        },
        |seat| journal_seat(&jury_id, seat, &cwd),
    )
    .await?;

    let record = ledger_record_for(&jury_id, &run);
    let ledger = write_ledger(record, req.cwd.as_deref(), !local_test_execution_allowed).await;
    if let Err(e) = &ledger {
        tracing::warn!(jury_id, error = %e, "ask_jury: ledger record failed; the verdicts stand");
    }
    Ok(Json(AskJuryResponse {
        jury_id,
        seats: run.seats,
        tally: run.tally,
        breaker_probe: run.breaker_probe,
        seats_retried_after_probe: run.seats_retried_after_probe,
        ledger_recorded: ledger.is_ok(),
        ledger_error: ledger.err(),
    }))
}

/// What a seat's failure reason may become OUTSIDE the caller's response.
///
/// `reason` is the daemon's own error text, and the daemon quotes things. A sight-gate
/// rejection appends the rejected reply verbatim under a marker; a peer-review block quotes the
/// reviewer. For a labelling jury that reply IS the label, so writing the reason straight into
/// the outbox and the ledger defeated the rule that only counts leave this tool. Two of those
/// rejections happened while this very change was under review.
///
/// The caller still gets the whole reason in the response: it asked, and it already holds the
/// brief. The journal gets the first line, cut at the first quoting marker, capped at 200
/// characters on a char boundary.
fn journal_reason(reason: &str) -> String {
    const MARKERS: [&str; 4] = ["--- rejected output", "rejected output, for inspection", "Evidence:", "codex said:"];
    let mut text = reason;
    for marker in MARKERS {
        if let Some(at) = text.find(marker) {
            text = &text[..at];
        }
    }
    let line = text.lines().next().unwrap_or_default().trim();
    match line.char_indices().nth(200) {
        Some((cut, _)) => format!("{}...", &line[..cut]),
        None => line.to_string(),
    }
}

/// The outbox line for a seat. A function of its own so a test can assert on the string that
/// is actually written, rather than on the helper it is supposed to call.
fn seat_detail(seat: &JurySeat) -> String {
    match (&seat.reason, &seat.output) {
        (Some(reason), _) => journal_reason(reason),
        (None, Some(out)) => format!("output rows={:?} changed_during_call={}", out.rows, out.changed_during_call),
        (None, None) => format!("answered by {}", seat.answered_by_agent.as_deref().unwrap_or(&seat.agent)),
    }
}

/// One outbox line and one PostHog event per seat, as that seat lands.
fn journal_seat(jury_id: &str, seat: &JurySeat, cwd: &Option<String>) {
    let detail = seat_detail(seat);
    if let Err(e) = fallback_outbox::append_outbox_event(&OutboxEvent {
        ts_ms: daemon_core::unix_time_ms(),
        request_id: jury_id.to_string(),
        tool: "ask_jury".to_string(),
        status: format!("SEAT_{}", seat.status.to_uppercase()),
        agent: Some(seat.agent.clone()),
        detail,
        cwd: cwd.clone(),
        repo: None,
        branch: None,
        working_state: None,
        token_usage: None,
        tool_name: None,
    }) {
        tracing::warn!("failed to append outbox event: {e}");
    }
    mcp_bridge::posthog::record_jury_seat(
        jury_id,
        &seat.agent,
        &seat.status,
        seat.answered_by_agent.as_deref(),
        seat.answered_by_backend.as_deref(),
        seat.duration_ms,
    );
}

/// Provenance and counts. No reply text and no verdict text: for a labelling jury the verdict
/// IS the label, and the ledger is not where labels belong.
fn ledger_record_for(jury_id: &str, run: &JuryRun) -> ManualRecord {
    let t = &run.tally;
    let seats: Vec<serde_json::Value> = run
        .seats
        .values()
        .map(|s| {
            serde_json::json!({
                "agent": s.agent,
                "status": s.status,
                "reason": s.reason.as_deref().map(journal_reason),
                "answered_by_agent": s.answered_by_agent,
                "answered_by_backend": s.answered_by_backend,
                "request_id": s.request_id,
                "cast_a_verdict": s.verdict.is_some(),
                "output_rows": s.output.as_ref().and_then(|o| o.rows),
                "output_changed_during_call": s.output.as_ref().map(|o| o.changed_during_call),
            })
        })
        .collect();
    ManualRecord {
        session_id: Some(jury_id.to_string()),
        title: format!("ask_jury {}: {} of {} seats answered", t.outcome, t.seats_answered, t.seats_requested),
        narrative: format!(
            "Jury {jury_id}: outcome {}, {} of {} seats answered, {} verdicts cast (read by {}). Seats: {}.",
            t.outcome,
            t.seats_answered,
            t.seats_requested,
            t.verdicts_cast,
            t.verdict_source,
            run.seats.values().map(|s| format!("{}={}", s.agent, s.status)).collect::<Vec<_>>().join(", ")
        ),
        facts_json: Some(
            serde_json::json!({
                "jury_id": jury_id,
                "outcome": t.outcome,
                "unanimous": t.unanimous,
                "split": t.split,
                "majority_count": t.majority.as_ref().map(|m| m.count),
                "seats_requested": t.seats_requested,
                "seats_answered": t.seats_answered,
                "verdicts_cast": t.verdicts_cast,
                "seats": seats,
            })
            .to_string(),
        ),
        concepts_json: None,
        affected_files_json: None,
        summary_type: "jury".to_string(),
    }
}

async fn write_ledger(record: ManualRecord, cwd: Option<&str>, via_daemon: bool) -> Result<(), String> {
    if via_daemon {
        return fetch_daemon_ledger_record(&record).await.map(|_| ()).map_err(|e| format!("{e:#}"));
    }
    let root = match cwd {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::env::current_dir().map_err(|e| format!("no current directory: {e}"))?,
    };
    ledger::LedgerStore::open(root)
        .and_then(|store| store.record(record))
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a CURRENT daemon returns for a strict seat: it acknowledges the strict path.
    fn reply(agent: &str, text: &str) -> AskAgentResponse {
        let mut r = AskAgentResponse::direct(format!("req-{agent}"), agent.to_string(), text.to_string(), Vec::new());
        r.strict_agent_honored = Some(true);
        r
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
            || async { Err("no probe expected in this test".to_string()) },
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

    /// RED IF: an unacknowledged reply counts. A daemon that never heard of `strict_agent`
    /// ignores the field, substitutes, and may omit the provenance fields too, so trusting
    /// silence put the whole defence on evidence that daemon had no reason to send.
    #[tokio::test]
    async fn a_daemon_that_does_not_acknowledge_strict_cannot_cast_a_vote() {
        let out = run(&jury(&[]), |r| {
            // No `strict_agent_honored`, and clean provenance: the pre-strict daemon's shape.
            Ok(AskAgentResponse::direct(
                "req-old".to_string(),
                r.agent.clone(),
                "APPROVE".to_string(),
                Vec::new(),
            ))
        })
        .await
        .expect("jury runs");

        for seat in out.seats.values() {
            assert_eq!(seat.status, STATUS_INVALID, "{}: {seat:?}", seat.agent);
            assert!(seat.reason.as_deref().unwrap_or_default().contains("did not acknowledge"));
            assert_eq!(seat.response, None);
        }
        assert_eq!(out.tally.outcome, "no_quorum", "unverifiable is not agreement");
        assert_eq!(out.tally.seats_answered, 0);
    }

    /// The three provenance checks are independent, so each is exercised ALONE. The test above
    /// trips two at once and would stay green with either one deleted.
    #[test]
    fn each_provenance_check_stands_on_its_own() {
        let first = VerdictExtractor::FirstLine;
        let judge = |resp: AskAgentResponse| classify_seat("gemini", SeatOutcome::Replied(Box::new(resp)), 1, &first);

        assert_eq!(
            judge(AskAgentResponse::direct("r".into(), "gemini".into(), "APPROVE".into(), Vec::new())).status,
            STATUS_INVALID,
            "an unacknowledged reply is unverifiable, whatever else it says"
        );

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
            assert_eq!(r.own_lane, Some(true), "{} would queue behind the other seats in the daemon", r.agent);
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
            || async { Err("no probe expected in this test".to_string()) },
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

    fn probe_result(closed: bool) -> shared_types::BreakerProbeResponse {
        let open = shared_types::BreakerSnapshot {
            phase: "open".to_string(),
            cooldown_remaining_s: 9000,
            open_count: 4,
            shed: 12,
        };
        let after = if closed {
            shared_types::BreakerSnapshot { phase: "closed".to_string(), cooldown_remaining_s: 0, open_count: 0, shed: 0 }
        } else {
            open.clone()
        };
        shared_types::BreakerProbeResponse {
            backend: "agy".to_string(),
            before: open,
            after,
            outcome: if closed { "ok" } else { "backend_failed" }.to_string(),
            detail: "test probe".to_string(),
            closed_by_probe: closed,
        }
    }

    /// Runs a jury with a scripted seat runner AND a scripted probe, counting both.
    async fn run_with_probe(
        req: &AskJuryRequest,
        script: impl Fn(&AskAgentRequest, usize) -> Result<AskAgentResponse, String> + Send + Sync + 'static,
        probe: Result<shared_types::BreakerProbeResponse, String>,
    ) -> (JuryRun, usize) {
        let script = std::sync::Arc::new(script);
        let dispatches = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let probes = std::sync::Arc::new(std::sync::Mutex::new(0usize));
        let probe = std::sync::Arc::new(probe);
        let seen = dispatches.clone();
        let counted = probes.clone();
        let run = run_jury(
            req,
            None,
            None,
            Duration::from_secs(5),
            move |r: AskAgentRequest| {
                let script = script.clone();
                let seen = seen.clone();
                let nth = {
                    let mut n = seen.lock().expect("count");
                    *n += 1;
                    *n
                };
                async move { script(&r, nth) }
            },
            move || {
                let probe = probe.clone();
                let counted = counted.clone();
                async move {
                    *counted.lock().expect("count") += 1;
                    (*probe).clone()
                }
            },
            |_| {},
        )
        .await
        .expect("jury runs");
        let n = *probes.lock().expect("count");
        (run, n)
    }

    /// THE BRIEF'S 2026-09-19 ADDENDUM. The breaker outlived the outage: agy answered by hand
    /// while the bridge still refused, so a reachable seat would have been reported dead.
    /// RED IF: a breaker-shaped failure is reported unavailable without a probe having tried.
    #[tokio::test]
    async fn a_breaker_failure_is_probed_before_the_seat_is_called_unavailable() {
        let (run, probes) = run_with_probe(
            &jury(&[]),
            |r, nth| match (r.agent.as_str(), nth) {
                // The gemini seat fails on the open breaker, then succeeds once it is closed.
                ("gemini", n) if n <= 3 => Err("agy capacity/quota: circuit breaker open".to_string()),
                (agent, _) => Ok(reply(agent, "APPROVE")),
            },
            Ok(probe_result(true)),
        )
        .await;

        assert_eq!(probes, 1, "exactly one probe: three seats failing on one breaker is one outage");
        assert_eq!(run.seats["gemini"].status, STATUS_ANSWERED, "{:?}", run.seats["gemini"]);
        assert_eq!(run.seats_retried_after_probe, vec!["gemini".to_string()]);
        assert_eq!(run.breaker_probe.as_ref().map(|p| p.closed_by_probe), Some(true));
        assert!(run.tally.unanimous, "the recovered seat counts: {:?}", run.tally);
    }

    /// A probe is a live call. RED IF: it is spent on a failure it could not possibly fix.
    #[tokio::test]
    async fn no_probe_is_spent_on_a_failure_a_probe_cannot_fix() {
        for (agent, reason) in [
            // Not the breaker.
            ("gemini", "prompt too large for agy (900000 bytes > 400000 limit)"),
            // A genuinely quota-shaped failure, on an agent that HAS NO BREAKER. The first
            // version of this case said "at capacity", which the reason filter rejects on its
            // own, so removing the agent guard changed nothing and the mutant lived.
            ("codex", "codex connector failed: quota exceeded (429), try again later"),
        ] {
            let failing = agent.to_string();
            let (run, probes) = run_with_probe(
                &jury(&[]),
                move |r, _| {
                    if r.agent == failing { Err(reason.to_string()) } else { Ok(reply(&r.agent, "APPROVE")) }
                },
                Ok(probe_result(true)),
            )
            .await;
            assert_eq!(probes, 0, "{agent} / {reason}");
            assert_eq!(run.seats[agent].status, STATUS_UNAVAILABLE);
            assert!(run.breaker_probe.is_none());
        }
    }

    /// RED IF: a probe that did not close the breaker triggers a retry anyway, or a probe that
    /// could not run at all takes the jury down with it.
    #[tokio::test]
    async fn a_probe_that_does_not_close_leaves_the_seat_unavailable() {
        for probe in [Ok(probe_result(false)), Err("daemon unreachable".to_string())] {
            let reported = probe.is_ok();
            let (run, probes) = run_with_probe(
                &jury(&[]),
                |r, _| {
                    if r.agent == "gemini" {
                        Err("agy capacity/quota: circuit breaker open".to_string())
                    } else {
                        Ok(reply(&r.agent, "APPROVE"))
                    }
                },
                probe,
            )
            .await;
            assert_eq!(probes, 1);
            assert_eq!(run.seats["gemini"].status, STATUS_UNAVAILABLE);
            assert!(run.seats_retried_after_probe.is_empty());
            assert_eq!(run.breaker_probe.is_some(), reported, "a probe that ran is reported either way");
            assert_eq!(run.tally.outcome, "majority");
        }
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
            || async { Err("no probe expected in this test".to_string()) },
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
        for (a, b) in [
            ("/tmp/x", "/tmp/x"),
            // Same file, different spelling. String equality accepted these (Grok).
            ("/tmp/x", "/tmp/./x"),
            ("/tmp/x", "/tmp/sub/../x"),
        ] {
            let shared = BTreeMap::from([("codex".to_string(), a.to_string()), ("gemini".to_string(), b.to_string())]);
            let err = resolve_outputs(&shared, &seats).unwrap_err();
            assert!(err.contains("both told to write"), "{a} vs {b}: {err}");
        }
        let distinct = BTreeMap::from([("codex".to_string(), "/tmp/a".to_string()), ("gemini".to_string(), "/tmp/b".to_string())]);
        assert!(resolve_outputs(&distinct, &seats).is_ok(), "different files are fine");
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

    fn verdict_of(e: &VerdictExtractor, reply: &str) -> Option<String> {
        e.extract(reply).0
    }

    #[test]
    fn verdicts_are_read_three_ways_and_normalized_once() {
        let first = VerdictExtractor::FirstLine;
        assert_eq!(verdict_of(&first, "\n  **APPROVE.**\nbecause reasons").as_deref(), Some("approve"));
        assert_eq!(verdict_of(&first, "   \n\n"), None);

        let mut req = jury(&[]);
        req.verdict_regex = Some(r"(?m)^VERDICT:\s*(\w+)".to_string());
        let re = VerdictExtractor::from_request(&req).expect("regex");
        assert_eq!(verdict_of(&re, "I looked at it.\nVERDICT: Reject\n").as_deref(), Some("reject"));
        assert_eq!(verdict_of(&re, "no verdict line here"), None);
        // The same answer twice is one answer, not an ambiguity.
        assert_eq!(verdict_of(&re, "VERDICT: reject\nTo restate, VERDICT: REJECT.").as_deref(), Some("reject"));

        req.verdict_json_pointer = Some("/label".to_string());
        let ptr = VerdictExtractor::from_request(&req).expect("pointer wins over regex");
        assert_eq!(ptr.source(), "json_pointer");
        assert_eq!(verdict_of(&ptr, r#"{"label":"NONE","n":3}"#).as_deref(), Some("none"));
        assert_eq!(verdict_of(&ptr, "Here you go:\n{\"label\": \"Person\"}\nthanks").as_deref(), Some("person"));
        assert_eq!(verdict_of(&ptr, r#"{"label":["a"]}"#), None, "a non-scalar is not a verdict");
    }

    /// ANTIGRAVITY'S FINDINGS on extraction. Each one silently changed a tally.
    #[test]
    fn a_reply_cannot_hide_or_fake_a_vote_through_the_extractor() {
        let mut req = jury(&[]);
        req.verdict_json_pointer = Some("/verdict".to_string());
        let ptr = VerdictExtractor::from_request(&req).expect("pointer");

        // Two objects: brace-to-brace spanned them both and parsed as nothing, so a cast vote
        // vanished and the tally reported a false split.
        let two = "{\"thinking\":\"weighing it up\"}\n{\"verdict\":\"APPROVE\"}";
        assert_eq!(verdict_of(&ptr, two).as_deref(), Some("approve"), "the vote must survive a preamble object");
        assert_eq!(verdict_of(&ptr, "{\"thinking\":\"no verdict anywhere\"}"), None);

        // Two DIFFERENT verdicts in one reply. Neither is the answer: the seat said both.
        let revised = "{\"verdict\":\"APPROVE\"}\nOn reflection:\n{\"verdict\":\"REJECT\"}";
        let (v, note) = ptr.extract(revised);
        assert_eq!(v, None, "a reply that supports two readings casts nothing");
        assert!(note.unwrap_or_default().contains("ambiguous"));

        // THE ECHOED PROMPT (Antigravity): first-match voted for the restatement.
        req.verdict_json_pointer = None;
        req.verdict_regex = Some(r"(?i)verdict:\s*(approve|reject)".to_string());
        let loose = VerdictExtractor::from_request(&req).expect("regex");
        let echo = "You asked for verdict: approve or reject.\nMy answer: VERDICT: Reject";
        assert_eq!(verdict_of(&loose, echo), None, "approve and reject both appear; guessing is what broke this");

        // THE LIVE CASE, verbatim from grok on the first real jury run. Last-match, the fix for
        // the echoed prompt, read the `no` in "no positive divisors" and recorded a NO against
        // a correct YES, turning a unanimous jury into a false majority.
        req.verdict_regex = Some(r"(?i)\b(YES|NO)\b".to_string());
        let sloppy = VerdictExtractor::from_request(&req).expect("regex");
        let grok = "YES\n17 is a prime number because it has no positive divisors other than 1 and itself.";
        let (v, note) = sloppy.extract(grok);
        assert_eq!(v, None, "a pattern this loose cannot tell the vote from the prose");
        assert!(note.clone().unwrap_or_default().contains("yes"), "{note:?}");
        assert!(note.unwrap_or_default().contains("no"));

        // And the documented answer: a pattern that matches one thing reads it correctly.
        req.verdict_regex = Some(r"(?m)^\s*(YES|NO)\s*$".to_string());
        let anchored = VerdictExtractor::from_request(&req).expect("regex");
        assert_eq!(verdict_of(&anchored, grok).as_deref(), Some("yes"), "anchor it and the same reply is clear");
    }

    /// The reason a seat failed is the DAEMON'S text, and the daemon quotes what it rejected.
    /// RED IF: the journal stops trimming it. A rejected reply is a label.
    #[test]
    fn a_failure_reason_never_carries_a_quoted_reply_into_the_journal() {
        let label = "PERSON: Jane Rutherford";
        let rejection = format!(
            "Codex was dispatched as a review over 1 named source(s) and never opened it. \
             Evidence: /work/part-7.md: no tool call named it\n\n\
             --- rejected output, for inspection, NOT a review ---\n[{{\"n\":1,\"label\":\"{label}\"}}]"
        );
        let long = format!("quota exhausted: {}", "x".repeat(500));
        assert!(journal_reason(&long).chars().count() <= 204, "bounded");

        let first = VerdictExtractor::FirstLine;
        let seat = classify_seat("grok", SeatOutcome::Failed(rejection.clone()), 1, &first);
        // The CALLER still gets the whole thing: it asked, and it already holds the brief.
        assert_eq!(seat.reason.as_deref(), Some(rejection.as_str()));

        // THE TWO PAYLOADS THAT LEAVE, asserted on the real strings. Asserting on
        // `journal_reason` alone left both call sites free to drop it: two mutants that
        // wrote the reason verbatim to the outbox and the ledger both survived.
        let outbox = seat_detail(&seat);
        let run = JuryRun {
            breaker_probe: None,
            seats_retried_after_probe: Vec::new(),
            tally: tally(3, &BTreeMap::from([("grok".to_string(), seat)]), "first_line"),
            seats: BTreeMap::from([("grok".to_string(), classify_seat("grok", SeatOutcome::Failed(rejection.clone()), 1, &first))]),
        };
        let record = ledger_record_for("jury-test", &run);
        let ledger = serde_json::to_string(&record).expect("record serializes");
        for (surface, text) in [("outbox detail", &outbox), ("ledger record", &ledger)] {
            assert!(!text.contains(label), "{surface} carried the label: {text}");
            assert!(!text.contains("rejected output"), "{surface} carried the quoting marker: {text}");
            assert!(!text.contains("Evidence:"), "{surface} carried the evidence block: {text}");
        }
        assert!(outbox.starts_with("Codex was dispatched"), "the cause itself survives: {outbox}");
        assert!(ledger.contains("Codex was dispatched"), "the ledger keeps the cause: {ledger}");
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
        assert!(!missing.exists && !missing.changed_during_call && missing.error.is_some());

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
            assert!(check.changed_during_call && check.error.is_none(), "{name}: {check:?}");
            assert!(!serde_json::to_string(&check).expect("json").contains(secret), "{name} leaked a label");
        }

        let stale = path("a.json");
        let before = output_baseline(Path::new(&stale));
        let check = check_output(&stale, &before, true);
        assert!(check.exists && !check.changed_during_call, "an untouched file is not this call's work");
        assert!(check.error.as_deref().unwrap_or_default().contains("predates"));

        // `{}` parses and used to report one row, telling a caller something had been written.
        for (name, body) in [("empty.json", "{}"), ("empty-arr.json", "[]")] {
            let p = path(name);
            std::fs::write(&p, body).expect("write");
            let check = check_output(&p, &OutputBaseline::default(), true);
            assert_eq!(check.rows, Some(0), "{name} carries nothing: {check:?}");
        }

        let junk = path("junk.raw");
        std::fs::write(&junk, "I could not complete the task.").expect("write");
        let check = check_output(&junk, &OutputBaseline::default(), true);
        assert_eq!(check.rows, None);
        assert!(check.error.as_deref().unwrap_or_default().contains("no parseable JSON"));
    }
}
