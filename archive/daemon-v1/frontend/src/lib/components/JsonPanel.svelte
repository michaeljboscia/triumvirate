<script lang="ts">
  export let title = '';
  export let data: unknown;
</script>

<article class="card">
  <h2>{title}</h2>
  <pre>{JSON.stringify(data, null, 2)}</pre>
</article>

<style>
  .card {
    border: 1px solid rgba(167, 196, 226, 0.2);
    border-radius: 12px;
    padding: 1rem;
    background: rgba(17, 29, 48, 0.75);
    backdrop-filter: blur(2px);
  }

  .card h2 {
    margin: 0 0 0.75rem;
    font-size: 1rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #b5cae3;
  }

  pre {
    margin: 0;
    font-size: 0.8rem;
    line-height: 1.4;
    color: #b9d7fb;
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
