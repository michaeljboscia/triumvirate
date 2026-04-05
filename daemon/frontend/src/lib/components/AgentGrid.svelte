<script lang="ts">
  import type { AgentView } from '../stores/agents';

  export let agents: AgentView[] = [];
</script>

<article class="card">
  <h2>Agents</h2>
  {#if agents.length === 0}
    <p class="muted">No agents loaded yet.</p>
  {:else}
    <ul>
      {#each agents as agent}
        <li>
          <strong>{agent.name}</strong>
          <span>{agent.model}</span>
          <span class={`status status-${agent.status}`}>{agent.status}</span>
        </li>
      {/each}
    </ul>
  {/if}
</article>

<style>
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
