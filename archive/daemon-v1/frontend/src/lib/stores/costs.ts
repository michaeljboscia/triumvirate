import { writable } from 'svelte/store';

export interface AgentCost {
  turns: number;
  input_tokens: number;
  output_tokens: number;
  estimated_cost_usd: number;
}

export interface CostSnapshot {
  summary: {
    estimated_total_cost_usd: number;
    turns_total: number;
  };
  agents: {
    claude: AgentCost;
    gemini: AgentCost;
    codex: AgentCost;
  };
}

export const costs = writable<CostSnapshot>({
  summary: {
    estimated_total_cost_usd: 0,
    turns_total: 0,
  },
  agents: {
    claude: { turns: 0, input_tokens: 0, output_tokens: 0, estimated_cost_usd: 0 },
    gemini: { turns: 0, input_tokens: 0, output_tokens: 0, estimated_cost_usd: 0 },
    codex: { turns: 0, input_tokens: 0, output_tokens: 0, estimated_cost_usd: 0 },
  },
});

export async function refreshCosts(baseUrl = ''): Promise<void> {
  const res = await fetch(`${baseUrl}/api/costs`);
  if (!res.ok) return;
  const data = (await res.json()) as CostSnapshot;
  costs.set(data);
}
