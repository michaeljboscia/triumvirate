import { writable, type Readable } from 'svelte/store';

export interface LedgerHealth {
  status: 'healthy' | 'degraded' | 'dead' | 'unknown';
  queue_depth: number;
  spool_size_bytes: number;
  events_last_5min: number;
}

export interface LedgerSummary {
  session_id: string;
  title: string;
  narrative: string;
  summary_type: string;
  created_at?: string;
}

const healthState = writable<LedgerHealth>({
  status: 'unknown',
  queue_depth: 0,
  spool_size_bytes: 0,
  events_last_5min: 0
});
const searchResults = writable<LedgerSummary[]>([]);
const queryText = writable('');

let socket: WebSocket | null = null;
let pollTimer: number | null = null;

function wsUrl(): string {
  if (typeof window === 'undefined') {
    return 'ws://127.0.0.1:8080/ws';
  }
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}/ws`;
}

function authHeaders(): HeadersInit {
  if (typeof window === 'undefined') {
    return {};
  }
  const token = window.localStorage.getItem('triumvirate_daemon_token');
  if (!token) {
    return { 'Content-Type': 'application/json' };
  }
  return {
    'Content-Type': 'application/json',
    Authorization: `Bearer ${token}`
  };
}

function ingest(raw: string): void {
  let parsed: { type?: string; payload?: Record<string, unknown> };
  try {
    parsed = JSON.parse(raw);
  } catch {
    return;
  }

  if (parsed.type === 'ledger_health') {
    const payload = parsed.payload ?? {};
    healthState.set({
      status: String(payload.status ?? 'unknown') as LedgerHealth['status'],
      queue_depth: Number(payload.queue_depth ?? 0),
      spool_size_bytes: Number(payload.spool_size_bytes ?? 0),
      events_last_5min: Number(payload.events_last_5min ?? 0)
    });
  }
}

export async function searchLedger(query: string): Promise<void> {
  queryText.set(query);
  if (!query.trim()) {
    searchResults.set([]);
    return;
  }

  try {
    const res = await fetch('/ledger/query', {
      method: 'POST',
      headers: authHeaders(),
      body: JSON.stringify({ query, limit: 20 })
    });
    if (!res.ok) {
      searchResults.set([]);
      return;
    }
    const data = (await res.json()) as { summaries?: LedgerSummary[] };
    searchResults.set(data.summaries ?? []);
  } catch {
    searchResults.set([]);
  }
}

export async function refreshLedgerHealth(): Promise<void> {
  try {
    const res = await fetch('/ledger/health', {
      method: 'GET',
      headers: authHeaders()
    });
    if (!res.ok) {
      return;
    }
    const data = (await res.json()) as Partial<LedgerHealth>;
    healthState.set({
      status: (data.status as LedgerHealth['status']) ?? 'unknown',
      queue_depth: Number(data.queue_depth ?? 0),
      spool_size_bytes: Number(data.spool_size_bytes ?? 0),
      events_last_5min: Number(data.events_last_5min ?? 0)
    });
  } catch {
    // keep latest state if fetch fails
  }
}

export function connectLedgerStream(): void {
  if (typeof window === 'undefined') {
    return;
  }

  if (!socket) {
    socket = new WebSocket(wsUrl());
    socket.onmessage = (event) => ingest(String(event.data));
    socket.onclose = () => {
      socket = null;
    };
  }

  void refreshLedgerHealth();
  if (pollTimer === null) {
    pollTimer = window.setInterval(() => {
      void refreshLedgerHealth();
    }, 15000);
  }
}

export function disconnectLedgerStream(): void {
  if (socket) {
    socket.close();
    socket = null;
  }
  if (pollTimer !== null && typeof window !== 'undefined') {
    window.clearInterval(pollTimer);
    pollTimer = null;
  }
}

export const ledgerHealth: Readable<LedgerHealth> = healthState;
export const ledgerResults: Readable<LedgerSummary[]> = searchResults;
export const ledgerQuery: Readable<string> = queryText;
