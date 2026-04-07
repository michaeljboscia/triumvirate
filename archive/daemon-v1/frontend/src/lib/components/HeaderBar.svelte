<script lang="ts">
  import type { QuotaSnapshot } from '../stores/quota';

  export let title = 'Triumvirate v2 Dashboard';
  export let fabricConnected = false;
  export let quota: QuotaSnapshot = { agents: {} };

  const order: Array<'claude' | 'gemini' | 'codex'> = ['claude', 'gemini', 'codex'];
</script>

<header class="hero card">
  <div class="top">
    <h1>{title}</h1>
    <span class={`pill ${fabricConnected ? 'ok' : 'bad'}`}>
      fabric {fabricConnected ? 'connected' : 'disconnected'}
    </span>
  </div>

  <div class="quota-row">
    {#each order as agent}
      {@const q = quota.agents[agent]}
      <div class="quota-item">
        <span class="agent">{agent}</span>
        <div class="bar-wrap">
          <div class="bar" style={`width:${Math.max(0, Math.min(100, q?.utilization_percent ?? 0))}%`}></div>
        </div>
        <small>{Math.round(q?.utilization_percent ?? 0)}%</small>
      </div>
    {/each}
  </div>
</header>

<style>
  .card {
    border: 1px solid rgba(167, 196, 226, 0.2);
    border-radius: 12px;
    padding: 1rem;
    background: rgba(17, 29, 48, 0.75);
  }

  .top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
  }

  h1 {
    margin: 0;
    font-size: 2.1rem;
    letter-spacing: 0.02em;
  }

  .pill {
    border-radius: 999px;
    padding: 0.2rem 0.6rem;
    text-transform: uppercase;
    font-size: 0.72rem;
    letter-spacing: 0.03em;
    border: 1px solid transparent;
  }

  .ok { color: #68e7ac; border-color: #2b9a67; }
  .bad { color: #ff8f8f; border-color: #a74949; }

  .quota-row {
    margin-top: 0.8rem;
    display: grid;
    gap: 0.5rem;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  }

  .quota-item {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: 0.5rem;
    align-items: center;
  }

  .agent {
    text-transform: uppercase;
    color: #bed5f1;
    font-size: 0.75rem;
    letter-spacing: 0.04em;
  }

  .bar-wrap {
    height: 8px;
    background: #132640;
    border-radius: 999px;
    overflow: hidden;
    border: 1px solid rgba(120, 157, 191, 0.25);
  }

  .bar {
    height: 100%;
    background: linear-gradient(90deg, #3dd6c6, #8fd3ff);
  }

  small {
    color: #b8cbe5;
    font-size: 0.72rem;
  }
</style>
