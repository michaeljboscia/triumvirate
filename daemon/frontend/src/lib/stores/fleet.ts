import { writable } from 'svelte/store';

export interface FleetSummary {
  task_total: number;
  task_completed: number;
  task_failed: number;
  worktree_total: number;
}

export interface FleetStatusTask {
  task_key: string;
  title: string;
  status: string;
  assigned_agent: string | null;
  depends_on: string | null;
  updated_at: string;
}

export interface FleetStatusWorktree {
  member_key: string;
  agent_type: string;
  branch_name: string;
  worktree_path: string;
  status: string;
  updated_at: string;
}

export interface FleetStatus {
  fleet_id: string;
  summary: FleetSummary;
  tasks: FleetStatusTask[];
  worktrees: FleetStatusWorktree[];
}

export interface FleetMergeResult {
  fleet_id: string;
  repo_root: string;
  merged_branches: string[];
  failed_branch: string | null;
  conflict: boolean;
}

export const fleetStatus = writable<FleetStatus | null>(null);
export const mergeResult = writable<FleetMergeResult | null>(null);
export const mergeError = writable<string | null>(null);

export async function refreshFleetStatus(fleetId: string, baseUrl = ''): Promise<void> {
  if (!fleetId.trim()) {
    fleetStatus.set(null);
    return;
  }
  const res = await fetch(`${baseUrl}/api/fleet/status/${encodeURIComponent(fleetId)}`);
  if (!res.ok) return;
  const data = (await res.json()) as FleetStatus;
  fleetStatus.set(data);
}

export async function runFleetMerge(
  fleetId: string,
  humanApproved: boolean,
  baseUrl = '',
): Promise<void> {
  mergeError.set(null);
  if (!fleetId.trim()) return;

  const res = await fetch(`${baseUrl}/api/fleet/merge`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      fleet_id: fleetId,
      human_approved: humanApproved,
    }),
  });

  if (!res.ok) {
    const payload = (await res.json().catch(() => ({}))) as { error?: string };
    mergeError.set(payload.error ?? `merge failed with status ${res.status}`);
    return;
  }

  const data = (await res.json()) as FleetMergeResult;
  mergeResult.set(data);
}
