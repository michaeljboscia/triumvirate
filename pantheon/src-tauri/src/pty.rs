// T-016 (REQ-004, REQ-007) — Pantheon PTY host.
//
// Embeds a single Claude Code session inside a xterm.js canvas by
// spawning the `claude` binary under a pseudo-terminal, piping stdout +
// stderr back to the frontend via a Tauri IPC Channel, and accepting
// stdin + resize events via Tauri commands.
//
// Architecture (one terminal per process for T-016; T-017 multiplies):
//
//     Svelte TerminalPanel      Rust (this file)        Claude Code
//      xterm.js canvas            portable-pty            child
//        |                          |                       |
//        | --invoke pty_spawn()----->                        |
//        |      (passing Channel)    openpty() + spawn ---->|
//        |                          |                       |
//        |                          [dedicated std::thread]  |
//        |                          reads master.read_to_vec |
//        |                          base64-encode chunks     |
//        |<---channel.send(chunk)---|                        |
//        |     (onmessage fires)     |                       |
//        | term.write(decoded)      |                       |
//        |                          |                       |
//        | --invoke pty_write(data)->                        |
//        |                          writer.write_all(bytes) ->
//        |                          |                       |
//        | --invoke pty_resize(c,r)->                        |
//        |                          master.resize(PtySize)   |
//        |                          (SIGWINCH to child)      |
//
// Why Channel and not app.emit:
// PTY output is a high-frequency stream (60fps redraw on keystroke
// echo). Tauri events broadcast globally and have higher serialization
// overhead — the mx-tauri-ipc skill explicitly calls out 10x faster
// delivery via the Channel API. For a terminal emulator that's the
// difference between smooth typing and visible lag.
//
// Why std::thread and not tokio::spawn:
// portable-pty's `master.try_clone_reader()` returns a blocking reader
// — calling `.read()` on it blocks the calling thread. You CANNOT use
// tokio::spawn here: the spawned task would hog a worker thread and
// starve the rest of the runtime. The mx-rust-systems skill explicitly
// documents this failure mode. Use a dedicated OS thread.
//
// Why drop(pair.slave) immediately after spawn_command:
// The slave fd needs to be closed in the parent process so the child
// owns the only reference. Without this, `read()` on the master never
// returns EOF when the child exits, and the reader thread spins
// forever on the now-orphaned PTY.
//
// State model for T-016 (SIMPLE — single terminal):
// A single `Mutex<Option<PtyHandle>>` stored in Tauri state. T-017
// replaces this with a HashMap keyed by panel id when tabs land.

use std::sync::Mutex;

use base64::Engine;
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use serde::Serialize;
use tauri::ipc::Channel;

/// A single live PTY + its attached child process. Stored behind a
/// Mutex in Tauri state so command handlers can atomically write or
/// resize without racing the reader thread.
pub struct PtyHandle {
    /// PTY master — kept for `resize()` calls. Sending SIGWINCH to the
    /// child happens automatically when master.resize() is called.
    master: Box<dyn MasterPty + Send>,
    /// Write-half of the master used for stdin forwarding. Taken via
    /// `master.take_writer()` exactly once at spawn time.
    writer: Box<dyn std::io::Write + Send>,
    /// Handle to the spawned child. Used only for `kill()` — the
    /// child's lifetime otherwise follows the PTY master.
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

/// Global state wrapper. `Option` because T-016 allows "no terminal
/// yet" as the initial state (frontend creates one on mount).
#[derive(Default)]
pub struct PtyState(pub Mutex<Option<PtyHandle>>);

/// Envelope sent through the Tauri Channel for each PTY output chunk.
/// `data` is base64-encoded raw bytes from `master.read()`. We base64
/// rather than UTF-8-stringify because terminal streams can contain
/// partial multi-byte sequences at chunk boundaries — dropping invalid
/// bytes would corrupt split UTF-8. The frontend decodes back to bytes
/// and feeds them to `term.write(Uint8Array)` which handles the
/// boundary stitching internally.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PtyOutputEvent {
    /// Raw bytes from the PTY master, base64-encoded.
    Data { b64: String },
    /// The reader thread saw EOF — the child process has exited or the
    /// PTY was closed. Frontend should render "Session ended" UI.
    Exit,
}

