/**
 * Pythia Oracle Engine — Type Definitions (FEAT-016)
 *
 * All types, interfaces, constants, and error codes for the oracle engine.
 * Sourced from the design doc (v6, decisions #1-51) and BACKEND_STRUCTURE.md.
 *
 * This file has NO runtime dependencies — it exports only types and constants.
 * No circular dependency risk with runtime.ts or oracle-tools.ts.
 */

// ─── Constants ───────────────────────────────────────────────────────────────

export const DEFAULT_CHARS_PER_TOKEN_ESTIMATE = 4;
export const MAX_BOOTSTRAP_STDIN_BYTES = 6_000_000;
export const MAX_INHERITED_WISDOM_INLINE_CHARS = 180_000;
export const DEFAULT_CHECKPOINT_HEADROOM_TOKENS = 250_000;
export const DEFAULT_POOL_SIZE = 2;
export const DEFAULT_IDLE_TIMEOUT_MS = 300_000;       // 5 minutes
export const DEFAULT_MAX_SYNC_BYTES = 5_000_000;      // 5MB safety rail

/**
 * Hardcoded context window sizes per Gemini model.
 * Used as the authoritative source for pressure calculations.
 * Unknown models fall back to 2M (conservative — assumes pro-class).
 */
export const CONTEXT_WINDOW_BY_MODEL: Record<string, number> = {
  "gemini-2.5-pro":         2_000_000,
  "gemini-2.5-flash":       1_000_000,
  "gemini-3-pro-preview":   2_000_000,
  "gemini-3-flash-preview": 1_000_000,
};

export function discoverContextWindow(modelName: string): number {
  return CONTEXT_WINDOW_BY_MODEL[modelName.toLowerCase()] ?? 2_000_000;
}

// ─── Status & Role Types ─────────────────────────────────────────────────────

export type OracleStatus =
  | "healthy"
  | "degraded"        // pool member(s) dead but oracle operational (partial pool failure)
  | "warning"          // context pressure approaching checkpoint threshold
  | "critical"
  | "emergency"
  | "error"
  | "quota_exhausted"
  | "preserving"      // reconstitution in progress — new queries rejected (decision #45)
  | "decommissioned";

export type OracleRecommendation =
  | "healthy"
  | "checkpoint_soon"
  | "checkpoint_now"
  | "reconstitute";

export type CorpusRole =
  | "core_research"
  | "prompt_architecture"
  | "pain_signals"
  | "learnings"
  | "checkpoint"
  | "other";

export type SyncMode = "manual" | "on_spawn" | "interval";

/**
 * hash_gated_delta (default): tree hash fast gate + per-file diff, send only changed files.
 * full_rescan: re-send entire live_sources snapshot regardless of change.
 */
export type ReconstituteSyncMode = "hash_gated_delta" | "full_rescan";

export type InteractionType = "consultation" | "feedback" | "sync_event" | "session_note";
export type InteractionScope = "architectural" | "operational" | "other";

// ─── Data Structures ─────────────────────────────────────────────────────────

export interface DaemonPoolMember {
  daemon_id: string | null;                    // null when soft-dismissed (no live process)
  session_name: string;                        // e.g. "daemon-pythia-0" (stable, survives dismiss)
  session_dir: string | null;
  status: "idle" | "busy" | "dead" | "dismissed"; // dismissed = soft-dismissed, can respawn
  query_count: number;
  chars_in: number;
  chars_out: number;
  last_synced_interaction_id: string | null;   // for cross-daemon context sync
  last_query_at: string | null;                // ISO timestamp — for idle timeout detection
  idle_timeout_ms?: number;                    // default: DEFAULT_IDLE_TIMEOUT_MS (5 min)
  last_corpus_sync_hash: Record<string, string> | null; // per-source tree hashes at last sync
  pending_syncs: Array<{                       // queued corpus syncs awaiting injection
    source_id: string;
    tree_hash: string;
    payload_ref: string;                       // temp file or memory ref
    queued_at: string;
  }>;
}

