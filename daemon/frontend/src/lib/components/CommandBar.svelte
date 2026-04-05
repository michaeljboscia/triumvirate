<script lang="ts">
  import { createEventDispatcher } from 'svelte';

  const dispatch = createEventDispatcher<{ send: { kind: string; value: string } }>();
  let input = '';

  function submit(kind: string) {
    const value = input.trim();
    if (!value) return;
    dispatch('send', { kind, value });
    input = '';
  }
</script>

<article class="card">
  <h2>Command Bar</h2>
  <div class="row">
    <input bind:value={input} placeholder="Type message, fleet spec, or debate topic" />
    <button on:click={() => submit('message')}>Send</button>
    <button on:click={() => submit('fleet')}>Fleet</button>
    <button on:click={() => submit('debate')}>Debate</button>
  </div>
</article>

<style>
  .card {
    border: 1px solid rgba(167, 196, 226, 0.2);
    border-radius: 12px;
    padding: 1rem;
    background: rgba(17, 29, 48, 0.75);
  }

  .row {
    display: grid;
    gap: 0.5rem;
    grid-template-columns: 1fr auto auto auto;
  }

  input {
    background: #122038;
    color: #e8f1ff;
    border: 1px solid rgba(167, 196, 226, 0.25);
    border-radius: 8px;
    padding: 0.55rem 0.65rem;
  }

  button {
    border: 1px solid rgba(61, 214, 198, 0.55);
    background: rgba(61, 214, 198, 0.12);
    color: #a4f4ec;
    border-radius: 8px;
    padding: 0.52rem 0.68rem;
    cursor: pointer;
  }
</style>
