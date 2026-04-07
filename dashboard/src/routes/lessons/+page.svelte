<script lang="ts">
  import { onMount } from 'svelte';
  import { lessons, loadLessons, validateLesson } from '$lib/stores/lessons';

  onMount(() => {
    void loadLessons();
  });

  function confidenceClass(value: number): string {
    if (value < 0.3) {
      return 'low';
    }
    if (value < 0.7) {
      return 'mid';
    }
    return 'high';
  }

  function pct(value: number): string {
    const clamped = Math.max(0, Math.min(1, value));
    return `${Math.round(clamped * 100)}%`;
  }
</script>

<svelte:head>
  <title>Lessons | Triumvirate Dashboard</title>
</svelte:head>

<main class="page">
  <header>
    <p class="eyebrow">Knowledge Retention</p>
    <h1>Lessons</h1>
  </header>

  {#if $lessons.length === 0}
    <p class="empty">No lessons available.</p>
  {:else}
    <ul class="lessons">
      {#each $lessons as lesson (lesson.id)}
        <li class:stale={lesson.confidence < 0.1}>
          <div class="row-top">
            <strong>{lesson.title}</strong>
            <button type="button" on:click={() => void validateLesson(lesson.id)}>Validate</button>
          </div>
          <div class="bar-wrap" role="img" aria-label={`confidence ${pct(lesson.confidence)}`}>
            <div
              class={`bar ${confidenceClass(lesson.confidence)}`}
              style={`width: ${pct(lesson.confidence)}`}
            ></div>
          </div>
          <div class="meta">
            <small>Confidence: {pct(lesson.confidence)}</small>
            <small>{lesson.tags ?? 'no tags'}</small>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</main>

<style>
  .page {
    padding: 1.5rem;
    display: grid;
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

  .lessons {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.8rem;
  }

  li {
    border: 1px solid var(--color-border);
    border-radius: 10px;
    background: rgb(26 29 39 / 0.82);
    padding: 0.8rem;
    display: grid;
    gap: 0.55rem;
  }

  li.stale {
    border-color: rgb(239 68 68 / 0.75);
    box-shadow: inset 0 0 0 1px rgb(239 68 68 / 0.25);
  }

  .row-top {
    display: flex;
    justify-content: space-between;
    gap: 0.8rem;
    align-items: center;
  }

  button {
    border-radius: 8px;
    border: 1px solid var(--color-border);
    background: rgb(34 197 94 / 0.15);
    color: var(--color-text);
    cursor: pointer;
    padding: 0.35rem 0.65rem;
    font-size: 0.75rem;
  }

  .bar-wrap {
    width: 100%;
    height: 0.6rem;
    border-radius: 999px;
    background: rgb(113 113 122 / 0.35);
    overflow: hidden;
  }

  .bar {
    height: 100%;
    border-radius: 999px;
  }

  .bar.high {
    background: linear-gradient(90deg, #22c55e, #16a34a);
  }

  .bar.mid {
    background: linear-gradient(90deg, #facc15, #eab308);
  }

  .bar.low {
    background: linear-gradient(90deg, #f87171, #ef4444);
  }

  .meta {
    display: flex;
    justify-content: space-between;
    gap: 0.7rem;
  }

  small,
  .empty {
    color: var(--color-text-muted);
    font-size: 0.78rem;
  }
</style>
