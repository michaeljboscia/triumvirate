import { writable } from 'svelte/store';

export interface FabricEvent {
  id: string;
  topic: string;
  source: string;
  payload: unknown;
  timestamp: string;
}

export const fabricEvents = writable<FabricEvent[]>([]);

export function pushFabricEvent(event: FabricEvent): void {
  fabricEvents.update((current) => [event, ...current].slice(0, 200));
}
