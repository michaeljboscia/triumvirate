<script lang="ts">
  import { onMount } from 'svelte';
  import { agents, refreshAgents } from '../lib/stores/agents';
  import { quota, refreshQuota } from '../lib/stores/quota';
  import { tasks, refreshTasks } from '../lib/stores/tasks';
  import { workflows, refreshWorkflows } from '../lib/stores/workflow';

  const title = 'Triumvirate v2 Dashboard';
  let timer: ReturnType<typeof setInterval> | undefined;

  async function refreshAll() {
    await Promise.all([refreshAgents(), refreshTasks(), refreshQuota(), refreshWorkflows()]);
  }

  onMount(() => {
    void refreshAll();
    timer = setInterval(() => {
      void refreshAll();
    }, 3000);

    return () => {
      if (timer) clearInterval(timer);
    };
  });
</script>

<main class="page">
  <header class="hero">
    <h1>{title}</h1>
    <p>Live orchestration shell (Phase 5.2 stores wired)</p>
  </header>

  <section class="grid two-up">
    <article class="card">
      <h2>Agents</h2>
      {#if $agents.length === 0}
        <p class="muted">No agents loaded yet.</p>
      {:else}
        <ul>
          {#each $agents as agent}
            <li>
              <strong>{agent.name}</strong>
              <span>{agent.model}</span>
              <span class={`status status-${agent.status}`}>{agent.status}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </article>

    <article class="card">
      <h2>Quota</h2>
      <pre>{JSON.stringify($quota, null, 2)}</pre>
    </article>

    <article class="card">
      <h2>Fleet Tasks</h2>
      <pre>{JSON.stringify($tasks.slice(0, 12), null, 2)}</pre>
    </article>

    <article class="card">
      <h2>Workflows</h2>
      <pre>{JSON.stringify($workflows, null, 2)}</pre>
    </article>
  </section>
</main>

<style>
  .page {
    max-width: 1080px;
    margin: 2.5rem auto;
    padding: 0 1rem 2rem;
  }

  .hero {
    margin-bottom: 1.25rem;
  }

  .hero h1 {
    margin: 0;
    font-size: 2.2rem;
    letter-spacing: 0.02em;
  }

  .hero p {
    color: var(--muted);
  }

  .grid {
    display: grid;
    gap: 1rem;
  }

  .two-up {
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  }

  .card {
    border: 1px solid rgba(167, 196, 226, 0.2);
    border-radius: 12px;
    padding: 1rem;
    background: rgba(17, 29, 48, 0.75);
    backdrop-filter: blur(2px);
  }

  .card h2 {
    margin: 0 0 0.75rem;
    font-size: 1rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #b5cae3;
  }

  .muted {
    color: var(--muted);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.5rem;
  }

  li {
    display: grid;
    grid-template-columns: 1fr auto auto;
    gap: 0.5rem;
    align-items: center;
    padding: 0.4rem 0;
    border-bottom: 1px dashed rgba(167, 196, 226, 0.12);
  }

  pre {
    margin: 0;
    font-size: 0.8rem;
    line-height: 1.4;
    color: #b9d7fb;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .status {
    padding: 0.1rem 0.45rem;
    border-radius: 999px;
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    border: 1px solid transparent;
  }

  .status-ready { color: #68e7ac; border-color: #2b9a67; }
  .status-busy { color: #ffd978; border-color: #c39a1c; }
  .status-starting, .status-restarting { color: #88c8ff; border-color: #2f6ea7; }
  .status-unresponsive, .status-dead { color: #ff8f8f; border-color: #a74949; }
</style>
