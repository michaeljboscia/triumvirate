//! The surface gate (Seven Primes FEAT-007, REQ-090, REQ-091): one named refusal code per refused call, decided before
//! anything spawns. BACKEND_STRUCTURE.md section 3 holds precedence and payloads.
//!
//! Amendment A-2 (Mike overrode D-02): Kimi and Qwen get exactly Mistral's launch surfaces, so the gate no longer
//! distinguishes interactive from unattended calls. `origin` and `tty` stay in the signature (callers already pass
//! them, and a future vendor-terms rule would need them), but at launch no (seat, surface, origin, tty) triple produces
//! `seat_surface_refused`.
//!
//! The decision is table-driven: a surface is allowed when it is in the seat's `SeatProfile.surfaces_allowed`.
//! Outside it, BlindValidate on a seat that cannot write a test file is `seat_cannot_write_here`, and anything else is
//! `seat_surface_deferred` (peer-review rotation and prewarm ship in v1.1, F-4; the writer surfaces opened in W-1). Existing seats
//! (configured always, no verification) are allowed everywhere, which is today's behavior.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub use crate::seat_profile::Surface;
pub use shared_types::Origin;

use crate::seat_profile::{Configured, SeatProfile, SuspensionPolicy, VerifyPolicy};
use crate::seat_state::{Blocker, SeatAvailability};

/// Refusal codes in precedence order: the first that applies is the one returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalCode {
    SeatNotConfigured,
    SeatSuspended,
    SeatUnverified,
    SeatSurfaceRefused,
    SeatSurfaceDeferred,
    SeatCannotWriteHere,
}

impl RefusalCode {
    /// The code as it starts every payload string.
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalCode::SeatNotConfigured => "seat_not_configured",
            RefusalCode::SeatSuspended => "seat_suspended",
            RefusalCode::SeatUnverified => "seat_unverified",
            RefusalCode::SeatSurfaceRefused => "seat_surface_refused",
            RefusalCode::SeatSurfaceDeferred => "seat_surface_deferred",
            RefusalCode::SeatCannotWriteHere => "seat_cannot_write_here",
        }
    }
}

/// A refused call: the code and the exact payload string the caller sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: RefusalCode,
    pub message: String,
}

fn refuse(code: RefusalCode, message: String) -> Result<(), Refusal> {
    Err(Refusal { code, message })
}

/// The decision for one seat profile and its state at this moment. Precedence (REQ-091): not configured, suspended,
/// unverified (not checked for DoctorLiveProbe, which is how a seat GETS verified), then the surface rule.
fn decide(
    profile: &SeatProfile,
    surface: Surface,
    state: &SeatAvailability,
) -> Result<(), Refusal> {
    let seat = profile.key;
    // Existing seats are not gated on files they never write (no doctor probe, no suspension classifier).
    let state = SeatAvailability {
        configured: state.configured,
        verified: profile.verify_policy == VerifyPolicy::NotRequired || state.verified,
        suspended: match profile.suspension_policy {
            SuspensionPolicy::Classified => state.suspended.clone(),
            SuspensionPolicy::NotClassified => None,
        },
    };
    match state.first_blocker() {
        Some(Blocker::NotConfigured) => {
            return refuse(
                RefusalCode::SeatNotConfigured,
                format!(
                    "seat_not_configured: {seat}: set TRIUMVIRATE_ENABLE_{}=1 and restart the daemon",
                    seat.to_ascii_uppercase()
                ),
            );
        }
        Some(Blocker::Suspended(reason)) => {
            return refuse(
                RefusalCode::SeatSuspended,
                format!("seat_suspended: {seat}: {reason} (run: triumvirate seat clear {seat})"),
            );
        }
        Some(Blocker::Unverified) if surface != Surface::DoctorLiveProbe => {
            return refuse(
                RefusalCode::SeatUnverified,
                format!("seat_unverified: {seat}: run triumvirate doctor --live in a terminal"),
            );
        }
        Some(Blocker::Unverified) | None => {}
    }
    if profile.surfaces_allowed.contains(&surface) {
        return Ok(());
    }
    if surface == Surface::BlindValidate && !profile.can_write_test_file {
        return refuse(
            RefusalCode::SeatCannotWriteHere,
            format!(
                "seat_cannot_write_here: {seat}: blind validators must write test files and {seat} is read-only"
            ),
        );
    }
    refuse(
        RefusalCode::SeatSurfaceDeferred,
        format!("seat_surface_deferred: {seat} on {surface:?} ships in v1.1"),
    )
}

