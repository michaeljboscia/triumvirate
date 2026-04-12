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

  import { daemon } from "$lib/stores/daemon";

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
      <header class="region-header">Sidebar</header>
      <p class="placeholder">Worker hierarchy lands in T-021.</p>
      <p class="hint">⌘B to toggle</p>
    </aside>
  {/if}

  <main class="terminal-area" aria-label="Terminal panels">
    <header class="region-header">Terminal Area</header>
    <p class="placeholder">Terminal panels land in T-016 and T-017.</p>
    <p class="hint">Window: {windowWidth}px wide</p>
  </main>

  {#if statusOpen}
    <section class="status-area" aria-label="Status panels">
      <header class="region-header">Status</header>
      <p class="placeholder">Status panels land in T-022 and T-023.</p>
      <p class="hint">⌘⇧B to toggle · auto-collapses &lt;1200px</p>
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
    padding: 12px;
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
</style>
