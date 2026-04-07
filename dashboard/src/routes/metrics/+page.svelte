<script lang="ts">
  import { onDestroy, onMount } from 'svelte';

  interface MetricRow {
    name: string;
    value: number;
  }

  let rows: MetricRow[] = [];
  let lastUpdated = '';
  let pollTimer: number | null = null;

  function parsePrometheus(text: string): MetricRow[] {
    const parsed: MetricRow[] = [];
    for (const line of text.split('\n')) {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith('#')) {
        continue;
      }
      const match = trimmed.match(/^([a-zA-Z_:][a-zA-Z0-9_:]*)\s+([+-]?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)$/);
      if (!match) {
        continue;
      }
      parsed.push({ name: match[1], value: Number(match[2]) });
    }
    return parsed;
  }

  function tokenRows(items: MetricRow[]): MetricRow[] {
    return items.filter((row) => row.name.includes('agent_tokens_total') || row.name.includes('agent_requests_total'));
  }

  function latencyRows(items: MetricRow[]): MetricRow[] {
    return items.filter((row) => row.name.includes('agent_duration_seconds'));
  }

  async function load(): Promise<void> {
    try {
      const res = await fetch('/metrics');
      if (!res.ok) {
        return;
      }
      const text = await res.text();
      rows = parsePrometheus(text);
      lastUpdated = new Date().toLocaleTimeString();
    } catch {
      // keep old data if fetch fails
    }
  }

  onMount(() => {
    void load();
    pollTimer = window.setInterval(() => {
      void load();
    }, 10000);
  });

  onDestroy(() => {
    if (pollTimer !== null) {
      window.clearInterval(pollTimer);
      pollTimer = null;
    }
  });
</script>

<svelte:head>
  <title>Metrics | Triumvirate Dashboard</title>
</svelte:head>

<main class="page">
  <header>
    <p class="eyebrow">Prometheus Snapshot</p>
    <h1>Metrics</h1>
    <small>Last updated: {lastUpdated || 'n/a'}</small>
  </header>

  <section class="panel">
    <h2>Agent Token + Request Totals</h2>
    {#if tokenRows(rows).length === 0}
      <p class="empty">No token/request metrics yet.</p>
    {:else}
      <table>
        <thead>
          <tr><th>Metric</th><th>Value</th></tr>
        </thead>
        <tbody>
          {#each tokenRows(rows) as row (row.name)}
            <tr>
              <td>{row.name}</td>
              <td>{row.value}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  <section class="panel">
    <h2>Latency Histogram/Counters</h2>
    {#if latencyRows(rows).length === 0}
      <p class="empty">No latency metrics yet.</p>
    {:else}
      <table>
        <thead>
          <tr><th>Metric</th><th>Value</th></tr>
        </thead>
        <tbody>
          {#each latencyRows(rows) as row (row.name)}
            <tr>
              <td>{row.name}</td>
              <td>{row.value}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>
</main>

<style>
  .page {
    padding: 1.5rem;
    display: grid;
    gap: 1rem;
  }

  .eyebrow {
    margin: 0;
    font-size: 0.75rem;
    color: var(--color-text-muted);
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  h1 {
    margin: 0.2rem 0;
    font-size: clamp(1.6rem, 3vw, 2rem);
  }

  header small,
  .empty {
    color: var(--color-text-muted);
    font-size: 0.78rem;
  }

  .panel {
    border: 1px solid var(--color-border);
    border-radius: 10px;
    background: rgb(26 29 39 / 0.82);
    padding: 0.8rem;
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    margin-top: 0.6rem;
    font-size: 0.85rem;
  }

  th,
  td {
    border-bottom: 1px solid var(--color-border);
    text-align: left;
    padding: 0.42rem;
    vertical-align: top;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
  }
</style>
