<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import {
    approvalRateByAgent,
    connectReviewStream,
    disconnectReviewStream,
    pendingReviews,
    reviewHistory
  } from '$lib/stores/reviews';

  onMount(() => {
    connectReviewStream();
  });

  onDestroy(() => {
    disconnectReviewStream();
  });

  function ageLabel(requestedAtMs: number): string {
    const elapsed = Math.max(0, Date.now() - requestedAtMs);
    const seconds = Math.floor(elapsed / 1000);
    if (seconds < 60) {
      return `${seconds}s`;
    }
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) {
      return `${minutes}m`;
    }
    return `${Math.floor(minutes / 60)}h`;
  }

  function pct(value: number): string {
    return `${Math.round(value * 100)}%`;
  }
</script>

<svelte:head>
  <title>Reviews | Triumvirate Dashboard</title>
</svelte:head>

<main class="page">
  <header>
    <p class="eyebrow">Quality Gate</p>
    <h1>Reviews</h1>
  </header>

  <section class="rates">
    <h2>Approval Rate By Agent</h2>
    {#if Object.keys($approvalRateByAgent).length === 0}
      <p class="empty">No completed reviews yet.</p>
    {:else}
      <ul>
        {#each Object.entries($approvalRateByAgent) as [agent, rate]}
          <li><strong>{agent}</strong><span>{pct(rate)}</span></li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="split">
    <article>
      <h2>Pending Reviews</h2>
      {#if $pendingReviews.length === 0}
        <p class="empty">No pending reviews.</p>
      {:else}
        <ul>
          {#each $pendingReviews as review (review.review_id)}
            <li>
              <strong>{review.review_id}</strong>
              <small>{review.reviewer_agent}</small>
              <small>age: {ageLabel(review.requested_at_ms)}</small>
            </li>
          {/each}
        </ul>
      {/if}
    </article>

    <article>
      <h2>History</h2>
      {#if $reviewHistory.length === 0}
        <p class="empty">No history yet.</p>
      {:else}
        <ul>
          {#each $reviewHistory as review (review.review_id)}
            <li>
              <strong>{review.review_id}</strong>
              <small>{review.reviewer_agent}</small>
              <small class:approved={review.verdict === 'approve'} class:changes={review.verdict === 'request_changes'}>
                {review.verdict}
              </small>
            </li>
          {/each}
        </ul>
      {/if}
    </article>
  </section>
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

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  .rates,
  article {
    border: 1px solid var(--color-border);
    border-radius: 10px;
    background: rgb(26 29 39 / 0.82);
    padding: 0.8rem;
  }

  .split {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 0.75rem;
  }

  ul {
    margin: 0.7rem 0 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 0.5rem;
  }

  li {
    border: 1px solid var(--color-border);
    border-radius: 8px;
    padding: 0.55rem;
    background: rgb(15 17 23 / 0.76);
    display: flex;
    justify-content: space-between;
    gap: 0.5rem;
    align-items: center;
  }

  small,
  .empty {
    color: var(--color-text-muted);
    font-size: 0.78rem;
  }

  .approved {
    color: #22c55e;
  }

  .changes {
    color: #ef4444;
  }
</style>
