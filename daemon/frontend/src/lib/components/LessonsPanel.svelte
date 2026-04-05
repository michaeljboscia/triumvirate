<script lang="ts">
  import { createEventDispatcher } from 'svelte';
  import type { LessonRecord } from '../stores/lessons';

  export let lessons: LessonRecord[] = [];

  const dispatch = createEventDispatcher<{
    refilter: { outcome: string; agent_source: string; pattern: string; min_confidence: number };
  }>();

  let outcome = '';
  let agent_source = '';
  let pattern = '';
  let min_confidence = 0.0;

  function applyFilters() {
    dispatch('refilter', { outcome, agent_source, pattern, min_confidence });
  }
</script>

<article class="card">
  <h2>Lessons Ledger</h2>
  <div class="filters">
    <select bind:value={outcome} on:change={applyFilters}>
      <option value="">all outcomes</option>
      <option value="success">success</option>
      <option value="failure">failure</option>
      <option value="partial">partial</option>
    </select>
    <select bind:value={agent_source} on:change={applyFilters}>
      <option value="">all agents</option>
      <option value="claude">claude</option>
      <option value="gemini">gemini</option>
      <option value="codex">codex</option>
      <option value="human">human</option>
      <option value="system">system</option>
    </select>
    <input bind:value={pattern} on:change={applyFilters} placeholder="pattern contains..." />
    <label>
      min confidence
      <input
        type="range"
        min="0"
        max="1"
        step="0.05"
        bind:value={min_confidence}
        on:change={applyFilters}
      />
      <small>{min_confidence.toFixed(2)}</small>
    </label>
  </div>

  {#if lessons.length === 0}
    <p class="muted">No lessons match current filters.</p>
  {:else}
    <ul>
      {#each lessons.slice(0, 20) as lesson}
        <li>
          <span class={`badge ${lesson.outcome}`}>{lesson.outcome}</span>
          <strong>{lesson.decision}</strong>
          <small>{lesson.rationale}</small>
          <small>
            pattern={lesson.pattern} agent={lesson.agent_source} conf={lesson.confidence_score.toFixed(2)} ->
            {lesson.effective_confidence.toFixed(2)}
          </small>
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
    margin: 0 0 0.6rem;
    font-size: 1rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #b5cae3;
  }

  .filters {
    display: grid;
    gap: 0.5rem;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    margin-bottom: 0.75rem;
  }

  select,
  input {
    background: #122038;
    color: #e8f1ff;
    border: 1px solid rgba(167, 196, 226, 0.25);
    border-radius: 8px;
    padding: 0.45rem 0.55rem;
  }

  label {
    display: grid;
    gap: 0.2rem;
    font-size: 0.74rem;
    color: #9fb2cc;
  }

  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.5rem;
  }

  li {
    display: grid;
    gap: 0.2rem;
    border: 1px solid rgba(167, 196, 226, 0.15);
    border-radius: 10px;
    padding: 0.5rem 0.6rem;
    background: rgba(10, 20, 38, 0.7);
  }

  .badge {
    width: fit-content;
    font-size: 0.68rem;
    text-transform: uppercase;
    border-radius: 999px;
    padding: 0.1rem 0.4rem;
    border: 1px solid transparent;
  }

  .badge.success {
    color: #68e7ac;
    border-color: #2b9a67;
  }
  .badge.failure {
    color: #ff8f8f;
    border-color: #a74949;
  }
  .badge.partial {
    color: #ffd978;
    border-color: #c39a1c;
  }

  small,
  .muted {
    color: #9fb2cc;
    font-size: 0.72rem;
  }
</style>
