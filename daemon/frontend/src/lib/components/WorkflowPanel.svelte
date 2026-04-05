<script lang="ts">
  import type { WorkflowSummary } from '../stores/workflow';

  export let workflows: WorkflowSummary[] = [];

  function percent(step: number): number {
    return Math.max(10, Math.min(100, step * 25));
  }
</script>

<article class="card">
  <h2>Workflow Panel</h2>
  {#if workflows.length === 0}
    <p class="muted">No active workflows found.</p>
  {:else}
    <ul>
      {#each workflows as workflow}
        <li>
          <div class="top">
            <strong>{workflow.workflow_type}</strong>
            <small>{workflow.state}</small>
          </div>
          <code>{workflow.workflow_id}</code>
          <div class="bar-wrap">
            <div class="bar" style={`width:${percent(workflow.current_step)}%`}></div>
          </div>
          <small>step {workflow.current_step}</small>
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
  }

  h2 {
    margin: 0 0 0.75rem;
    font-size: 1rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #b5cae3;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.7rem;
  }

  li {
    border: 1px solid rgba(167, 196, 226, 0.2);
    border-radius: 10px;
    padding: 0.55rem 0.65rem;
    background: rgba(10, 20, 38, 0.7);
    display: grid;
    gap: 0.25rem;
  }

  .top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
  }

  .top strong {
    text-transform: uppercase;
    font-size: 0.78rem;
    letter-spacing: 0.04em;
  }

  code {
    font-size: 0.72rem;
    color: #a8c4e5;
    word-break: break-all;
  }

  .bar-wrap {
    height: 8px;
    border-radius: 999px;
    overflow: hidden;
    border: 1px solid rgba(120, 157, 191, 0.25);
    background: #132640;
  }

  .bar {
    height: 100%;
    background: linear-gradient(90deg, #4cc9ff, #3dd6c6);
  }

  small,
  .muted {
    color: #9fb2cc;
    font-size: 0.72rem;
  }
</style>
