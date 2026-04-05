<script lang="ts">
  import type { FabricEvent } from '../stores/fabric';

  export let events: FabricEvent[] = [];
</script>

<article class="card feed">
  <h2>Live Fabric</h2>
  {#if events.length === 0}
    <p class="muted">No events yet.</p>
  {:else}
    <ul>
      {#each events as event}
        <li>
          <code>{event.topic}</code>
          <span>{event.source}</span>
          <small>{new Date(event.timestamp).toLocaleTimeString()}</small>
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

  .feed ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 240px;
    overflow: auto;
    display: grid;
    gap: 0.35rem;
  }

  .feed li {
    display: grid;
    grid-template-columns: 1.2fr 0.8fr auto;
    gap: 0.5rem;
    font-size: 0.78rem;
    border-bottom: 1px dashed rgba(167, 196, 226, 0.12);
    padding-bottom: 0.3rem;
  }

  .muted { color: var(--muted); }
</style>
