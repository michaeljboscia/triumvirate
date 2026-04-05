<script lang="ts">
  import { onMount } from 'svelte';
  import HeaderBar from '../lib/components/HeaderBar.svelte';
  import AgentGrid from '../lib/components/AgentGrid.svelte';
  import JsonPanel from '../lib/components/JsonPanel.svelte';
  import EventFeed from '../lib/components/EventFeed.svelte';
  import CommandBar from '../lib/components/CommandBar.svelte';
  import QuotaDashboard from '../lib/components/QuotaDashboard.svelte';
  import MemoryViewer from '../lib/components/MemoryViewer.svelte';
  import WorkflowPanel from '../lib/components/WorkflowPanel.svelte';
  import MergeResolver from '../lib/components/MergeResolver.svelte';
  import { agents, refreshAgents } from '../lib/stores/agents';
  import { quota, refreshQuota } from '../lib/stores/quota';
  import { tasks, refreshTasks } from '../lib/stores/tasks';
  import { workflows, refreshWorkflows } from '../lib/stores/workflow';
  import { decisions, refreshDecisions } from '../lib/stores/decisions';
  import {
    fleetStatus,
    mergeError,
    mergeResult,
    refreshFleetStatus,
    runFleetMerge,
  } from '../lib/stores/fleet';
  import { connectFabric, disconnectFabric, fabricConnected, fabricEvents } from '../lib/stores/fabric';
  import { sendMessage, spawnFleet, startDebate } from '../lib/stores/commands';

  const title = 'Triumvirate v2 Dashboard';
  let timer: ReturnType<typeof setInterval> | undefined;
  let activeFleetId = '';
  let mergeBusy = false;

  async function refreshAll() {
    await Promise.all([
      refreshAgents(),
      refreshTasks(),
      refreshQuota(),
      refreshWorkflows(),
      refreshDecisions(),
    ]);
    if (!activeFleetId && $tasks.length > 0) {
      activeFleetId = $tasks[0].fleet_id;
    }
    if (activeFleetId) {
      await refreshFleetStatus(activeFleetId);
    }
  }

  async function handleCommand(event: CustomEvent<{ kind: string; value: string }>) {
    const { kind, value } = event.detail;
    if (kind === 'fleet') {
      const result = await spawnFleet(value);
      if (result?.fleet_id) activeFleetId = result.fleet_id;
    } else if (kind === 'debate') {
      await startDebate(value);
    } else {
      await sendMessage(value);
    }
    await refreshAll();
  }

  async function handleMerge(event: CustomEvent<{ fleetId: string; humanApproved: boolean }>) {
    const fleetId = event.detail.fleetId.trim();
    if (!fleetId) return;
    mergeBusy = true;
    await runFleetMerge(fleetId, event.detail.humanApproved);
    await refreshFleetStatus(fleetId);
    await refreshTasks();
    mergeBusy = false;
  }

  async function handleInspect(event: CustomEvent<{ fleetId: string }>) {
    const fleetId = event.detail.fleetId.trim();
    if (!fleetId) return;
    activeFleetId = fleetId;
    await refreshFleetStatus(fleetId);
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
  <HeaderBar title={title} fabricConnected={$fabricConnected} quota={$quota} />

  <CommandBar on:send={handleCommand} />

  <section class="grid two-up">
    <AgentGrid agents={$agents} />
    <EventFeed events={$fabricEvents} />
    <QuotaDashboard quota={$quota} />
    <WorkflowPanel workflows={$workflows} />
    <MemoryViewer decisions={$decisions} />
    <MergeResolver
      fleetId={activeFleetId}
      fleetStatus={$fleetStatus}
      mergeResult={$mergeResult}
      mergeError={$mergeError}
      busy={mergeBusy}
      on:merge={handleMerge}
      on:inspect={handleInspect}
    />
    <JsonPanel title="Fleet Tasks" data={$tasks.slice(0, 12)} />
    <JsonPanel title="Fleet Status" data={$fleetStatus} />
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

  .grid {
    display: grid;
    gap: 1rem;
  }

  .two-up {
    grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  }
</style>
