<script lang="ts">
  import { onMount } from 'svelte';
  import AgentGrid from '../lib/components/AgentGrid.svelte';
  import JsonPanel from '../lib/components/JsonPanel.svelte';
  import EventFeed from '../lib/components/EventFeed.svelte';
  import CommandBar from '../lib/components/CommandBar.svelte';
  import { agents, refreshAgents } from '../lib/stores/agents';
  import { quota, refreshQuota } from '../lib/stores/quota';
  import { tasks, refreshTasks } from '../lib/stores/tasks';
  import { workflows, refreshWorkflows } from '../lib/stores/workflow';
  import { connectFabric, disconnectFabric, fabricConnected, fabricEvents } from '../lib/stores/fabric';
  import { sendMessage, spawnFleet, startDebate } from '../lib/stores/commands';

  const title = 'Triumvirate v2 Dashboard';
  let timer: ReturnType<typeof setInterval> | undefined;

  async function refreshAll() {
    await Promise.all([refreshAgents(), refreshTasks(), refreshQuota(), refreshWorkflows()]);
  }

  async function handleCommand(event: CustomEvent<{ kind: string; value: string }>) {
    const { kind, value } = event.detail;
    if (kind === 'fleet') {
      await spawnFleet(value);
    } else if (kind === 'debate') {
      await startDebate(value);
    } else {
      await sendMessage(value);
    }
    await refreshAll();
  }

  onMount(() => {
    connectFabric();
    void refreshAll();
    timer = setInterval(() => {
      void refreshAll();
    }, 3000);

    return () => {
      if (timer) clearInterval(timer);
      disconnectFabric();
    };
  });
</script>

<main class="page">
  <header class="hero">
    <h1>{title}</h1>
    <p>Live orchestration shell • fabric {$fabricConnected ? 'connected' : 'disconnected'}</p>
  </header>

  <CommandBar on:send={handleCommand} />

  <section class="grid two-up">
    <AgentGrid agents={$agents} />
    <EventFeed events={$fabricEvents} />
    <JsonPanel title="Quota" data={$quota} />
    <JsonPanel title="Fleet Tasks" data={$tasks.slice(0, 12)} />
    <JsonPanel title="Workflows" data={$workflows} />
  </section>
</main>

<style>
  .page {
    max-width: 1080px;
    margin: 2.5rem auto;
    padding: 0 1rem 2rem;
    display: grid;
    gap: 1rem;
  }

  .hero h1 {
    margin: 0;
    font-size: 2.2rem;
    letter-spacing: 0.02em;
  }

  .hero p {
    color: var(--muted);
    margin: 0.3rem 0 0;
  }

  .grid {
    display: grid;
    gap: 1rem;
  }

  .two-up {
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  }
</style>