export interface StaticEntry {
  id?: string;
  path: string;
  role: CorpusRole;
  required: boolean;
  sha256: string;
  added_at: string;
  priority?: number;           // sort order within role group (lower = earlier)
}

export interface LiveSource {
  id: string;
  root: string;
  include: string[];
  exclude: string[];
  role: CorpusRole;
  required: boolean;
  sync_mode: SyncMode;
  interval_seconds?: number;
  max_files?: number;
  max_sync_bytes?: number;                     // default: DEFAULT_MAX_SYNC_BYTES (5MB)
  reconstitute_sync_mode?: ReconstituteSyncMode; // default: "hash_gated_delta"
  priority?: number;                           // sort order within role group
  last_sync_at?: string;
  last_tree_hash?: string;                     // fast gate: did anything change?
  last_file_hashes?: Record<string, string>;   // precise diff: which files changed?
}

export interface OracleManifest {
  schema_version: number;
  name: string;
  description?: string;
  project: string;
  version: number;
  checkpoint_headroom_tokens: number;
  pool_size: number;                           // default: DEFAULT_POOL_SIZE
  static_entries: StaticEntry[];
  live_sources: LiveSource[];
  load_order: CorpusRole[];
  created_at: string;
  last_spawned_at?: string;
}

export interface OracleState {
  schema_version: number;
  oracle_name: string;
  version: number;
  spawned_at: string | null;
  last_spawn_at: string | null;
  discovered_context_window: number | null;
  daemon_pool: DaemonPoolMember[];             // up to pool_size members; spawned on demand
  session_chars_at_spawn: number | null;       // bootstrap payload chars (same for all members)
  chars_per_token_estimate: number;            // default: DEFAULT_CHARS_PER_TOKEN_ESTIMATE
  token_count_method: "exact" | "estimate";    // decision #49: countTokens API vs char heuristic
  estimated_total_tokens: number | null;       // MAX across pool members (drives checkpoint)
  estimated_cluster_tokens: number | null;     // SUM across pool members (observability only)
  tokens_remaining: number | null;             // based on highest-pressure pool member (MAX)
  query_count: number;                         // total queries across all pool members
  last_checkpoint_path: string | null;
  status: OracleStatus;
  lock_held_by: string | null;                 // operation name holding the lock
  lock_expires_at: string | null;              // ISO timestamp — TTL prevents orphans
  last_error: string | null;                   // set when status === "error"
  last_bootstrap_ack: {                        // set after corpus load completes
    ok: boolean;
    raw: string;                               // Pythia's raw ack response
    checked_at: string;
  } | null;
  next_seq: number;                            // decision #49: monotonic counter for InteractionEntry.seq
  generation_since_reground: number;           // decision #51: generations since last full corpus re-grounding
  state_version: number;
  updated_at: string;
}

export interface OracleRegistryEntry {
  name: string;
  oracle_dir: string;                          // absolute path to <project>/oracle/
  project_root: string;                        // absolute path to project root
  description?: string;
  created_at: string;
  decommissioned_at?: string;                  // set on oracle_decommission_execute
}

// ─── Interaction Entry ───────────────────────────────────────────────────────

export interface InteractionEntry {
  // Identity & sequencing
  id: string;                                  // "v<N>-q<NNN>" or "v<N>-q<NNN>-fb"
  seq: number;                                 // monotonic sequence number (oracle-local)
  entry_schema_version: number;                // per-entry schema version (current: 2)
  type: InteractionType;
  oracle_name: string;
  version: number;
  query_count: number;
  timestamp: string;

  // Tracing (OpenTelemetry-compatible, decision #50)
  trace_id: string;                            // groups related operations
  span_id: string;                             // identifies this specific operation
  parent_span_id: string | null;               // links to parent span (null for root spans)

  // Pressure snapshot
  tokens_remaining_at_query: number;
  chars_in_at_query: number;

  // Model provenance
  model_actual?: string;                       // which model actually responded (after fallback)

  // Interaction scope
  interaction_scope?: InteractionScope;

