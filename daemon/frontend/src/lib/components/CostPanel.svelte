<script lang="ts">
  import type { CostSnapshot } from '../stores/costs';

  export let costs: CostSnapshot;

  const order: Array<'claude' | 'gemini' | 'codex'> = ['claude', 'gemini', 'codex'];

  function usd(value: number): string {
    return `$${value.toFixed(4)}`;
  }
</script>

<article class="card">
  <h2>Cost Attribution</h2>
  <p class="summary">
    total {usd(costs.summary.estimated_total_cost_usd)} · turns {costs.summary.turns_total}
  </p>

  <ul>
    {#each order as agent}
      {@const row = costs.agents[agent]}
      <li>
        <strong>{agent}</strong>
        <small>{row.turns} turns</small>
        <small>{row.input_tokens}/{row.output_tokens} in/out tok</small>
        <span>{usd(row.estimated_cost_usd)}</span>
      </li>
    {/each}
  </ul>
</article>

<style>
  .card {
    border: 1px solid rgba(167, 196, 226, 0.2);
    border-radius: 12px;
    padding: 1rem;
    background: rgba(17, 29, 48, 0.75);
  }

  h2 {
    margin: 0 0 0.55rem;
    font-size: 1rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #b5cae3;
  }

  .summary {
    margin: 0 0 0.65rem;
    color: #b8cbe5;
    font-size: 0.8rem;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.45rem;
  }

  li {
    display: grid;
    grid-template-columns: auto auto 1fr auto;
    gap: 0.5rem;
    align-items: center;
    border-bottom: 1px dashed rgba(167, 196, 226, 0.12);
    padding-bottom: 0.35rem;
  }

  li strong {
    text-transform: uppercase;
    font-size: 0.78rem;
    letter-spacing: 0.04em;
  }

  li small {
    color: #9fb2cc;
    font-size: 0.72rem;
  }

  li span {
    color: #a4f4ec;
    font-size: 0.8rem;
    font-weight: 600;
  }
</style>
