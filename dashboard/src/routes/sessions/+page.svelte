<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    agents,
    connectAgentStream,
    currentVerbosity,
    disconnectAgentStream,
    filteredEvents,
    verbosity,
    type Verbosity
  } from '$lib/stores/agents';

  const levels: Verbosity[] = ['minimal', 'standard', 'detailed', 'raw'];
  let selected: Verbosity = currentVerbosity();

  $: verbosity.set(selected);

  onMount(() => {
    connectAgentStream();
  });

  onDestroy(() => {
    disconnectAgentStream();
  });

  function formatTime(ts?: number): string {
    if (!ts) {
      return 'n/a';
    }
    return new Date(ts).toLocaleTimeString();
  }
</script>

<svelte:head>
  <title>Sessions | Triumvirate Dashboard</title>
</svelte:head>

<main class="page">
  <header class="header">
    <div>
      <p class="eyebrow">Live State</p>
      <h1>Sessions</h1>
    </div>
    <label class="verbosity">
      Verbosity
      <select bind:value={selected} aria-label="Verbosity selector">
        {#each levels as level}
          <option value={level}>{level}</option>
        {/each}
      </select>
    </label>
  </header>

  <section class="grid">
    {#each $agents as agent (agent.agent)}
      <article class="agent-card">
        <div class="agent-top">
          <strong>{agent.agent}</strong>
          <span class="state">{agent.state}</span>
        </div>
        <small>Last update: {formatTime(agent.updatedAt)}</small>
      </article>
    {/each}
  </section>

  <section class="events">
    <h2>Event Stream</h2>
    {#if $filteredEvents.length === 0}
      <p class="empty">No events for the selected verbosity.</p>
    {:else}
      <ul>
        {#each $filteredEvents as event, index (`${event.raw}-${index}`)}
          <li>
            <span class="event-type">{event.type}</span>
            <span class="event-time">{formatTime(event.ts_ms)}</span>
            <pre>{event.raw}</pre>
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
    gap: 1.5rem;
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
    text-transform: uppercase;
    color: var(--color-text-muted);
    letter-spacing: 0.06em;
  }

  h1 {
    margin: 0.2rem 0 0;
    font-size: clamp(1.6rem, 3vw, 2rem);
  }

  .verbosity {
    display: grid;
    gap: 0.35rem;
    font-size: 0.85rem;
    color: var(--color-text-muted);
  }

  .verbosity select {
    min-width: 10rem;
    border-radius: 8px;
    border: 1px solid var(--color-border);
    background: #1a1d27;
    color: var(--color-text);
    padding: 0.45rem 0.6rem;
  }

  .grid {
    display: grid;
    gap: 0.75rem;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
  }

  .agent-card {
    border: 1px solid var(--color-border);
    border-radius: 10px;
    padding: 0.8rem;
    background: rgb(26 29 39 / 0.85);
  }

  .agent-top {
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    align-items: center;
    margin-bottom: 0.5rem;
  }

  .state {
    display: inline-flex;
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    background: rgb(59 130 246 / 0.16);
    color: #60a5fa;
    font-size: 0.75rem;
  }

  .events {
    border: 1px solid var(--color-border);
    border-radius: 12px;
    padding: 1rem;
    background: rgb(26 29 39 / 0.8);
  }

  .events h2 {
    margin: 0 0 0.8rem;
    font-size: 1.05rem;
  }

  .events ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.8rem;
  }

  .events li {
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: 0.7rem;
    background: rgb(15 17 23 / 0.75);
  }

  .event-type {
    font-size: 0.72rem;
    color: #93c5fd;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-right: 0.7rem;
  }

  .event-time {
    font-size: 0.78rem;
    color: var(--color-text-muted);
  }

  pre {
    margin: 0.55rem 0 0;
    overflow: auto;
    white-space: pre-wrap;
    font-size: 0.78rem;
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
  }

  .empty {
    color: var(--color-text-muted);
    margin: 0;
  }
</style>
