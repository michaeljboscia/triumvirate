import { writable } from 'svelte/store';

export interface DecisionItem {
  id: number;
  session_id: string;
  decision_text: string;
  proposed_by: string;
  validated_by: string | null;
  created_at: string;
  evidence: string | null;
}

export const decisions = writable<DecisionItem[]>([]);

export async function refreshDecisions(baseUrl = ''): Promise<void> {
  const res = await fetch(`${baseUrl}/api/decisions`);
  if (!res.ok) return;
  const data = (await res.json()) as { decisions: DecisionItem[] };
  decisions.set(data.decisions ?? []);
}
