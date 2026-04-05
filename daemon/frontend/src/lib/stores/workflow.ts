import { writable } from 'svelte/store';

export interface WorkflowSummary {
  workflow_id: string;
  workflow_type: string;
  state: string;
  current_step: number;
}

export const workflows = writable<WorkflowSummary[]>([]);

export async function refreshWorkflows(baseUrl = ''): Promise<void> {
  const res = await fetch(`${baseUrl}/api/workflows`);
  if (!res.ok) return;
  const data = (await res.json()) as { workflows: WorkflowSummary[] };
  workflows.set(data.workflows ?? []);
}
