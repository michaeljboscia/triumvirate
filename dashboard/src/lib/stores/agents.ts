import { derived, get, writable, type Readable } from 'svelte/store';

export type Verbosity = 'minimal' | 'standard' | 'detailed' | 'raw';

export interface StreamEvent {
  type: string;
  ts_ms?: number;
  payload: Record<string, unknown>;
  raw: string;
}

export interface AgentStatus {
  agent: string;
  state: string;
  updatedAt: number;
}

const MAX_EVENTS = 300;

const agentMap = writable<Record<string, AgentStatus>>({
  system: {
    agent: 'system',
    state: 'idle',
    updatedAt: Date.now()
  }
});
const events = writable<StreamEvent[]>([]);
export const verbosity = writable<Verbosity>('standard');

let socket: WebSocket | null = null;

function wsUrl(): string {
  if (typeof window === 'undefined') {
    return 'ws://127.0.0.1:8080/ws';
  }
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}/ws`;
}

function shouldShow(event: StreamEvent, level: Verbosity): boolean {
  if (level === 'raw' || level === 'detailed') {
    return true;
  }
  if (level === 'standard') {
    return event.type === 'agent_state' || event.type === 'review_completed';
  }
  const state = String(event.payload.state ?? '').toLowerCase();
  return event.type === 'agent_state' && ['done', 'failed', 'stuck'].includes(state);
}

function ingest(raw: string): void {
  let parsed: StreamEvent;
  try {
    const json = JSON.parse(raw) as {
      type?: string;
      ts_ms?: number;
      payload?: Record<string, unknown>;
    };
    parsed = {
      type: json.type ?? 'unknown',
      ts_ms: json.ts_ms,
      payload: json.payload ?? {},
      raw
    };
  } catch {
    parsed = { type: 'parse_error', payload: {}, raw };
  }

  if (parsed.type === 'agent_state') {
    const agent = String(parsed.payload.agent ?? 'unknown');
    const state = String(parsed.payload.state ?? 'unknown');
    agentMap.update((current) => ({
      ...current,
      [agent]: {
        agent,
        state,
        updatedAt: parsed.ts_ms ?? Date.now()
      }
    }));
  }

  events.update((current) => [parsed, ...current].slice(0, MAX_EVENTS));
}

export function connectAgentStream(): void {
  if (typeof window === 'undefined' || socket) {
    return;
  }

  socket = new WebSocket(wsUrl());
  socket.onmessage = (message) => ingest(String(message.data));
  socket.onerror = () => {
    events.update((current) => [
      {
        type: 'connection_error',
        payload: { state: 'stuck', detail: 'websocket error' },
        raw: 'websocket error'
      },
      ...current
    ].slice(0, MAX_EVENTS));
  };
  socket.onclose = () => {
    socket = null;
  };
}

export function disconnectAgentStream(): void {
  if (socket) {
    socket.close();
    socket = null;
  }
}

export const agents: Readable<AgentStatus[]> = derived(agentMap, ($agentMap) =>
  Object.values($agentMap).sort((a, b) => a.agent.localeCompare(b.agent))
);

export const filteredEvents: Readable<StreamEvent[]> = derived(
  [events, verbosity],
  ([$events, $verbosity]) => $events.filter((event) => shouldShow(event, $verbosity))
);

export function currentVerbosity(): Verbosity {
  return get(verbosity);
}
