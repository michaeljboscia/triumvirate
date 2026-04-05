import { writable } from 'svelte/store';

export type AgentStatus = 'starting' | 'ready' | 'busy' | 'unresponsive' | 'restarting' | 'dead';

export interface AgentView {
  id: string;
  name: string;
  model: string;
  status: AgentStatus;
}

export const agents = writable<AgentView[]>([]);

export async function refreshAgents(baseUrl = ''): Promise<void> {
  const res = await fetch(`${baseUrl}/api/agents`);
  if (!res.ok) return;
  const data = (await res.json()) as AgentView[];
  agents.set(data);
}
