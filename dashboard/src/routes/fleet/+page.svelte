<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    connectFleetStream,
    disconnectFleetStream,
    fleetSnapshot,
    kanbanColumns,
    type FleetTaskState
  } from '$lib/stores/fleet';

  const order: FleetTaskState[] = ['pending', 'claimed', 'in_progress', 'done', 'failed'];

  onMount(() => {
    connectFleetStream();
  });

  onDestroy(() => {
    disconnectFleetStream();
  });
</script>

<svelte:head>
  <title>Fleet | Triumvirate Dashboard</title>
</svelte:head>

<main class="page">
  <header class="header">
    <div>
      <p class="eyebrow">Parallel Reviews, Sequential Merge</p>
      <h1>Fleet</h1>
    </div>
    <div class="fleet-id">{$fleetSnapshot.fleet_id}</div>
  </header>

  <section class="queue">
    <h2>Merge Queue</h2>
    {#if $fleetSnapshot.merge_queue.length === 0}
      <p class="empty">No queued merges.</p>
    {:else}
      <ol>
        {#each $fleetSnapshot.merge_queue as taskId}
          <li>{taskId}</li>
        {/each}
      </ol>
    {/if}
  </section>

  <section class="kanban">
    {#each order as state}
      <article class="column">
        <header>
          <h2>{state.replace('_', ' ')}</h2>
          <span>{$kanbanColumns[state].length}</span>
        </header>
        {#if $kanbanColumns[state].length === 0}
          <p class="empty">No tasks</p>
        {:else}
          <ul>
            {#each $kanbanColumns[state] as task (task.task_id)}
              <li>
                <strong>{task.task_id}</strong>
                <small>{task.agent ?? 'unassigned'}</small>
                <small>{task.worktree ?? 'worktree pending'}</small>
              </li>
            {/each}
          </ul>
        {/if}
      </article>
    {/each}
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
    justify-content: space-between;
    align-items: end;
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

  .fleet-id {
    border: 1px solid var(--color-border);
    border-radius: 999px;
    padding: 0.35rem 0.7rem;
    font-size: 0.78rem;
    color: var(--color-text-muted);
  }

  .queue,
  .column {
    border: 1px solid var(--color-border);
    border-radius: 10px;
    background: rgb(26 29 39 / 0.82);
  }

  .queue {
    padding: 0.9rem;
  }

  .queue h2 {
    margin: 0;
    font-size: 0.98rem;
  }

  .queue ol {
    margin: 0.8rem 0 0;
    padding-left: 1rem;
  }

  .kanban {
    display: grid;
    gap: 0.75rem;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  }

  .column {
    padding: 0.7rem;
  }

  .column header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.65rem;
  }

  .column h2 {
    margin: 0;
    font-size: 0.86rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .column span {
    border-radius: 999px;
    border: 1px solid var(--color-border);
    padding: 0.05rem 0.45rem;
    font-size: 0.72rem;
    color: var(--color-text-muted);
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.6rem;
  }

  li {
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: 0.55rem;
    background: rgb(15 17 23 / 0.75);
    display: grid;
    gap: 0.2rem;
  }

  strong {
    font-size: 0.9rem;
  }

  small,
  .empty {
    color: var(--color-text-muted);
    font-size: 0.78rem;
    margin: 0;
  }
</style>
