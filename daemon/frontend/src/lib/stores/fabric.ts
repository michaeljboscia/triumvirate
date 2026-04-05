import { writable } from "svelte/store";

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
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let baseUrlCached = "";

export function pushFabricEvent(event: FabricEvent): void {
	fabricEvents.update((current) => [event, ...current].slice(0, 300));
}

export function connectFabric(baseUrl = ""): void {
	baseUrlCached = baseUrl;
	if (socket && socket.readyState <= WebSocket.OPEN) return;

	const wsUrl =
		`${baseUrl || window.location.origin}`.replace(/^http/, "ws") + "/ws";
	socket = new WebSocket(wsUrl);

	socket.onopen = () => {
		fabricConnected.set(true);
		if (reconnectTimer) {
			clearTimeout(reconnectTimer);
			reconnectTimer = null;
		}
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
		// Auto-reconnect after 2 seconds
		reconnectTimer = setTimeout(() => connectFabric(baseUrlCached), 2000);
	};

	socket.onerror = () => {
		fabricConnected.set(false);
	};
}

export function disconnectFabric(): void {
	if (reconnectTimer) {
		clearTimeout(reconnectTimer);
		reconnectTimer = null;
	}
	if (!socket) return;
	socket.close();
	socket = null;
	fabricConnected.set(false);
}
