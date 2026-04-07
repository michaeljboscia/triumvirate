<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    connectLedgerStream,
    disconnectLedgerStream,
    ledgerHealth,
    ledgerResults,
    searchLedger
  } from '$lib/stores/ledger';

  let query = '';

  onMount(() => {
    connectLedgerStream();
  });

  onDestroy(() => {
    disconnectLedgerStream();
  });

  $: healthClass = $ledgerHealth.status;
  $: shouldPulse = $ledgerHealth.status === 'dead';
</script>

<svelte:head>
  <title>Ledger | Triumvirate Dashboard</title>
</svelte:head>

<main class="page">
  <header class="header">
    <div>
      <p class="eyebrow">Memory + Retrieval</p>
      <h1>Ledger</h1>
    </div>
    <div class="health {healthClass} {shouldPulse ? 'pulse' : ''}">
      <span class="dot" aria-hidden="true"></span>
      <span>{($ledgerHealth.status || 'unknown').toUpperCase()}</span>
    </div>
  </header>

  <section class="metrics">
    <div>
      <p>Compression queue depth</p>
      <strong>{$ledgerHealth.queue_depth}</strong>
    </div>
    <div>
      <p>Spool size (bytes)</p>
      <strong>{$ledgerHealth.spool_size_bytes}</strong>
    </div>
    <div>
      <p>Events (last 5m)</p>
      <strong>{$ledgerHealth.events_last_5min}</strong>
    </div>
  </section>

  <section class="search">
    <label for="ledger-query">FTS5 Search</label>
    <div class="search-row">
      <input
        id="ledger-query"
        bind:value={query}
        placeholder="authentication middleware bug"
      />
      <button type="button" on:click={() => void searchLedger(query)}>Search</button>
    </div>

    {#if $ledgerResults.length === 0}
      <p class="empty">No search results yet.</p>
    {:else}
      <ul>
        {#each $ledgerResults as row (`${row.session_id}-${row.title}`)}
          <li>
            <div class="row-top">
              <strong>{row.title}</strong>
              <small>{row.summary_type}</small>
            </div>
            <p>{row.narrative}</p>
            <small class="session">session: {row.session_id}</small>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</main>

<style>
  .page {
    padding: 1.5rem;
    display: grid;
    gap: 1.2rem;
  }

  .header {
    display: flex;
    align-items: end;
    justify-content: space-between;
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
    margin: 0.2rem 0 0;
    font-size: clamp(1.6rem, 3vw, 2rem);
  }

  .health {
    display: inline-flex;
    align-items: center;
    gap: 0.45rem;
    border-radius: 999px;
    border: 1px solid var(--color-border);
    padding: 0.35rem 0.75rem;
    font-size: 0.78rem;
  }

  .dot {
    width: 0.68rem;
    height: 0.68rem;
    border-radius: 50%;
    background: #71717a;
  }

  .health.healthy .dot {
    background: #22c55e;
  }

  .health.degraded .dot {
    background: #eab308;
  }

  .health.dead .dot {
    background: #ef4444;
  }

  .pulse .dot {
    animation: pulse 1.15s infinite;
  }

  @keyframes pulse {
    0% { transform: scale(1); opacity: 1; }
    50% { transform: scale(1.22); opacity: 0.45; }
    100% { transform: scale(1); opacity: 1; }
  }

  .metrics {
    display: grid;
    gap: 0.65rem;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  }

  .metrics div,
  .search {
    border: 1px solid var(--color-border);
    border-radius: 10px;
    background: rgb(26 29 39 / 0.82);
    padding: 0.8rem;
  }

  .metrics p {
    margin: 0;
    font-size: 0.78rem;
    color: var(--color-text-muted);
  }

  .metrics strong {
    display: inline-block;
    margin-top: 0.3rem;
    font-size: 1.1rem;
  }

  .search-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0.55rem;
    margin-top: 0.55rem;
  }

  input,
  button {
    border-radius: 8px;
    border: 1px solid var(--color-border);
    background: rgb(15 17 23 / 0.7);
    color: var(--color-text);
    padding: 0.5rem 0.65rem;
  }

  button {
    cursor: pointer;
    background: rgb(34 197 94 / 0.15);
  }

  ul {
    margin: 0.8rem 0 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.65rem;
  }

  li {
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: 0.6rem;
    background: rgb(15 17 23 / 0.78);
  }

  .row-top {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    align-items: center;
  }

  .row-top small,
  .session,
  .empty {
    color: var(--color-text-muted);
    font-size: 0.78rem;
  }

  li p {
    margin: 0.35rem 0;
    line-height: 1.45;
  }
</style>