/// The pure decision, given the seat's state at this moment. A seat with no SeatProfile row is not a seat at all.
pub fn seat_gate_with(
    seat: &str,
    surface: Surface,
    origin: Origin,
    tty: bool,
    state: &SeatAvailability,
) -> Result<(), Refusal> {
    // A-2: no surface rule depends on origin or tty at launch.
    let _ = (origin, tty);
    match crate::seat_profile::seat_profile(seat) {
        Some(profile) => decide(profile, surface, state),
        None => refuse(
            RefusalCode::SeatNotConfigured,
            format!("seat_not_configured: {seat}: no such seat"),
        ),
    }
}

/// Installed CLI versions, registered by the daemon at startup (T-203/T-205 fill it from `<cli> --version`). A seat
/// with no registered version cannot match a verified.json, so it reads as unverified: the gate fails closed.
fn installed_versions() -> &'static Mutex<HashMap<String, String>> {
    static VERSIONS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    VERSIONS.get_or_init(Mutex::default)
}

/// The installed CLI version registered for a seat, if any (read by /status, Seven Primes T-203).
pub fn installed_cli_version(seat: &str) -> Option<String> {
    installed_versions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(seat)
        .cloned()
}

/// Record the installed CLI version for a seat (called at daemon start and after a CLI upgrade is detected).
pub fn register_installed_cli_version(seat: &str, version: &str) {
    installed_versions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(seat.to_string(), version.to_string());
}

/// The gate at a call site: reads the seat's state now (configured from its profile, availability files from
/// `$TRIUMVIRATE_HOME`), then decides.
pub fn seat_gate(seat: &str, surface: Surface, origin: Origin, tty: bool) -> Result<(), Refusal> {
    seat_gate_in(
        crate::seat_state::triumvirate_home(),
        seat,
        surface,
        origin,
        tty,
    )
}

/// `seat_gate`'s decision plus its telemetry (P-1): every refusal on every surface emits `tv_seat_gate_refused`.
fn seat_gate_in(
    home: Result<std::path::PathBuf, String>,
    seat: &str,
    surface: Surface,
    origin: Origin,
    tty: bool,
) -> Result<(), Refusal> {
    let decision = seat_gate_decide_in(home, seat, surface, origin, tty);
    if let Err(refusal) = &decision {
        crate::posthog::record_seat_gate_refused(seat, refusal.code, surface, origin);
    }
    decision
}

