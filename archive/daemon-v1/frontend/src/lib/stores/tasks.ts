import { writable } from 'svelte/store';

export interface FleetTask {
  fleet_id: string;
  task_key: string;
  title: string;
  status: string;
  assigned_agent: string | null;
  depends_on: string | null;
  updated_at: string;
}

export const tasks = writable<FleetTask[]>([]);

export async function refreshTasks(baseUrl = ''): Promise<void> {
  const res = await fetch(`${baseUrl}/api/fleet/tasks`);
  if (!res.ok) return;
  const data = (await res.json()) as { tasks: FleetTask[] };
  tasks.set(data.tasks ?? []);
}