/// T-016: spawn a PTY running `cmd` in `cwd` with an initial size of
/// `cols` x `rows`. Replaces any previously spawned PTY (single-slot
/// model for now). The `on_output` channel is the high-frequency
/// output stream — each chunk fires `on_output.send(PtyOutputEvent)`.
///
/// The `cwd` is passed verbatim to the child's working directory. For
/// T-016 testing this is hardcoded to `~/projects/triumvirate` from
/// the frontend side, but the backend accepts any absolute path.
#[tauri::command]
pub fn pty_spawn(
    state: tauri::State<PtyState>,
    on_output: Channel<PtyOutputEvent>,
    cols: u16,
    rows: u16,
    cwd: String,
    cmd: String,
    args: Vec<String>,
) -> Result<String, String> {
    // 1. Open the PTY pair. The default size must be non-zero; anything
    // less than 1 row or 1 col makes the child think the terminal is
    // invisible and many apps (including Claude Code) refuse to draw.
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty failed: {e}"))?;

    // 2. Build the child command. CommandBuilder is portable-pty's own
    // abstraction — we can't use std::process::Command here because
    // spawn_command takes the portable-pty type directly.
    let mut builder = CommandBuilder::new(&cmd);
    for arg in &args {
        builder.arg(arg);
    }
    builder.cwd(&cwd);

    // T-016 env inheritance: portable-pty gives the child a near-empty
    // environment by default — notably PATH is just "/usr/bin:/bin:
    // /usr/sbin:/sbin" which doesn't include Homebrew, Volta, or the
    // user's ~/.claude/local install. Iterate the parent process env
    // and apply each var so the spawned `claude` can be found.
    // This also propagates HOME, LANG, TERM-related vars that Claude
    // needs for correct rendering. The caveat: when Pantheon is
    // double-clicked from Finder (launchd), the parent env is itself
    // stripped; the user may need a PATH that includes the install
    // location baked into the launchd environment. For the dev path
    // this is sufficient.
    for (k, v) in std::env::vars() {
        builder.env(k, v);
    }

    // Belt-and-suspenders PATH: prepend the common install locations
    // for Claude CLI so a launchd-stripped environment still finds it.
    // This runs AFTER the inherit loop above, so if PATH was already
    // set by inheritance we prepend to it; otherwise we set from scratch.
    let extra_path = "/opt/homebrew/bin:/usr/local/bin";
    if let Some(home) = std::env::var_os("HOME") {
        let home_str = home.to_string_lossy();
        let existing = std::env::var("PATH").unwrap_or_default();
        let augmented = format!(
            "{home_str}/.local/bin:{home_str}/.claude/local:{home_str}/.volta/bin:{extra_path}:{existing}"
        );
        builder.env("PATH", augmented);
    } else {
        let existing = std::env::var("PATH").unwrap_or_default();
        builder.env("PATH", format!("{extra_path}:{existing}"));
    }
    // T-016 color fix: advertise full 256-color + truecolor capability
    // to the child. launchd doesn't set TERM when Pantheon launches
    // from Finder, so Claude Code (and any other color-aware CLI)
    // downgrades to monochrome by default. xterm.js supports both
    // 256-color palette and 24-bit truecolor, so set both vars. These
    // are applied AFTER the std::env::vars() inherit loop so they
    // overwrite whatever the parent had (e.g. TERM=dumb from launchd).
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");

    // T-019 (REQ-033): stamp the child with a unique PANTHEON_SESSION_ID
    // so any agent (Claude, Codex, Gemini) spawned inside this PTY can
    // be traced back to this specific Pantheon panel. The UUID also
    // propagates to every child of the Claude process — including the
    // MCP stdio proxy — so daemon-side worker lineage (handled by the
    // v3.9.0 DaemonState session-map) can associate dispatched workers
    // with their originating shell. The UUID is returned from this
    // command so the frontend can stash it in the sessions store for
    // later cross-reference against /api/workers data.
    let pantheon_session_id = uuid::Uuid::new_v4().to_string();
    builder.env("PANTHEON_SESSION_ID", &pantheon_session_id);

    // 3. Spawn the child into the PTY's slave side.
    let child = pair
        .slave
        .spawn_command(builder)
        .map_err(|e| format!("spawn_command failed: {e}"))?;

    // 4. CRITICAL: drop the slave in the parent so the child owns the
    // only reference. Without this, read() on the master never returns
    // EOF when the child exits, and the reader thread spins forever.
    drop(pair.slave);

    // 5. Take the writer for stdin forwarding. `take_writer` can only
    // be called ONCE — we stash the result in PtyHandle.
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("take_writer failed: {e}"))?;

    // 6. Clone a reader handle for the blocking read loop. The master
    // itself stays in PtyHandle so we can still resize it later.
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("try_clone_reader failed: {e}"))?;

    // 7. Spawn the OS thread that pumps bytes from the PTY into the
    // Tauri Channel. Channel is Clone + Send so we can move it in.
    let channel = on_output.clone();
    std::thread::spawn(move || {
        read_loop(reader, channel);
    });

    // 8. Store the handle in Tauri state, replacing any previous one.
    // Drop-order matters: writer first (closes stdin), master second
    // (closes pty), child last (SIGHUP then reap).
    let handle = PtyHandle {
        master: pair.master,
        writer,
        child,
    };
    {
        let mut guard = state.0.lock().map_err(|_| "pty state mutex poisoned".to_string())?;
        *guard = Some(handle);
    }

    // T-019: return the stamped UUID so the frontend can record the
    // linkage and display it on hover in the sidebar (future T-021).
    Ok(pantheon_session_id)
}

