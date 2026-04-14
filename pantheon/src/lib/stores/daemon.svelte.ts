// T-020 (REQ-019, REQ-020, REQ-021) — Svelte 5 store for daemon state.
//
// Subscribes to the Tauri events emitted by `src-tauri/src/daemon_client.rs`
// and exposes the data as Svelte 5 runes ($state) so components can consume
// the store without re-subscribing per-component. One subscription, many
// readers — the Rust side runs a single WebSocket + REST poll loop and
// fans events out through Tauri's IPC bus.
//
// Why a runes-based class instead of a writable store:
// Svelte 5 runes replace the Svelte 4 writable() pattern. Classes with
// $state fields give us a stable singleton, typed access, and explicit
// initialize/destroy — matching how the Rust daemon_client lifecycle
// behaves. Writable stores would also work but runes are the Svelte 5
// idiom and play cleaner with $derived reads in components.
//
// Event contract (must match daemon_client.rs EVENT_* constants):
//   "daemon://state"   → HealthState string ("starting"|"ready"|"degraded"|"disconnected")
//   "daemon://workers" → WorkersResponse  { workers: WorkerInfo[] }
//   "daemon://fleet"   → FleetResponse    { builds: FleetBuild[] }
//   "daemon://stream"  → { type: string, payload: any }  — live agent_stream + friends
//
// Usage from a component:
//   <script lang="ts">
//     import { daemon } from "$lib/stores/daemon";
//     // In +layout.svelte, call daemon.init() once on mount
//   </script>
//   <p>State: {daemon.state}</p>
//   <p>{daemon.workers.length} workers</p>

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type HealthState = "starting" | "ready" | "degraded" | "disconnected";

export interface WorkerInfo {
	session_id: string;
	agent: string;
	name: string;
	status: string;
	task_id?: string;
	parent_session_id?: string;
	root_session_id?: string;
	pantheon_session_id?: string;
	cwd?: string;
	started_at: string;
	elapsed_ms: number;
}

export interface FleetTask {
	task_id: string;
	status: string;
	files: string[];
	worker_session_id?: string;
	elapsed_ms: number;
	commit_sha?: string;
}

export interface FleetBuild {
	build_id: string;
	task_count: number;
	completed: number;
	failed: number;
	in_progress: number;
	queued: number;
	tasks: FleetTask[];
}

export interface StreamEvent {
	type: string;
	// Payload is intentionally loose — the Rust side forwards whatever the
	// daemon publishes and the consuming component narrows on `type`.
	payload: unknown;
}

/**
 * Singleton daemon store. Use the exported `daemon` instance below;
 * never instantiate directly. Call `.init()` once at app mount and
 * `.destroy()` on unmount to release the Tauri event listeners.
 *
 * Svelte 5 rune class fields are reactive — any template or $effect
 * that reads `daemon.state` re-runs when the underlying $state changes.
 */
class DaemonStore {
	state = $state<HealthState>("starting");
	workers = $state<WorkerInfo[]>([]);
	fleet = $state<FleetBuild[]>([]);

	/**
	 * T-020 fix: rolling buffer of stream events is DELIBERATELY NOT a
	 * $state rune. Every incoming ws event was reassigning a new array
	 * reference, triggering a full reactive re-render across the UI and
	 * producing visible flashing at even modest event rates. Components
	 * that want a snapshot of recent events should call
	 * `daemon.recentEventsSnapshot()` on demand (e.g. from an activity
	 * panel's own scheduled refresh), which returns a copy without
	 * entangling Svelte reactivity. Wave 6 activity panels can adopt
	 * their own local $state mirror with a throttle if they need live
	 * updates.
	 */
	private streamBuffer: StreamEvent[] = [];
	private static readonly STREAM_BUFFER_CAP = 500;

	/**
	 * Return a point-in-time copy of the stream buffer. Safe to read
	 * without triggering reactivity. Consumers that want live updates
	 * should poll this on their own cadence (setInterval) or use a
	 * manual $state mirror gated by a throttle.
	 */
	recentEventsSnapshot(): StreamEvent[] {
		return this.streamBuffer.slice();
	}

	private unlistens: UnlistenFn[] = [];
	private initialized = false;

	async init(): Promise<void> {
		if (this.initialized) return;
		this.initialized = true;

		// Four parallel subscriptions — the Rust side dispatches to all of
		// them from the same connection, so there's no concern about ordering.
		// Promise.all lets us fail fast if any listener registration errors.
		const [stateUnlisten, workersUnlisten, fleetUnlisten, streamUnlisten] =
			await Promise.all([
				listen<HealthState>("daemon://state", (event) => {
					this.state = event.payload;
				}),
				listen<{ workers: WorkerInfo[] }>("daemon://workers", (event) => {
					this.workers = event.payload.workers;
				}),
				listen<{ builds: FleetBuild[] }>("daemon://fleet", (event) => {
					this.fleet = event.payload.builds;
				}),
				listen<StreamEvent>("daemon://stream", (event) => {
					// T-020 fix: mutate the plain buffer in place — no
					// reactivity. Push newest to the front, trim from the
					// back. Components that care about stream events should
					// call `recentEventsSnapshot()` on their own cadence.
					this.streamBuffer.unshift(event.payload);
					if (this.streamBuffer.length > DaemonStore.STREAM_BUFFER_CAP) {
						this.streamBuffer.length = DaemonStore.STREAM_BUFFER_CAP;
					}
				}),
			]);

		this.unlistens.push(
			stateUnlisten,
			workersUnlisten,
			fleetUnlisten,
			streamUnlisten,
		);
	}

	destroy(): void {
		for (const unlisten of this.unlistens) unlisten();
		this.unlistens = [];
		this.initialized = false;
	}
}

/**
 * App-wide singleton. Import and read fields directly — Svelte 5 runes
 * make the reads reactive in any template or $effect.
 */
export const daemon = new DaemonStore();