/// `seat_gate` with the state directory already resolved. Tests pass a tempdir here instead of rewriting the
/// process-global `TRIUMVIRATE_HOME`, which other modules' tests also rewrite under their own locks.
fn seat_gate_decide_in(
    home: Result<std::path::PathBuf, String>,
    seat: &str,
    surface: Surface,
    origin: Origin,
    tty: bool,
) -> Result<(), Refusal> {
    let Some(profile) = crate::seat_profile::seat_profile(seat) else {
        return seat_gate_with(
            seat,
            surface,
            origin,
            tty,
            &SeatAvailability {
                configured: false,
                verified: false,
                suspended: None,
            },
        );
    };
    let configured = match profile.configured {
        Configured::Always => true,
        Configured::EnvFlag(var) => std::env::var(var).is_ok_and(|v| v == "1"),
    };
    let version = installed_versions()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(seat)
        .cloned();
    let state = home
        .and_then(|home| {
            crate::seat_state::availability(&home, seat, configured, version.as_deref())
        })
        // Unreadable state fails closed: treated as unverified, never as available.
        .unwrap_or(SeatAvailability {
            configured,
            verified: false,
            suspended: None,
        });
    seat_gate_with(seat, surface, origin, tty, &state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(configured: bool, verified: bool, suspended: Option<&str>) -> SeatAvailability {
        SeatAvailability {
            configured,
            verified,
            suspended: suspended.map(String::from),
        }
    }

    /// The A-2 launch answer for a new seat, written straight from the spec (REQ-091 precedence, then the surface
    /// rule) and independent of decide(): not configured, then suspended, then unverified (never for DoctorLiveProbe),
    /// then AskAgent/Session/Jury/DoctorLiveProbe and (W-1, A-16) AbeDispatch/FleetSpawn/FleetTemporal allowed,
    /// BlindValidate read-only, everything else (PeerReviewRotation, Prewarm) deferred.
    fn expected(
        k: &str,
        surface: Surface,
        configured: bool,
        verified: bool,
        suspended: Option<&str>,
    ) -> Result<(), String> {
        if !configured {
            return Err(format!(
                "seat_not_configured: {k}: set TRIUMVIRATE_ENABLE_{}=1 and restart the daemon",
                k.to_uppercase()
            ));
        }
        if let Some(reason) = suspended {
            return Err(format!(
                "seat_suspended: {k}: {reason} (run: triumvirate seat clear {k})"
            ));
        }
        if !verified && surface != Surface::DoctorLiveProbe {
            return Err(format!(
                "seat_unverified: {k}: run triumvirate doctor --live in a terminal"
            ));
        }
        match surface {
            Surface::AskAgent
            | Surface::Session
            | Surface::Jury
            | Surface::DoctorLiveProbe
            | Surface::AbeDispatch
            | Surface::FleetSpawn
            | Surface::FleetTemporal => Ok(()),
            Surface::BlindValidate => Err(format!(
                "seat_cannot_write_here: {k}: blind validators must write test files and {k} is read-only"
            )),
            other => Err(format!(
                "seat_surface_deferred: {k} on {other:?} ships in v1.1"
            )),
        }
    }

    /// Every (new seat, surface, origin, tty, configured, verified, suspended) combination, the full Cartesian
    /// product of the availability state, with the exact payload from expected(). origin and tty are walked on
    /// purpose: under A-2 they must not change any answer.
    /// RED IF: any new seat's answer differs from the REQ-091 precedence plus the A-2 surface rule for any surface or
    /// any of the 8 availability states, origin/tty start mattering, or any triple yields seat_surface_refused.
    #[test]
    fn launch_table_for_every_new_seat_surface_origin_tty_and_state() {
        // The REAL rows (T-201 landed): kimi, mistral and qwen exactly as seat_profile() returns them.
        let mut refused_codes = 0;
        let mut checked = 0;
        for k in ["kimi", "mistral", "qwen"] {
            for &surface in Surface::ALL {
                for origin in [Origin::Interactive, Origin::Unattended] {
                    for tty in [true, false] {
                        for configured in [false, true] {
                            for verified in [false, true] {
                                for suspended in [None, Some("403 suspended")] {
                                    let got = seat_gate_with(
                                        k,
                                        surface,
                                        origin,
                                        tty,
                                        &st(configured, verified, suspended),
                                    )
                                    .map_err(|r| {
                                        assert!(
                                            r.message.starts_with(r.code.as_str()),
                                            "payload must start with its code: {}",
                                            r.message
                                        );
                                        r.message
                                    });
                                    assert_eq!(
                                        got,
                                        expected(k, surface, configured, verified, suspended),
                                        "{k} {surface:?} {origin:?} tty={tty} configured={configured} \
                                         verified={verified} suspended={suspended:?}"
                                    );
                                    if got
                                        .as_ref()
                                        .is_err_and(|m| m.starts_with("seat_surface_refused"))
                                    {
                                        refused_codes += 1;
                                    }
                                    checked += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(checked, 3 * Surface::ALL.len() * 2 * 2 * 8);
        assert_eq!(
            refused_codes, 0,
            "A-2: no launch triple yields seat_surface_refused"
        );
    }

    /// Spot checks named in the plan (rewritten for A-2) through the public entry point.
    /// RED IF: an existing seat starts being gated on state files, or an unknown seat stops being not configured.
    #[test]
    fn named_examples_through_seat_gate_with() {
        let ok = st(true, true, None);
        // Existing seats run everywhere, whatever the state files say (they write none).
        for seat in ["codex", "gemini", "grok", "claude", "deepseek"] {
            for &s in Surface::ALL {
                assert_eq!(
                    seat_gate_with(
                        seat,
                        s,
                        Origin::Unattended,
                        false,
                        &st(true, false, Some("x"))
                    ),
                    Ok(())
                );
            }
        }
        let e =
            seat_gate_with("zzz", Surface::AskAgent, Origin::Interactive, true, &ok).unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (
                RefusalCode::SeatNotConfigured,
                "seat_not_configured: zzz: no such seat"
            )
        );
        assert_eq!(
            RefusalCode::SeatSurfaceDeferred.as_str(),
            "seat_surface_deferred"
        );
    }

    /// Serialises this module's env mutation. No other mcp-bridge test reads TRIUMVIRATE_HOME or
    /// TRIUMVIRATE_ENABLE_KIMI, and the crate's other env locks are private to their own test modules.
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(Mutex::default)
    }

    /// Puts every captured env var back on drop, including when an assertion panics.
    struct EnvRestore(Vec<(&'static str, Option<std::ffi::OsString>)>);

    impl EnvRestore {
        fn capture(vars: &[&'static str]) -> Self {
            Self(vars.iter().map(|&v| (v, std::env::var_os(v))).collect())
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            for (var, value) in &self.0 {
                // SAFETY: serialised by env_lock(), which the caller holds for this guard's whole life.
                unsafe {
                    match value {
                        Some(v) => std::env::set_var(var, v),
                        None => std::env::remove_var(var),
                    }
                }
            }
        }
    }

    /// The wrapper itself, through real state files: the state directory at a tempdir, kimi enabled by its env flag and
    /// a verified.json written by seat_state. With no installed version registered (or a different one) seat_gate
    /// fails closed as unverified; the matching version makes it available; unsetting the flag is not configured.
    /// RED IF: seat_gate treats an unknown or mismatched installed CLI version as verified, stops reading
    /// verified.json from TRIUMVIRATE_HOME, or stops reading the seat's enable flag.
    #[test]
    fn wrapper_reads_state_files_and_fails_closed_without_a_version() {
        let _lock = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvRestore::capture(&["TRIUMVIRATE_ENABLE_KIMI"]);
        let home = tempfile::tempdir().unwrap();
        // SAFETY: serialised by env_lock(); _restore puts the value back. No other test touches this flag.
        unsafe {
            std::env::set_var("TRIUMVIRATE_ENABLE_KIMI", "1");
        }
        let rec = crate::seat_state::VerifiedRecord {
            cli_version: "9.9.9-gatetest".into(),
            seat_home: "/x".into(),
            probed_at: "t".into(),
            served_model: None,
        };
        crate::seat_state::write_verified(home.path(), "kimi", &rec).unwrap();
        // The state directory is passed in, not set through TRIUMVIRATE_HOME (deepseek_config's tests rewrite that
        // variable under their own lock, and the full suite raced them). seat_gate itself is one line over this.
        let gate = || {
            seat_gate_in(
                Ok(home.path().to_path_buf()),
                "kimi",
                Surface::AskAgent,
                Origin::Interactive,
                true,
            )
        };

        // No other test registers kimi, so this is the fail-closed starting point.
        assert_eq!(installed_cli_version("kimi"), None);
        let e = gate().unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (
                RefusalCode::SeatUnverified,
                "seat_unverified: kimi: run triumvirate doctor --live in a terminal"
            ),
            "no registered version must fail closed even with verified.json on disk"
        );

        register_installed_cli_version("kimi", "9.9.8-other");
        assert_eq!(
            gate().unwrap_err().code,
            RefusalCode::SeatUnverified,
            "a mismatched installed version voids verified.json"
        );

        register_installed_cli_version("kimi", "9.9.9-gatetest");
        assert_eq!(
            installed_cli_version("kimi").as_deref(),
            Some("9.9.9-gatetest")
        );
        assert_eq!(gate(), Ok(()));

        // SAFETY: serialised by env_lock(); _restore puts it back.
        unsafe { std::env::remove_var("TRIUMVIRATE_ENABLE_KIMI") };
        let e = gate().unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (
                RefusalCode::SeatNotConfigured,
                "seat_not_configured: kimi: set TRIUMVIRATE_ENABLE_KIMI=1 and restart the daemon"
            )
        );
    }

    /// P-1: one refusal on each of the ten surfaces, for both origins, through the gate's body (seat_gate is one line
    /// over seat_gate_in): each emits exactly one tv_seat_gate_refused with that surface and origin. kimi is not
    /// configured, so every surface refuses. A permitted call emits nothing.
    /// RED IF: a surface or origin is mislabeled, a refusal is silent, or an admitted call emits a refusal event.
    #[test]
    fn every_refusal_emits_one_gate_event_with_its_surface_and_origin() {
        let _lock = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _restore = EnvRestore::capture(&["TRIUMVIRATE_ENABLE_KIMI"]);
        // SAFETY: serialised by env_lock(); _restore puts the value back.
        unsafe { std::env::remove_var("TRIUMVIRATE_ENABLE_KIMI") };
        let home = tempfile::tempdir().unwrap();
        let names = [
            (Surface::AskAgent, "ask_agent"),
            (Surface::Session, "session"),
            (Surface::Jury, "jury"),
            (Surface::BlindValidate, "blind_validate"),
            (Surface::DoctorLiveProbe, "doctor_live_probe"),
            (Surface::PeerReviewRotation, "peer_review_rotation"),
            (Surface::FleetSpawn, "fleet_spawn"),
            (Surface::FleetTemporal, "fleet_temporal"),
            (Surface::AbeDispatch, "abe_dispatch"),
            (Surface::Prewarm, "prewarm"),
        ];
        assert_eq!(names.len(), Surface::ALL.len());
        for (surface, surface_name) in names {
            for (origin, origin_name) in [
                (Origin::Interactive, "interactive"),
                (Origin::Unattended, "unattended"),
            ] {
                let rec = crate::posthog::start_recording();
                let err = seat_gate_in(
                    Ok(home.path().to_path_buf()),
                    "kimi",
                    surface,
                    origin,
                    false,
                )
                .unwrap_err();
                assert_eq!(err.code, RefusalCode::SeatNotConfigured);
                assert_eq!(
                    rec.events(),
                    [(
                        "tv_seat_gate_refused".to_string(),
                        serde_json::json!({
                            "tv_agent": "kimi",
                            "tv_agent_display": "Kimi",
                            "tv_refusal_code": "seat_not_configured",
                            "tv_surface": surface_name,
                            "tv_origin": origin_name,
                        })
                    )],
                    "{surface:?} {origin:?}"
                );
            }
        }
        let rec = crate::posthog::start_recording();
        assert_eq!(
            seat_gate_in(
                Ok(home.path().to_path_buf()),
                "codex",
                Surface::AskAgent,
                Origin::Interactive,
                false
            ),
            Ok(())
        );
        assert!(rec.events().is_empty(), "{:?}", rec.events());
    }
}
