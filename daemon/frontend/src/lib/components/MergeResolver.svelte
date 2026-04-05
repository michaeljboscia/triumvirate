<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import type { FleetMergeResult, FleetStatus } from '../stores/fleet';

  export let fleetId = '';
  export let fleetStatus: FleetStatus | null = null;
  export let mergeResult: FleetMergeResult | null = null;
  export let mergeError: string | null = null;
  export let busy = false;

  const dispatch = createEventDispatcher<{
    merge: { fleetId: string; humanApproved: boolean };
    inspect: { fleetId: string };
  }>();

  let localFleetId = '';
  let humanApproved = true;

  $: localFleetId = localFleetId || fleetId;

  function inspect() {
    dispatch('inspect', { fleetId: localFleetId.trim() });
  }

  function merge() {
    dispatch('merge', { fleetId: localFleetId.trim(), humanApproved });
  }
</script>

<article class="card">
  <h2>Merge Resolver</h2>
  <div class="controls">
    <input bind:value={localFleetId} placeholder="fleet id" />
    <label>
      <input type="checkbox" bind:checked={humanApproved} />
      human approved
    </label>
    <button on:click={inspect}>Inspect</button>
    <button class="primary" on:click={merge} disabled={busy}>Merge</button>
  </div>

  {#if fleetStatus}
    <p class="status">
      tasks {fleetStatus.summary.task_completed}/{fleetStatus.summary.task_total}
      · worktrees {fleetStatus.summary.worktree_total}
    </p>
  {/if}

  {#if mergeError}
    <p class="error">{mergeError}</p>
  {/if}

  {#if mergeResult}
    <div class="result">
      <small>repo {mergeResult.repo_root}</small>
      <small>merged {mergeResult.merged_branches.length} branch(es)</small>
      {#if mergeResult.conflict}
        <small class="error">conflict at {mergeResult.failed_branch}</small>
      {:else}
        <small class="ok">merge completed cleanly</small>
      {/if}
    </div>
  {/if}
</article>

<style>
  .card {
    border: 1px solid rgba(167, 196, 226, 0.2);
    border-radius: 12px;
    padding: 1rem;
    background: rgba(17, 29, 48, 0.75);
  }

  h2 {
    margin: 0 0 0.75rem;
    font-size: 1rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #b5cae3;
  }

  .controls {
    display: grid;
    gap: 0.5rem;
    grid-template-columns: 1fr auto auto auto;
    align-items: center;
  }

  .controls > input {
    background: #122038;
    color: #e8f1ff;
    border: 1px solid rgba(167, 196, 226, 0.25);
    border-radius: 8px;
    padding: 0.55rem 0.65rem;
  }

  label {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    color: #b8cbe5;
    font-size: 0.78rem;
  }

  button {
    border: 1px solid rgba(144, 173, 205, 0.5);
    background: rgba(144, 173, 205, 0.14);
    color: #d0e4fb;
    border-radius: 8px;
    padding: 0.5rem 0.68rem;
    cursor: pointer;
  }

  button.primary {
    border-color: rgba(61, 214, 198, 0.6);
    background: rgba(61, 214, 198, 0.16);
    color: #a4f4ec;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .status {
    margin: 0.7rem 0 0;
    color: #b8cbe5;
    font-size: 0.78rem;
  }

  .result {
    margin-top: 0.6rem;
    display: grid;
    gap: 0.2rem;
  }

  .result small {
    color: #b8cbe5;
  }

  .ok {
    color: #68e7ac;
  }

  .error {
    color: #ff8f8f;
  }
</style>
