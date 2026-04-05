import { writable } from 'svelte/store';

export interface AgentQuota {
  estimated_tokens: number;
  estimated_context_tokens: number;
  utilization_percent: number;
}

export interface QuotaSnapshot {
  agents: {
    claude?: AgentQuota;
    gemini?: AgentQuota;
    codex?: AgentQuota;
  };
}

export const quota = writable<QuotaSnapshot>({ agents: {} });

export async function refreshQuota(baseUrl = ''): Promise<void> {
  const res = await fetch(`${baseUrl}/api/quota`);
  if (!res.ok) return;
  const data = (await res.json()) as QuotaSnapshot;
  quota.set(data);
}
