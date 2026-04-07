<svelte:head>
  <title>Triumvirate Dashboard</title>
</svelte:head>

<main class="wrapper">
  <section class="card">
    <p class="eyebrow">Triumvirate v2.2</p>
    <h1>Dashboard Scaffolded</h1>
    <p>
      Svelte 5 + Vite 6 + Tailwind 4 are wired with static output to <code>dist/</code>.
    </p>
  </section>
</main>

<style>
  .wrapper {
    min-height: 100vh;
    display: grid;
    place-items: center;
    padding: 2rem;
  }

  .card {
    width: min(42rem, 100%);
    border: 1px solid var(--color-border);
    background: color-mix(in oklab, var(--color-surface) 88%, black 12%);
    border-radius: 12px;
    padding: 2rem;
    box-shadow: 0 8px 30px rgb(0 0 0 / 0.25);
  }

  .eyebrow {
    margin: 0 0 0.5rem;
    color: var(--color-text-muted);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-size: 0.75rem;
  }

  h1 {
    margin: 0;
    font-size: clamp(1.8rem, 4vw, 2.4rem);
  }

  p {
    margin: 1rem 0 0;
    line-height: 1.6;
  }

  code {
    font-family: 'JetBrains Mono', 'Fira Code', monospace;
    font-size: 0.95em;
  }
</style>
