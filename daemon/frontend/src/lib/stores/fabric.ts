import { writable } from 'svelte/store';

export interface FabricEvent {
  id: string;
  topic: string;
  source: string;
  payload: unknown;
  timestamp: string;
}

export const fabricEvents = writable<FabricEvent[]>([]);
export const fabricConnected = writable(false);

let socket: WebSocket | null = null;

export function pushFabricEvent(event: FabricEvent): void {
  fabricEvents.update((current) => [event, ...current].slice(0, 300));
}

export function connectFabric(baseUrl = ''): void {
  if (socket) return;

  const wsUrl = `${baseUrl || window.location.origin}`.replace(/^http/, 'ws') + '/ws';
  socket = new WebSocket(wsUrl);

  socket.onopen = () => {
    fabricConnected.set(true);
  };

  socket.onmessage = (evt) => {
    try {
      const parsed = JSON.parse(evt.data) as FabricEvent;
      pushFabricEvent(parsed);
    } catch {
      // Ignore malformed frames.
    }
  };

  socket.onclose = () => {
    fabricConnected.set(false);
    socket = null;
  };

  socket.onerror = () => {
    fabricConnected.set(false);
  };
}

export function disconnectFabric(): void {
  if (!socket) return;
  socket.close();
  socket = null;
  fabricConnected.set(false);
}
