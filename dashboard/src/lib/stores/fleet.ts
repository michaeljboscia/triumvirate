import { derived, writable, type Readable } from 'svelte/store';

export type FleetTaskState = 'pending' | 'claimed' | 'in_progress' | 'done' | 'failed';

export interface FleetTaskCard {
  task_id: string;
  state: FleetTaskState;
  agent?: string;
  worktree?: string;
}

export interface FleetSnapshot {
  fleet_id: string;
  tasks: FleetTaskCard[];
  merge_queue: string[];
}

const snapshot = writable<FleetSnapshot>({
  fleet_id: 'fleet-none',
  tasks: [],
  merge_queue: []
});

let socket: WebSocket | null = null;

function wsUrl(): string {
  if (typeof window === 'undefined') {
    return 'ws://127.0.0.1:8080/ws';
  }
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}/ws`;
}

function normalizeState(raw: unknown): FleetTaskState {
  const value = String(raw ?? '').toLowerCase();
  if (value === 'claimed' || value === 'in_progress' || value === 'done' || value === 'failed') {
    return value;
  }
  return 'pending';
}

function ingest(raw: string): void {
  let parsed: { type?: string; payload?: Record<string, unknown> };
  try {
    parsed = JSON.parse(raw);
  } catch {
    return;
  }
  if (parsed.type !== 'fleet_progress') {
    return;
  }

  const payload = parsed.payload ?? {};
  const tasksRaw = Array.isArray(payload.tasks) ? payload.tasks : [];
  const tasks: FleetTaskCard[] = tasksRaw.map((task) => {
    const input = (task ?? {}) as Record<string, unknown>;
    return {
      task_id: String(input.task_id ?? input.id ?? `task-${Math.random()}`),
      state: normalizeState(input.state),
      agent: input.agent ? String(input.agent) : undefined,
      worktree: input.worktree ? String(input.worktree) : undefined
    };
  });

  const mergeQueueRaw = Array.isArray(payload.merge_queue) ? payload.merge_queue : [];
  const merge_queue = mergeQueueRaw.map((entry) => String(entry));

  snapshot.set({
    fleet_id: String(payload.fleet_id ?? 'fleet-live'),
    tasks,
    merge_queue
  });
}

export function connectFleetStream(): void {
  if (typeof window === 'undefined' || socket) {
    return;
  }
  socket = new WebSocket(wsUrl());
  socket.onmessage = (event) => ingest(String(event.data));
  socket.onclose = () => {
    socket = null;
  };
}

export function disconnectFleetStream(): void {
  if (socket) {
    socket.close();
    socket = null;
  }
}

export const fleetSnapshot: Readable<FleetSnapshot> = derived(snapshot, ($snapshot) => $snapshot);

export const kanbanColumns: Readable<Record<FleetTaskState, FleetTaskCard[]>> = derived(
  snapshot,
  ($snapshot) => {
    const base: Record<FleetTaskState, FleetTaskCard[]> = {
      pending: [],
      claimed: [],
      in_progress: [],
      done: [],
      failed: []
    };
    for (const task of $snapshot.tasks) {
      base[task.state].push(task);
    }
    return base;
  }
);
