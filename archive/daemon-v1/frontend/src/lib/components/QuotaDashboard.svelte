<script lang="ts">
  import type { QuotaSnapshot } from '../stores/quota';

  export let quota: QuotaSnapshot = { agents: {} };

  const order: Array<'claude' | 'gemini' | 'codex'> = ['claude', 'gemini', 'codex'];

  function pct(agent: 'claude' | 'gemini' | 'codex'): number {
    return Math.max(0, Math.min(100, quota.agents[agent]?.utilization_percent ?? 0));
  }

  function stateLabel(value: number): string {
    if (value >= 90) return 'near limit';
    if (value >= 70) return 'elevated';
    return 'healthy';
  }
</script>

<article class="card">
  <h2>Quota Dashboard</h2>
  <div class="rows">
    {#each order as agent}
      {@const q = quota.agents[agent]}
      {@const p = pct(agent)}
      <div class="row">
        <div class="meta">
          <strong>{agent}</strong>
          <small>{stateLabel(p)}</small>
        </div>
        <div class="bar-wrap">
          <div class="bar" style={`width:${p}%`}></div>
        </div>
        <div class="nums">
          <small>{q?.estimated_tokens ?? 0} tok</small>
          <small>{q?.estimated_context_tokens ?? 0} ctx</small>
        </div>
      </div>
    {/each}
  </div>
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

  .rows {
    display: grid;
    gap: 0.6rem;
  }

  .row {
    display: grid;
    gap: 0.5rem;
    grid-template-columns: minmax(95px, auto) 1fr auto;
    align-items: center;
  }

  .meta {
    display: grid;
  }

  .meta strong {
    text-transform: uppercase;
    font-size: 0.78rem;
    letter-spacing: 0.04em;
  }

  .meta small {
    color: #9fb2cc;
    font-size: 0.72rem;
  }

  .bar-wrap {
    height: 10px;
    border-radius: 999px;
    overflow: hidden;
    border: 1px solid rgba(120, 157, 191, 0.25);
    background: #132640;
  }

  .bar {
    height: 100%;
    background: linear-gradient(90deg, #3dd6c6, #8fd3ff);
  }

  .nums {
    display: grid;
    text-align: right;
    color: #b8cbe5;
    font-size: 0.72rem;
  }
</style>
