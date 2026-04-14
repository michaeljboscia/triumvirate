<script lang="ts">
  // T-016 (REQ-004, REQ-007) — Pantheon terminal panel.
  //
  // Mounts an xterm.js canvas, spawns a PTY child via the pty_spawn
  // Tauri command, wires the Channel for streaming output back to
  // term.write(), and forwards keystrokes + resize events back to the
  // Rust side. One panel per component instance.
  //
  // Scope for T-016: single hardcoded terminal. No tabs, no splits, no
  // directory picker (those are T-017). The cwd and command are passed
  // as props so the parent can decide what to spawn, but for Wave 5
  // the parent in +page.svelte hardcodes `claude` in ~/projects/triumvirate.
  //
  // Key design choices:
  //
  // 1. WebGL addon for rendering. xterm.js's default renderer is Canvas
  //    which caps around 30fps on typing echo. WebGL hits 60fps on
  //    M-series Macs and is the only way to get buttery scroll on a
  //    5000-line scrollback (REQ spec). Requires the addon to be
  //    loaded BEFORE `open()` is called or fallback to canvas.
  //
  // 2. Fit addon drives the Rust-side resize. We call `fit.fit()` in
  //    a ResizeObserver on the container, which updates term.cols/rows,
  //    then invoke pty_resize so the kernel sends SIGWINCH to the
  //    child. Without that the child keeps drawing at its initial
  //    dimensions and wrapping looks broken.
  //
  // 3. Base64 envelope for bytes. Tauri's JSON IPC can't carry raw
  //    Uint8Array efficiently — we'd lose bytes on UTF-8 boundaries.
  //    The Rust side base64s each PTY read chunk, we decode back to
  //    Uint8Array here, and feed it to term.write() which handles
  //    the boundary stitching internally.
  //
  // 4. Channel API instead of events. Per the mx-tauri-ipc skill,
  //    Channels are point-to-point, typed, and 10x faster than global
  //    app.emit() for streaming. For a PTY at 60fps this is the
  //    difference between smooth and laggy.

  import { onMount } from "svelte";
  import { invoke, Channel } from "@tauri-apps/api/core";
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebglAddon } from "@xterm/addon-webgl";
  import "@xterm/xterm/css/xterm.css";

  // Props for the parent to configure what gets spawned. The parent in
  // +page.svelte hardcodes these for T-016; T-017 makes them dynamic
  // via a directory picker.
  let {
    cwd,
    cmd = "claude",
    args = [],
  }: {
    cwd: string;
    cmd?: string;
    args?: string[];
  } = $props();

  // The div xterm.js paints into. Bound via bind:this. Must be in the
  // DOM before Terminal.open() is called, which is why everything
  // happens in onMount.
  let container: HTMLDivElement | null = $state(null);

  // Envelope shape must match Rust's `PtyOutputEvent` enum with
  // #[serde(tag = "type", rename_all = "snake_case")]. That produces
  // JSON like `{"type": "data", "b64": "..."}` or `{"type": "exit"}`.
  type PtyOutputEvent = { type: "data"; b64: string } | { type: "exit" };

  onMount(() => {
    if (!container) return;

    // 1. Build the xterm.js terminal with the Pantheon color palette.
    // Font stack favors installed programmer fonts but falls through
    // to the system monospace. 14px is the sweet spot for a dense
    // terminal that still scales well on Retina.
    const term = new Terminal({
      fontFamily:
        "'SF Mono', 'Menlo', 'Monaco', 'Fira Code', 'JetBrains Mono', monospace",
      fontSize: 14,
      lineHeight: 1.2,
      cursorBlink: true,
      // REQ — 5000-line scrollback. NOT 10000 per the spec; that's the
      // memory budget constraint.
      scrollback: 5000,
      theme: {
        background: "#1a1a1a",
        foreground: "#f6f6f6",
        cursor: "#f6f6f6",
        // ANSI 16 colors — conservative defaults, the spec calls out
        // more specific values in DESIGN_SYSTEM but those aren't loaded
        // as tokens yet. Good enough for T-016.
        black: "#000000",
        red: "#f87171",
        green: "#4ade80",
        yellow: "#fbbf24",
        blue: "#4a9eff",
        magenta: "#c084fc",
        cyan: "#22d3ee",
        white: "#e5e5e5",
        brightBlack: "#666666",
        brightRed: "#fca5a5",
        brightGreen: "#86efac",
        brightYellow: "#fcd34d",
        brightBlue: "#93c5fd",
        brightMagenta: "#d8b4fe",
        brightCyan: "#67e8f9",
        brightWhite: "#ffffff",
      },
    });

    // 2. Attach addons BEFORE open(). FitAddon reflows on container
    // resize; WebglAddon swaps the renderer from canvas to WebGL for
    // 60fps throughput on high-DPI displays.
    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);

    // 3. Mount into the DOM.
    term.open(container);

    // WebGL addon must be loaded AFTER open() per xterm.js docs —
    // it needs a mounted renderer to attach to. Wrap in try/catch
    // because WebGL context creation can fail on virtualized Macs
    // or when the 16-WebGL-context-per-WKWebView limit is hit.
    try {
      term.loadAddon(new WebglAddon());
    } catch (err) {
      console.warn("WebGL addon failed, falling back to canvas", err);
    }

    // 4. Initial fit — must happen after open() so xterm.js knows
    // the container's real dimensions. This sets term.cols/rows to
    // whatever fits the current container size.
    fitAddon.fit();

    // 5. Set up the output channel BEFORE calling pty_spawn. The
    // channel is passed as a command parameter; Rust stores it and
    // uses it for the lifetime of the PTY. We set onmessage before
    // sending so we don't miss any initial output chunks (Claude Code
    // prints its greeting as the first thing).
    const channel = new Channel<PtyOutputEvent>();
    channel.onmessage = (msg) => {
      if (msg.type === "data") {
        // Decode base64 → Uint8Array → term.write(). xterm.js accepts
        // Uint8Array directly and handles partial UTF-8 sequences
        // that span chunk boundaries — which happens frequently when
        // Claude Code streams long responses.
        const binary = atob(msg.b64);
        const bytes = new Uint8Array(binary.length);
        for (let i = 0; i < binary.length; i++) {
          bytes[i] = binary.charCodeAt(i);
        }
        term.write(bytes);
      } else if (msg.type === "exit") {
        // Session ended. T-017 replaces this with a proper "Session
        // ended" overlay UI with Restart/Close buttons; for T-016 we
        // just print a dim message into the terminal itself.
        term.write("\r\n\x1b[90m[pantheon] session ended\x1b[0m\r\n");
      }
    };

    // 6. Spawn the PTY. Invoke returns once the Rust side has stored
    // the handle and spawned the reader thread — the reader thread
    // is already pumping data into the channel by the time the
    // promise resolves.
    invoke<string>("pty_spawn", {
      onOutput: channel,
      cols: term.cols,
      rows: term.rows,
      cwd,
      cmd,
      args,
    })
      .then((sessionId) => {
        // T-019: the Rust side returns the PANTHEON_SESSION_ID it
        // stamped on the child environment. Stash it on the component
        // instance so later features (sidebar hover, worker-lineage
        // cross-reference against /api/workers) can display it.
        // Console log is intentional for now — Wave 6's T-021 will
        // consume this via a sessions store.
        console.info("[pantheon] pty spawned", { sessionId, cwd, cmd });
      })
      .catch((err) => {
        console.error("pty_spawn failed", err);
        term.write(
          `\r\n\x1b[31m[pantheon] pty_spawn failed: ${err}\x1b[0m\r\n`,
        );
      });

    // 7. Wire keystrokes → pty_write. xterm.js fires `onData` with
    // the raw bytes the user typed, already including ANSI sequences
    // for special keys (arrows, ctrl chars, etc). We base64 and
    // forward to the Rust side.
    const dataDisposable = term.onData((data) => {
      // TextEncoder gives us the UTF-8 bytes for any string. base64
      // is then a straightforward map over the byte array.
      const bytes = new TextEncoder().encode(data);
      let binary = "";
      for (const byte of bytes) binary += String.fromCharCode(byte);
      const b64 = btoa(binary);
      invoke("pty_write", { dataB64: b64 }).catch((err) => {
        console.error("pty_write failed", err);
      });
    });

    // 8. ResizeObserver → fit + pty_resize. Fires on every container
    // size change (window resize, sidebar toggle, etc). We debounce
    // lightly via requestAnimationFrame to coalesce rapid resize
    // events into at most one fit+pty_resize per frame.
    let rafId: number | null = null;
    const resizeObserver = new ResizeObserver(() => {
      if (rafId !== null) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        try {
          fitAddon.fit();
          invoke("pty_resize", { cols: term.cols, rows: term.rows }).catch(
            (err) => console.error("pty_resize failed", err),
          );
        } catch (err) {
          // fit() can throw if the container is detached mid-resize.
          // Not fatal; next resize tick will reconcile.
          console.warn("fit failed", err);
        }
      });
    });
    resizeObserver.observe(container);

    // 9. Cleanup on unmount. Order matters: stop forwarding keystrokes
    // and resize events first, then kill the PTY (so the child gets
    // SIGHUP cleanly), then dispose the terminal itself.
    return () => {
      dataDisposable.dispose();
      resizeObserver.disconnect();
      if (rafId !== null) cancelAnimationFrame(rafId);
      invoke("pty_kill").catch(() => {
        // Non-fatal — the child may have already exited.
      });
      term.dispose();
    };
  });
</script>

<div class="terminal-container" bind:this={container}></div>

<style>
  .terminal-container {
    /* The container's dimensions DRIVE the PTY rows/cols via the fit
     * addon. It must fill its flex parent exactly, with no padding on
     * this level (padding would create a phantom scroll area that
     * xterm.js's internal canvas doesn't see). */
    width: 100%;
    height: 100%;
    min-height: 0;
    min-width: 0;
    background-color: #1a1a1a;
  }

  /* The xterm.css import sets most of the viewport styles, but we
   * override the default padding because it creates a visible dark
   * strip around the canvas that feels unpolished next to the
   * sidebar/status rail borders. */
  .terminal-container :global(.xterm .xterm-viewport) {
    padding: 8px;
  }
  .terminal-container :global(.xterm) {
    padding: 0;
    height: 100%;
  }
</style>
