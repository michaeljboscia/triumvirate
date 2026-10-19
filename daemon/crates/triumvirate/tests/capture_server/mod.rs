//! Seven Primes P-1: a local stand-in for PostHog's capture endpoint. One std TcpListener on 127.0.0.1, one thread,
//! every POST body recorded as JSON. Tests point POSTHOG_HOST here (with a fake POSTHOG_API_KEY), so nothing they run
//! ever reaches the real PostHog.

// Shared by two test crates; each uses a different subset.
#![allow(dead_code)]

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde_json::Value;

/// The transport credential tests hand the daemon: a fixture value, never a real ingest key.
pub const TEST_INGEST_VALUE: &str = "test-key";

/// The uppercase and the lowercase code-shaped leak sentinels.
pub const SENTINELS: [&str; 2] = ["ZQX-LEAK-7731", "customer_secret_zqx"];

pub struct CaptureServer {
    pub url: String,
    bodies: Arc<Mutex<Vec<Value>>>,
}

impl CaptureServer {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind capture server");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let bodies: Arc<Mutex<Vec<Value>>> = Arc::default();
        let sink = bodies.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
                let mut length = 0usize;
                let mut line = String::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':')
                        && k.eq_ignore_ascii_case("content-length")
                    {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                if reader.read_exact(&mut body).is_ok()
                    && let Ok(json) = serde_json::from_slice::<Value>(&body)
                {
                    sink.lock().unwrap().push(json);
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                );
            }
        });
        Self { url, bodies }
    }

    /// Every request body received so far.
    pub fn bodies(&self) -> Vec<Value> {
        self.bodies.lock().unwrap().clone()
    }

    /// The properties of every received `event`.
    pub fn events(&self, event: &str) -> Vec<Value> {
        self.bodies()
            .into_iter()
            .filter(|b| b["event"] == event)
            .map(|b| b["properties"].clone())
            .collect()
    }

    /// Poll until at least `n` `event`s whose properties satisfy `pred` arrived (capture is fire-and-forget), or fail
    /// after 10 s.
    pub fn wait_for(&self, event: &str, n: usize, pred: impl Fn(&Value) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let got: Vec<Value> = self.events(event).into_iter().filter(|p| pred(p)).collect();
            if got.len() >= n {
                return got;
            }
            assert!(
                Instant::now() < deadline,
                "{event}: {} of {n} arrived; all bodies: {:?}",
                got.len(),
                self.bodies()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Every body is a well-formed capture call carrying the test credential, and no tv_seat_* event's properties
    /// contain either sentinel. `$ai_generation` carries the prompt in `$ai_input` by its own (pre-P-1) design, so
    /// the sentinel check covers the events P-1 adds.
    pub fn assert_clean(&self) {
        let bodies = self.bodies();
        assert!(!bodies.is_empty(), "nothing was captured");
        for b in &bodies {
            assert_eq!(b["api_key"], TEST_INGEST_VALUE, "{b}");
            assert_eq!(b["distinct_id"], "triumvirate-daemon", "{b}");
            let event = b["event"].as_str().unwrap_or_default();
            if event.starts_with("tv_seat_") {
                let props = b["properties"].to_string();
                for s in SENTINELS {
                    assert!(!props.contains(s), "{event} carried {s}: {props}");
                }
            }
        }
    }
}

/// The documented property keys of each tv_seat_* event (tv_run_tag is checked separately).
pub fn documented_keys(event: &str) -> &'static [&'static str] {
    match event {
        "tv_seat_gate_refused" => &["tv_refusal_code", "tv_surface", "tv_origin"],
        "tv_seat_lock_wait" => &["tv_waited_ms", "tv_outcome"],
        "tv_seat_quota" => &["tv_quota_signal"],
        "tv_seat_suspended" => &["tv_suspend_signal"],
        "tv_seat_containment" => &["tv_containment_code", "tv_paths_count"],
        "tv_seat_doctor_probe" => &["tv_outcome", "tv_code", "tv_cli_version", "tv_served_model"],
        "tv_seat_session" => &["tv_session_event", "tv_reason_code"],
        "tv_seat_served_model_mismatch" => &["tv_served_model"],
        "tv_seat_writer_run" => &[
            "tv_surface",
            "tv_outcome",
            "tv_files_changed",
            "tv_duration_ms",
        ],
        other => panic!("not a P-1 event: {other}"),
    }
}

/// `props` has exactly tv_agent, tv_agent_display, the event's documented keys, and tv_run_tag iff `run_tag` is set
/// (equal to it).
pub fn assert_keys(event: &str, props: &Value, run_tag: Option<&str>) {
    let mut want: Vec<&str> = vec!["tv_agent", "tv_agent_display"];
    want.extend(documented_keys(event));
    if run_tag.is_some() {
        want.push("tv_run_tag");
    }
    want.sort_unstable();
    let mut got: Vec<&str> = props
        .as_object()
        .expect("properties object")
        .keys()
        .map(String::as_str)
        .collect();
    got.sort_unstable();
    assert_eq!(got, want, "{event}: {props}");
    if let Some(tag) = run_tag {
        assert_eq!(props["tv_run_tag"], tag, "{event}: {props}");
    }
}
