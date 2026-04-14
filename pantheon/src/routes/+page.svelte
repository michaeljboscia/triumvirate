<script lang="ts">
  // T-013 (REQ-001, REQ-002, REQ-027, REQ-029, REQ-034) — Pantheon shell layout.
  //
  // Three-region flexbox: sidebar (250px) | terminal area (flex:1) | status (280px).
  // Cmd+B toggles sidebar. Cmd+Shift+B toggles status area. Status area
  // auto-collapses below 1200px window width and re-expands above it (unless
  // the user has manually toggled it). Wave 5+ replaces the placeholder
  // <div>s with real terminal panels, sidebar tree, and status panels.
  //
  // T-020 wire-in: reads daemon.state + daemon.workers + daemon.fleet from
  // the store for the status-area preview. This is a placeholder view so
  // you can see data flowing from the daemon today — T-021/T-022 replace
  // these <p> tags with the real sidebar tree and status panels.

  import { daemon } from "$lib/stores/daemon.svelte";
  import TerminalPanel from "$lib/components/TerminalPanel.svelte";

  // T-016: hardcoded cwd for the initial terminal. T-017 replaces this
  // with a directory picker + recent projects list. The path must be
  // absolute and must exist; portable-pty's cwd is passed straight to
  // execve-like spawn.
  const INITIAL_CWD = "/Users/mikeboscia/projects/triumvirate";

  let sidebarOpen = $state(true);
  let statusOpen = $state(true);
  // Tracks whether the user has manually toggled status. If true, the
  // auto-collapse breakpoint stops fighting them.
  let statusManuallyToggled = $state(false);
  let windowWidth = $state(typeof window !== "undefined" ? window.innerWidth : 1400);

  // Window-width tracking for the 1200px auto-collapse breakpoint.
  $effect(() => {
    if (typeof window === "undefined") return;
    const onResize = () => {
      windowWidth = window.innerWidth;
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  });

  // Auto-collapse status area below 1200px UNLESS the user has overridden.
  $effect(() => {
    if (statusManuallyToggled) return;
    statusOpen = windowWidth >= 1200;
  });

  // Keyboard shortcuts. Cmd+B → sidebar, Cmd+Shift+B → status. Both fire
  // on macOS Command and Linux/Windows Ctrl so the same shell works for
  // dev runs on Linux during integration testing — though Pantheon ships
  // macOS-only per REQ-028.
  $effect(() => {
    if (typeof window === "undefined") return;
    const onKeyDown = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      if (e.key.toLowerCase() === "b") {
        e.preventDefault();
        if (e.shiftKey) {
          statusOpen = !statusOpen;
          statusManuallyToggled = true;
        } else {
          sidebarOpen = !sidebarOpen;
        }
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });
</script>

<svelte:head>
  <title>Pantheon</title>
</svelte:head>

<div class="shell">
  {#if sidebarOpen}
    <aside class="sidebar" aria-label="Workers and sessions sidebar">
      <header class="region-header">Workers · {daemon.workers.length}</header>
      {#if daemon.workers.length === 0}
        <p class="placeholder">No workers yet. Dispatch one via Claude or Codex.</p>
      {:else}
        <ul class="worker-list">
          {#each daemon.workers as w (w.session_id)}
            <li class="worker-row">
              <span class="worker-name">{w.name}</span>
              <span class="worker-status worker-status-{w.status}">{w.status}</span>
              <span class="worker-meta">{w.agent} · {Math.round(w.elapsed_ms / 1000)}s</span>
            </li>
          {/each}
        </ul>
      {/if}
      <p class="hint">⌘B to toggle · T-021 replaces with real tree</p>
    </aside>
  {/if}

  <main class="terminal-area" aria-label="Terminal panels">
    <!-- T-016: single hardcoded terminal running Claude Code in the
         triumvirate directory. T-017 replaces this with a TabBar +
         multiple panels managed by sessions.ts store. -->
    <TerminalPanel cwd={INITIAL_CWD} cmd="claude" args={[]} />
  </main>

  {#if statusOpen}
    <section class="status-area" aria-label="Status panels">
      <header class="region-header">Daemon</header>
      <div class="state-row">
        <span class="state-dot state-dot-{daemon.state}"></span>
        <span class="state-label">{daemon.state}</span>
      </div>
      <header class="region-header region-header-sub">Fleet · {daemon.fleet.length}</header>
      {#if daemon.fleet.length === 0}
        <p class="placeholder">No active fleet builds.</p>
      {:else}
        <ul class="fleet-list">
          {#each daemon.fleet as build (build.build_id)}
            <li class="fleet-row">
              <span class="fleet-id">{build.build_id}</span>
              <span class="fleet-counts">
                {build.completed}/{build.task_count}
                {#if build.failed > 0}<span class="fleet-fail">· {build.failed} fail</span>{/if}
              </span>
            </li>
          {/each}
        </ul>
      {/if}
      <p class="hint">⌘⇧B to toggle · T-022/T-023 replace with real panels</p>
    </section>
  {/if}
</div>

<style>
  :global(html), :global(body) {
    margin: 0;
    padding: 0;
    height: 100vh;
    overflow: hidden;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    color: #f6f6f6;
    background-color: #1a1a1a;
  }

  .shell {
    display: flex;
    height: 100vh;
    width: 100vw;
  }

  .sidebar {
    flex: 0 0 250px;
    width: 250px;
    border-right: 1px solid #333;
    background-color: #181818;
    padding: 12px;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }

  .terminal-area {
    flex: 1 1 auto;
    min-width: 0;
    background-color: #1a1a1a;
    /* T-016: no padding — the TerminalPanel component owns its own
     * internal padding (inside .xterm-viewport) so xterm.js's canvas
     * can measure the exact container width/height without phantom
     * scroll areas. */
    padding: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .status-area {
    flex: 0 0 280px;
    width: 280px;
    border-left: 1px solid #333;
    background-color: #181818;
    padding: 12px;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
  }

  .region-header {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #888;
    border-bottom: 1px solid #333;
    padding-bottom: 6px;
    margin-bottom: 12px;
  }

  .placeholder {
    margin: 0 0 8px 0;
    font-size: 13px;
    line-height: 1.5;
    color: #aaa;
  }

  .hint {
    margin: 0;
    font-size: 11px;
    color: #666;
    margin-top: auto;
  }

  /* T-020: placeholder preview styles for daemon data. Get replaced by
   * real Sidebar.svelte / StatusArea.svelte components in Wave 6. */
  .region-header-sub {
    margin-top: 16px;
  }

  .worker-list,
  .fleet-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .worker-row,
  .fleet-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    background-color: #212121;
    border-radius: 4px;
    font-size: 12px;
  }

  .worker-name,
  .fleet-id {
    color: #e6e6e6;
    font-weight: 500;
  }

  .worker-status {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #888;
  }

  .worker-status-working,
  .worker-status-in_progress {
    color: #4a9eff;
  }
  .worker-status-committed,
  .worker-status-completed {
    color: #4ade80;
  }
  .worker-status-failed {
    color: #f87171;
  }

  .worker-meta,
  .fleet-counts {
    font-size: 10px;
    color: #888;
  }

  .fleet-fail {
    color: #f87171;
  }

  .state-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 0 12px 0;
  }

  .state-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    display: inline-block;
  }
  /* T-020 fix: no animation on state-dot-starting. Pulsing + rapid
   * state transitions during reconnect was a flashing risk — static
   * colors only until the state machine is proven stable in Wave 6. */
  .state-dot-starting {
    background-color: #fbbf24;
  }
  .state-dot-ready {
    background-color: #4ade80;
  }
  .state-dot-degraded {
    background-color: #fb923c;
  }
  .state-dot-disconnected {
    background-color: #f87171;
  }

  .state-label {
    font-size: 12px;
    color: #e6e6e6;
    text-transform: capitalize;
  }
</style>