/// Blocking read loop running on a dedicated OS thread. Reads bytes
/// from the PTY master in 4KB chunks, base64-encodes each chunk, and
/// sends it through the Tauri Channel. Terminates on EOF (child exit)
/// or any read error — both emit a final `Exit` event.
fn read_loop(mut reader: Box<dyn std::io::Read + Send>, channel: Channel<PtyOutputEvent>) {
    let mut buf = vec![0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => {
                // Clean EOF — child closed its end of the PTY.
                let _ = channel.send(PtyOutputEvent::Exit);
                break;
            }
            Ok(n) => {
                let chunk = &buf[..n];
                let b64 = base64::engine::general_purpose::STANDARD.encode(chunk);
                if channel.send(PtyOutputEvent::Data { b64 }).is_err() {
                    // Channel closed — frontend unmounted, stop reading.
                    break;
                }
            }
            Err(err) => {
                // EIO on Linux when PTY closes; BSD/macOS raises
                // BrokenPipe. Either way we're done.
                tracing::debug!(error = %err, "pty read_loop: read error, exiting");
                let _ = channel.send(PtyOutputEvent::Exit);
                break;
            }
        }
    }
}

/// Write bytes to the PTY's stdin. Accepts a base64-encoded string
/// from the frontend — xterm.js produces bytes in Uint8Array form for
/// keystrokes, and base64 is the simplest JSON-safe envelope.
#[tauri::command]
pub fn pty_write(state: tauri::State<PtyState>, data_b64: String) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&data_b64)
        .map_err(|e| format!("base64 decode failed: {e}"))?;

    let mut guard = state
        .0
        .lock()
        .map_err(|_| "pty state mutex poisoned".to_string())?;
    let handle = guard
        .as_mut()
        .ok_or_else(|| "no pty spawned".to_string())?;
    handle
        .writer
        .write_all(&bytes)
        .map_err(|e| format!("pty write failed: {e}"))?;
    // Flush isn't strictly required for a PTY (the kernel already
    // line-buffers for us) but costs nothing and prevents surprise.
    handle
        .writer
        .flush()
        .map_err(|e| format!("pty flush failed: {e}"))?;
    Ok(())
}

/// Resize the PTY, sending SIGWINCH to the child automatically via
/// the portable-pty abstraction. Called on every xterm.js fit-addon
/// resize event — typically on window resize or sidebar toggle.
#[tauri::command]
pub fn pty_resize(state: tauri::State<PtyState>, cols: u16, rows: u16) -> Result<(), String> {
    let guard = state
        .0
        .lock()
        .map_err(|_| "pty state mutex poisoned".to_string())?;
    let handle = guard
        .as_ref()
        .ok_or_else(|| "no pty spawned".to_string())?;
    handle
        .master
        .resize(PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("pty resize failed: {e}"))?;
    Ok(())
}

/// Kill the spawned child process. Called from the frontend on
/// explicit session-end UI. For T-016 this is all-or-nothing — T-018
/// adds the "1 session still active. Quit anyway?" confirmation flow.
#[tauri::command]
pub fn pty_kill(state: tauri::State<PtyState>) -> Result<(), String> {
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "pty state mutex poisoned".to_string())?;
    if let Some(mut handle) = guard.take() {
        let _ = handle.child.kill();
    }
    Ok(())
}