  // Consultation fields
  question?: string;
  ion_delegated?: boolean;
  ion_query?: string;
  ion_response?: string;
  counsel?: string;                            // full raw Pythia response
  counsel_sha256?: string;                     // SHA-256 hash of counsel content
  decision?: string | null;
  quality_signal?: 1 | 2 | 3 | 4 | 5 | null;

  // Causal links
  caused_by?: string[];                        // parent interaction IDs (decision graph)
  flags?: string[];

  // Usage telemetry (from Gemini API response)
  usage?: {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
    cached_tokens?: number;
  };
  latency?: {
    started_at: string;                        // ISO 8601
    first_token_ms?: number;
    duration_ms: number;
  };

  // Feedback fields
  references?: string;                         // consultation id this feedback closes
  implemented?: boolean;
  outcome?: string;
  divergence?: string;                         // how reality differed from counsel
}

// ─── Ion Handoff ─────────────────────────────────────────────────────────────

export interface IonHandoffRequest {
  oracle_name: string;
  version: number;
  query_id: string;                            // consultation id this derives from
  question: string;
  context_paths?: string[];
  timeout_ms?: number;
}

export interface IonHandoffResponse {
  query_id: string;
  success: boolean;
  response: string;
  files_touched?: string[];
  commit_sha?: string;
  error?: string;
  duration_ms: number;
}

// ─── Quality & Degradation ───────────────────────────────────────────────────

export interface DegradationFlag {
  type: "length_drop" | "vagueness" | "self_contradiction" | "hallucination";
  query_id: string;
  tokens_remaining: number;
  description: string;
}

export interface QualityReport {
  oracle_name: string;
  version: number;
  query_count: number;
  degradation_onset_query?: string;
  degradation_onset_tokens_remaining?: number;
  avg_answer_length_early: number;
  avg_answer_length_late: number;
  length_trend_pct_change: number;
  code_symbol_density_early: number;           // ratio: code-like tokens / total words
  code_symbol_density_late: number;
  suggested_headroom_tokens?: number;          // P50(onset) + safety_buffer, clamped
  flags: DegradationFlag[];
}

// ─── Result Envelope ─────────────────────────────────────────────────────────

/**
 * All 25 oracle error codes.
 *
 * DAEMON_BUSY_QUERY: daemon processing a query (seconds) — auto-retry transparently
 * DAEMON_BUSY_LOCK:  heavyweight operation holds the lock (minutes) — surface to user
 */
export type OracleErrorCode =
  | "ORACLE_NOT_FOUND"
  | "ORACLE_ALREADY_EXISTS"
  | "MANIFEST_INVALID"
  | "STATE_INVALID"
  | "DAEMON_NOT_FOUND"
  | "DAEMON_BUSY_QUERY"
  | "DAEMON_BUSY_LOCK"
  | "DAEMON_DEAD"
  | "DAEMON_QUOTA_EXHAUSTED"
  | "FILE_NOT_FOUND"
  | "HASH_MISMATCH"
  | "HASH_MISMATCH_BATCH"
  | "MISSING_REQUIRED_FILE"
  | "PRESSURE_UNAVAILABLE"
  | "CHECKPOINT_FAILED"
  | "BOOTSTRAP_FAILED"
  | "RECONSTITUTE_FAILED"
  | "IO_ERROR"
  | "CONCURRENCY_CONFLICT"
  | "CORPUS_CAP_EXCEEDED"
  | "LOCK_TIMEOUT"
  | "STALE_REGISTRY_PATH"
  | "DECOMMISSION_REFUSED"
  | "DECOMMISSION_TOKEN_EXPIRED"
  | "DECOMMISSION_CANCELLED"
  | "TOTP_INVALID"
  | "CONFIRMATION_PHRASE_MISMATCH";

/**
 * Discriminated union result envelope for all oracle operations.
 * Callers narrow via `result.ok` — TypeScript infers the correct branch.
 */
export type OracleResult<T> =
  | { ok: true; data: T; warnings?: string[] }
  | {
      ok: false;
      error: {
        code: OracleErrorCode;
        message: string;
        retryable: boolean;
        details?: unknown;
      };
    };
