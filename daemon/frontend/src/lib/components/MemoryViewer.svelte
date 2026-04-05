<script lang="ts">
  import type { DecisionItem } from '../stores/decisions';

  export let decisions: DecisionItem[] = [];

  let selected: DecisionItem | null = null;

  function closeModal() {
    selected = null;
  }

  function closeIfBackdrop(event: MouseEvent) {
    if (event.currentTarget === event.target) closeModal();
  }
</script>

<article class="card">
  <h2>Memory Viewer</h2>
  {#if decisions.length === 0}
    <p class="muted">No captured decisions yet.</p>
  {:else}
    <ul>
      {#each decisions.slice(0, 12) as decision}
        <li>
          <button class="decision-btn" on:click={() => (selected = decision)}>
            <strong>#{decision.id}</strong>
            <span>{decision.decision_text}</span>
            <small>{decision.proposed_by}</small>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</article>

{#if selected}
  <div
    class="modal-backdrop"
    role="button"
    tabindex="0"
    on:click={closeIfBackdrop}
    on:keydown={(event) => {
      if (event.key === 'Escape' || event.key === 'Enter' || event.key === ' ') closeModal();
    }}
  >
    <div
      class="modal"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
    >
      <h3>Decision Confirmation</h3>
      <p>{selected.decision_text}</p>
      <small>proposed by {selected.proposed_by}</small>
      <small>session {selected.session_id}</small>
      <button on:click={closeModal}>Close</button>
    </div>
  </div>
{/if}

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
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.45rem;
  }

  .decision-btn {
    width: 100%;
    text-align: left;
    border-radius: 10px;
    border: 1px solid rgba(167, 196, 226, 0.2);
    background: rgba(10, 20, 38, 0.7);
    color: #e5edf8;
    padding: 0.55rem 0.65rem;
    display: grid;
    gap: 0.2rem;
    cursor: pointer;
  }

  .decision-btn span {
    font-size: 0.84rem;
    color: #c9d9ee;
  }

  .decision-btn small,
  .muted {
    color: #9fb2cc;
    font-size: 0.72rem;
  }

  .modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(3, 8, 18, 0.68);
    display: grid;
    place-items: center;
    padding: 1rem;
  }

  .modal {
    width: min(540px, 100%);
    border: 1px solid rgba(167, 196, 226, 0.25);
    border-radius: 12px;
    background: #111d30;
    padding: 1rem;
    display: grid;
    gap: 0.5rem;
  }

  .modal h3 {
    margin: 0;
  }

  .modal button {
    justify-self: start;
    border: 1px solid rgba(61, 214, 198, 0.55);
    background: rgba(61, 214, 198, 0.12);
    color: #a4f4ec;
    border-radius: 8px;
    padding: 0.48rem 0.68rem;
    cursor: pointer;
  }
</style>
