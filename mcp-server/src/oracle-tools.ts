/**
 * Pythia Oracle Engine — Registry, State, Manifest, Locking, Corpus & MCP Tools
 * (FEAT-017, FEAT-018, FEAT-019, FEAT-001, FEAT-024)
 *
 * Phase 1 Step 1.3 + Phase 2 Steps 2.1-2.2 of the implementation plan.
 *
 * Provides:
 *   - Registry management (registry.json — oracle catalog)
 *   - State management (state.json — per-oracle runtime state with optimistic concurrency)
 *   - Manifest management (manifest.json — per-oracle corpus configuration)
 *   - Operation locking (CAS-based advisory lock with TTL + heartbeat)
 *   - Atomic file writes (temp + rename pattern)
 *   - Corpus loading pipeline (resolve, validate, sort, token-gate, stream to daemon)
 *
 * All mutations go through atomicWriteFile() to prevent partial writes.
 * State mutations go through writeStateWithRetry() for optimistic concurrency.
 * Long operations acquire an operation lock with TTL and heartbeat.
 */

import { createHash, createHmac, randomUUID } from "node:crypto";
import { execSync } from "node:child_process";
import { homedir } from "node:os";
import {
  readFile,
  writeFile,
  rename,
  unlink,
  mkdir,
  stat,
  readdir,
  rmdir,
} from "node:fs/promises";
import { appendFileSync, existsSync, globSync, mkdirSync } from "node:fs";
import { basename, extname, join, dirname, relative } from "node:path";

import type {
  OracleRegistryEntry,
  OracleState,
  OracleManifest,
  OracleResult,
  OracleErrorCode,
  OracleStatus,
  OracleRecommendation,
  CorpusRole,
  StaticEntry,
  LiveSource,
  InteractionEntry,
  InteractionType,
  InteractionScope,
  QualityReport,
  DegradationFlag,
} from "./oracle-types.js";

import {
  DEFAULT_CHARS_PER_TOKEN_ESTIMATE,
  DEFAULT_POOL_SIZE,
  DEFAULT_CHECKPOINT_HEADROOM_TOKENS,
  DEFAULT_MAX_SYNC_BYTES,
  MAX_BOOTSTRAP_STDIN_BYTES,
  MAX_INHERITED_WISDOM_INLINE_CHARS,
  discoverContextWindow,
} from "./oracle-types.js";

import { z } from "zod";
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";

import {
  getGeminiRuntime,
  executeWithFallback,
  type OracleRuntimeBridge,
} from "./gemini/runtime.js";
import { getCurrentModel } from "./gemini/model-fallback.js";
// ─── Internal Types ─────────────────────────────────────────────────────────

/** Shape of the registry.json file on disk. */
interface OracleRegistry {
  schema_version: number;
  oracles: Record<string, OracleRegistryEntry>;
}

/** A single resolved corpus entry ready for injection. */
export interface ResolvedCorpusEntry {
  path: string;
  role: CorpusRole;
  content: string;
  sha256: string;
  bytes: number;
  source_type: "static" | "live";
  source_id?: string;           // live_source id (undefined for static entries)
}

/** The fully resolved corpus, ready for Pass 2 injection. */
export interface ResolvedCorpus {
  entries: ResolvedCorpusEntry[];
  total_chars: number;
  total_bytes: number;
  file_count: number;
  estimated_tokens: number;
  stale_file_count: number;
  tree_hashes: Record<string, string>;  // live_source id → tree hash
}

/** Result of loading the resolved corpus into a daemon. */
export interface LoadResult {
  files_loaded: number;
  total_chars_injected: number;
  bootstrap_ack_ok: boolean;
  bootstrap_ack_raw: string;
}

interface StaticEntryScanRow {
  actual_sha256: string;
  content: string;
  entry: StaticEntry;
}

interface StaticEntryScanResult {
  missing_optional: string[];
  missing_required: string[];
  resolved: StaticEntryScanRow[];
  stale_files: Array<{ path: string; expected: string; actual: string }>;
}

interface SpawnAuditEntry {
  timestamp: string;
  oracle_name: string;
  outcome: "success" | "error";
  error_code?: string;
  stale_file_count: number;
  files_loaded: number;
  duration_ms: number;
}

// ─── Constants ──────────────────────────────────────────────────────────────

const REGISTRY_PATH =
  process.env.PYTHIA_REGISTRY_PATH ?? join(homedir(), ".pythia", "registry.json");
const PYTHIA_HOME = process.env.PYTHIA_HOME ?? join(homedir(), ".pythia");
const PYTHIA_LOGS_DIR = join(PYTHIA_HOME, "logs");
const PYTHIA_ORACLES_DIR = join(PYTHIA_HOME, "oracles");
const ORACLE_SPAWN_AUDIT_PATH = join(PYTHIA_LOGS_DIR, "oracle-spawn-audit.jsonl");

/** Where pythia-auth stores TOTP secrets: ~/.pythia/keys/<name>.totp (base32 plaintext) */
const PYTHIA_KEYS_DIR = join(PYTHIA_HOME, "keys");

const DEFAULT_STATE_RETRY_MAX = 5;
const DEFAULT_STATE_RETRY_BASE_MS = 50;
const DEFAULT_STATE_RETRY_JITTER_MS = 30;

const DEFAULT_LOCK_WAIT_TIMEOUT_MS = 30_000;
const DEFAULT_LOCK_TTL_MS = 600_000; // 10 minutes
const DEFAULT_LOCK_POLL_MS = 500;
const DEFAULT_HEARTBEAT_EXTEND_MS = 60_000;
const ORACLE_INIT_CORPUS_CHAR_CAP = 1_500_000;
const ORACLE_INIT_DISCOVERY_PATTERNS = [
  "README.md",
  "docs/**/*.md",
  "docs/**/*.mdx",
  "design/**/*.md",
  "design/**/*.mdx",
  "architecture/**/*.md",
] as const;
const ORACLE_DEFAULT_LOAD_ORDER: CorpusRole[] = [
  "core_research",
  "prompt_architecture",
  "pain_signals",
  "learnings",
  "checkpoint",
  "other",
];

const CURRENT_STATE_SCHEMA_VERSION = 2;
const CURRENT_MANIFEST_SCHEMA_VERSION = 2;
const CURRENT_ENTRY_SCHEMA_VERSION = 2;

// ─── Error Helpers ──────────────────────────────────────────────────────────

function fail<T>(
  code: OracleErrorCode,
  message: string,
  retryable = false,
  details?: unknown,
): OracleResult<T> {
  return { ok: false, error: { code, message, retryable, details } };
}

function ok<T>(data: T, warnings?: string[]): OracleResult<T> {
  return warnings ? { ok: true, data, warnings } : { ok: true, data };
}

function ensurePythiaLogsDir(): void {
  mkdirSync(PYTHIA_LOGS_DIR, { recursive: true });
}

function appendSpawnAuditLog(entry: SpawnAuditEntry): void {
  try {
    ensurePythiaLogsDir();
    appendFileSync(ORACLE_SPAWN_AUDIT_PATH, `${JSON.stringify(entry)}\n`, "utf8");
  } catch {
    // Audit log failures are intentionally non-fatal.
  }
}

// ─── Atomic File Write ──────────────────────────────────────────────────────

/**
 * Write content to a file atomically using the temp-file + rename pattern.
 *
 * The temp file is created in the same directory as the target to guarantee
 * that rename() is a same-filesystem atomic operation on POSIX.
 */
export async function atomicWriteFile(
  filePath: string,
  content: string,
): Promise<void> {
  const dir = dirname(filePath);
  if (!existsSync(dir)) {
    await mkdir(dir, { recursive: true });
  }
  const tmpPath = join(dir, `.tmp-${randomUUID()}`);
  try {
    await writeFile(tmpPath, content, "utf-8");
    await rename(tmpPath, filePath);
  } catch (err) {
    // Best-effort cleanup of temp file on failure
    try {
      await unlink(tmpPath);
    } catch {
      // ignore cleanup failure
    }
    throw err;
  }
}

// ─── Registry Management ────────────────────────────────────────────────────

/**
 * Read and parse the oracle registry from disk.
 * Returns a typed OracleRegistry object.
 */
export async function readRegistry(): Promise<OracleResult<OracleRegistry>> {
  try {
    const raw = await readFile(REGISTRY_PATH, "utf-8");
    const parsed = JSON.parse(raw) as OracleRegistry;
    if (typeof parsed.schema_version !== "number" || !parsed.oracles) {
      return fail("MANIFEST_INVALID", "Registry file has invalid structure");
    }
    return ok(parsed);
  } catch (err: unknown) {
    if ((err as NodeJS.ErrnoException).code === "ENOENT") {
      return fail("FILE_NOT_FOUND", `Registry not found at ${REGISTRY_PATH}`);
    }
    return fail("IO_ERROR", `Failed to read registry: ${(err as Error).message}`);
  }
}

/**
 * Register a new oracle in the registry.
 *
 * Enforces name uniqueness among non-decommissioned oracles.
 * A decommissioned oracle's name can be reused — the old entry is replaced.
 */
export async function registerOracle(
  entry: OracleRegistryEntry,
): Promise<OracleResult<void>> {
  const registryResult = await readRegistry();
  if (!registryResult.ok) return registryResult as OracleResult<void>;

  const registry = registryResult.data;
  const existing = registry.oracles[entry.name];

  if (existing && !existing.decommissioned_at) {
    return fail(
      "ORACLE_ALREADY_EXISTS",
      `Oracle "${entry.name}" already exists and is not decommissioned`,
    );
  }

  registry.oracles[entry.name] = entry;
  try {
    await atomicWriteFile(REGISTRY_PATH, JSON.stringify(registry, null, 2) + "\n");
    return ok(undefined);
  } catch (err: unknown) {
    return fail("IO_ERROR", `Failed to write registry: ${(err as Error).message}`);
  }
}

/**
 * Look up an oracle by name. Returns the entry or null.
 */
export async function lookupOracle(
  name: string,
): Promise<OracleResult<OracleRegistryEntry | null>> {
  const registryResult = await readRegistry();
  if (!registryResult.ok) return registryResult as OracleResult<OracleRegistryEntry | null>;

  const entry = registryResult.data.oracles[name] ?? null;
  return ok(entry);
}

/**
 * Update an existing registry entry with a partial patch.
 * Commonly used to set decommissioned_at.
 */
export async function updateRegistryEntry(
  name: string,
  patch: Partial<OracleRegistryEntry>,
): Promise<OracleResult<void>> {
  const registryResult = await readRegistry();
  if (!registryResult.ok) return registryResult as OracleResult<void>;

  const registry = registryResult.data;
  const existing = registry.oracles[name];
  if (!existing) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${name}" not found in registry`);
  }

  registry.oracles[name] = { ...existing, ...patch };
  try {
    await atomicWriteFile(REGISTRY_PATH, JSON.stringify(registry, null, 2) + "\n");
    return ok(undefined);
  } catch (err: unknown) {
    return fail("IO_ERROR", `Failed to write registry: ${(err as Error).message}`);
  }
}

// ─── State Management ───────────────────────────────────────────────────────

/**
 * Read and validate the oracle state file from an oracle directory.
 */
export async function readState(
  oracleDir: string,
): Promise<OracleResult<OracleState>> {
  const statePath = join(oracleDir, "state.json");
  try {
    const raw = await readFile(statePath, "utf-8");
    const parsed = JSON.parse(raw) as OracleState;
    if (typeof parsed.schema_version !== "number" || typeof parsed.state_version !== "number") {
      return fail("STATE_INVALID", `State file at ${statePath} has invalid structure`);
    }
    return ok(parsed);
  } catch (err: unknown) {
    if ((err as NodeJS.ErrnoException).code === "ENOENT") {
      return fail("FILE_NOT_FOUND", `State file not found at ${statePath}`);
    }
    return fail("IO_ERROR", `Failed to read state: ${(err as Error).message}`);
  }
}

/**
 * Write state with optimistic concurrency control.
 *
 * Read → apply mutator → check state_version hasn't changed → write.
 * If a concurrent write is detected, retry with exponential backoff + jitter.
 *
 * The mutator receives a fresh copy each retry, so it must be idempotent.
 */
export async function writeStateWithRetry(
  oracleDir: string,
  mutator: (state: OracleState) => OracleState,
  opts?: {
    maxRetries?: number;
    baseBackoffMs?: number;
    jitterMs?: number;
  },
): Promise<OracleResult<OracleState>> {
  const maxRetries = opts?.maxRetries ?? DEFAULT_STATE_RETRY_MAX;
  const baseBackoffMs = opts?.baseBackoffMs ?? DEFAULT_STATE_RETRY_BASE_MS;
  const jitterMs = opts?.jitterMs ?? DEFAULT_STATE_RETRY_JITTER_MS;
  const statePath = join(oracleDir, "state.json");

  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    // 1. Read current state
    const readResult = await readState(oracleDir);
    if (!readResult.ok) return readResult;

    const currentState = readResult.data;
    const expectedVersion = currentState.state_version;

    // 2. Apply mutator
    const newState = mutator({ ...currentState });
    newState.schema_version = CURRENT_STATE_SCHEMA_VERSION;
    newState.last_spawn_at = newState.last_spawn_at ?? null;
    newState.state_version = expectedVersion + 1;
    newState.updated_at = new Date().toISOString();

    // 3. Re-read to check for concurrent write
    const verifyResult = await readState(oracleDir);
    if (!verifyResult.ok) return verifyResult;

    if (verifyResult.data.state_version !== expectedVersion) {
      // Concurrent write detected — retry
      if (attempt === maxRetries) {
        return fail(
          "CONCURRENCY_CONFLICT",
          `State version conflict after ${maxRetries + 1} attempts (expected v${expectedVersion}, found v${verifyResult.data.state_version})`,
          true,
        );
      }
      const backoff = baseBackoffMs * Math.pow(2, attempt) + Math.random() * jitterMs;
      await sleep(backoff);
      continue;
    }

    // 4. Write atomically
    try {
      await atomicWriteFile(statePath, JSON.stringify(newState, null, 2) + "\n");
      return ok(newState);
    } catch (err: unknown) {
      return fail("IO_ERROR", `Failed to write state: ${(err as Error).message}`);
    }
  }

  // Should be unreachable, but TypeScript needs it
  return fail("CONCURRENCY_CONFLICT", "Exhausted retries", true);
}

/**
 * Initialize a fresh state.json for a new oracle (version 1).
 */
export async function initState(
  oracleDir: string,
  oracleName: string,
): Promise<OracleResult<OracleState>> {
  const statePath = join(oracleDir, "state.json");

  if (!existsSync(oracleDir)) {
    await mkdir(oracleDir, { recursive: true });
  }

  const initialState: OracleState = {
    schema_version: CURRENT_STATE_SCHEMA_VERSION,
    oracle_name: oracleName,
    version: 1,
    spawned_at: null,
    last_spawn_at: null,
    discovered_context_window: null,
    daemon_pool: [],
    session_chars_at_spawn: null,
    chars_per_token_estimate: DEFAULT_CHARS_PER_TOKEN_ESTIMATE,
    token_count_method: "estimate",
    estimated_total_tokens: null,
    estimated_cluster_tokens: null,
    tokens_remaining: null,
    query_count: 0,
    last_checkpoint_path: null,
    status: "healthy",
    lock_held_by: null,
    lock_expires_at: null,
    last_error: null,
    last_bootstrap_ack: null,
    next_seq: 1,
    generation_since_reground: 0,
    state_version: 1,
    updated_at: new Date().toISOString(),
  };

  try {
    await atomicWriteFile(statePath, JSON.stringify(initialState, null, 2) + "\n");
    return ok(initialState);
  } catch (err: unknown) {
    return fail("IO_ERROR", `Failed to initialize state: ${(err as Error).message}`);
  }
}

// ─── Operation Locking ──────────────────────────────────────────────────────

/**
 * Acquire an advisory operation lock on the oracle state.
 *
 * Uses CAS via writeStateWithRetry. Polls every 500ms up to waitTimeoutMs.
 * Returns DAEMON_BUSY_LOCK if lock is held and timeout exceeded.
 * Lock has a TTL to prevent orphans on crash.
 */
export async function acquireOperationLock(
  oracleDir: string,
  operation: string,
  opts?: {
    waitTimeoutMs?: number;
    lockTtlMs?: number;
  },
): Promise<OracleResult<{ lockToken: string }>> {
  const waitTimeoutMs = opts?.waitTimeoutMs ?? DEFAULT_LOCK_WAIT_TIMEOUT_MS;
  const lockTtlMs = opts?.lockTtlMs ?? DEFAULT_LOCK_TTL_MS;
  const lockToken = randomUUID();
  const deadline = Date.now() + waitTimeoutMs;

  while (true) {
    const stateResult = await readState(oracleDir);
    if (!stateResult.ok) return stateResult as OracleResult<{ lockToken: string }>;

    const state = stateResult.data;
    const now = new Date();

    // Check if lock is held and not expired
    if (state.lock_held_by && state.lock_expires_at) {
      const expiresAt = new Date(state.lock_expires_at);
      if (expiresAt > now) {
        // Lock is held and valid — wait or timeout
        if (Date.now() >= deadline) {
          return fail(
            "DAEMON_BUSY_LOCK",
            `Lock held by "${state.lock_held_by}" until ${state.lock_expires_at}, timeout exceeded`,
            true,
          );
        }
        await sleep(DEFAULT_LOCK_POLL_MS);
        continue;
      }
      // Lock expired — fall through to acquire
    }

    // Attempt to acquire via CAS
    const writeResult = await writeStateWithRetry(oracleDir, (s) => {
      // Double-check: if someone else grabbed the lock between our read and write
      if (s.lock_held_by && s.lock_expires_at) {
        const stillValid = new Date(s.lock_expires_at) > new Date();
        if (stillValid) {
          // Can't set lock — concurrent acquisition. The CAS will fail gracefully
          // by returning the state unchanged, which we'll detect below.
          return s;
        }
      }
      s.lock_held_by = `${operation}:${lockToken}`;
      s.lock_expires_at = new Date(Date.now() + lockTtlMs).toISOString();
      return s;
    });

    if (!writeResult.ok) {
      // CAS conflict or I/O error — retry if within timeout
      if (writeResult.error.code === "CONCURRENCY_CONFLICT" && Date.now() < deadline) {
        await sleep(DEFAULT_LOCK_POLL_MS);
        continue;
      }
      return writeResult as OracleResult<{ lockToken: string }>;
    }

    // Verify we actually got the lock (not a no-op from concurrent acquisition)
    if (writeResult.data.lock_held_by === `${operation}:${lockToken}`) {
      return ok({ lockToken });
    }

    // Someone else grabbed it — retry if within timeout
    if (Date.now() >= deadline) {
      return fail(
        "DAEMON_BUSY_LOCK",
        `Lock acquired by another operation: "${writeResult.data.lock_held_by}"`,
        true,
      );
    }
    await sleep(DEFAULT_LOCK_POLL_MS);
  }
}

/**
 * Release an operation lock. Only clears if the lockToken matches.
 */
export async function releaseLock(
  oracleDir: string,
  lockToken: string,
): Promise<OracleResult<void>> {
  const writeResult = await writeStateWithRetry(oracleDir, (s) => {
    if (s.lock_held_by && s.lock_held_by.endsWith(`:${lockToken}`)) {
      s.lock_held_by = null;
      s.lock_expires_at = null;
    }
    return s;
  });

  if (!writeResult.ok) return writeResult as OracleResult<void>;
  return ok(undefined);
}

/**
 * Start a heartbeat that extends lock_expires_at periodically.
 *
 * Returns a handle with stop() to clean up the interval.
 * The heartbeat prevents lock expiry during legitimately long operations.
 */
export function startLockHeartbeat(opts: {
  oracleDir: string;
  operation: string;
  lockToken: string;
  extendEveryMs?: number;
  ttlMs?: number;
}): { stop: () => Promise<void> } {
  const extendEveryMs = opts.extendEveryMs ?? DEFAULT_HEARTBEAT_EXTEND_MS;
  const ttlMs = opts.ttlMs ?? DEFAULT_LOCK_TTL_MS;
  let stopped = false;

  const intervalId = setInterval(async () => {
    if (stopped) return;
    try {
      await writeStateWithRetry(opts.oracleDir, (s) => {
        if (s.lock_held_by === `${opts.operation}:${opts.lockToken}`) {
          s.lock_expires_at = new Date(Date.now() + ttlMs).toISOString();
        }
        return s;
      });
    } catch {
      // Heartbeat failures are non-fatal — the lock will eventually expire
    }
  }, extendEveryMs);

  return {
    stop: async () => {
      stopped = true;
      clearInterval(intervalId);
    },
  };
}

// ─── Manifest Management ────────────────────────────────────────────────────

/**
 * Read and validate the oracle manifest from an oracle directory.
 */
export async function readManifest(
  oracleDir: string,
): Promise<OracleResult<OracleManifest>> {
  const manifestPath = join(oracleDir, "manifest.json");
  try {
    const raw = await readFile(manifestPath, "utf-8");
    const parsed = JSON.parse(raw) as OracleManifest;
    if (typeof parsed.schema_version !== "number" || !parsed.name || !parsed.project) {
      return fail(
        "MANIFEST_INVALID",
        `Manifest at ${manifestPath} missing required fields (schema_version, name, project)`,
      );
    }
    return ok(parsed);
  } catch (err: unknown) {
    if ((err as NodeJS.ErrnoException).code === "ENOENT") {
      return fail("FILE_NOT_FOUND", `Manifest not found at ${manifestPath}`);
    }
    if (err instanceof SyntaxError) {
      return fail("MANIFEST_INVALID", `Manifest at ${manifestPath} contains invalid JSON`);
    }
    return fail("IO_ERROR", `Failed to read manifest: ${(err as Error).message}`);
  }
}

function normalizeManifestForWrite(manifest: OracleManifest): OracleManifest {
  return {
    ...manifest,
    schema_version: CURRENT_MANIFEST_SCHEMA_VERSION,
    description: manifest.description ?? "",
  };
}

/**
 * Write manifest to an oracle directory atomically.
 */
export async function writeManifest(
  oracleDir: string,
  manifest: OracleManifest,
): Promise<OracleResult<void>> {
  const manifestPath = join(oracleDir, "manifest.json");
  try {
    await atomicWriteFile(manifestPath, JSON.stringify(normalizeManifestForWrite(manifest), null, 2) + "\n");
    return ok(undefined);
  } catch (err: unknown) {
    return fail("IO_ERROR", `Failed to write manifest: ${(err as Error).message}`);
  }
}

async function acquireManifestLock(
  manifestPath: string,
  timeoutMs = 5_000,
  pollMs = 50,
): Promise<OracleResult<{ lockPath: string; token: string }>> {
  const lockPath = `${manifestPath}.lock`;
  const token = randomUUID();
  const deadline = Date.now() + timeoutMs;

  while (Date.now() <= deadline) {
    try {
      await writeFile(lockPath, JSON.stringify({ token, created_at: new Date().toISOString() }), {
        encoding: "utf-8",
        flag: "wx",
      });
      return ok({ lockPath, token });
    } catch (err: unknown) {
      if ((err as NodeJS.ErrnoException).code !== "EEXIST") {
        return fail("IO_ERROR", `Failed to acquire manifest lock: ${(err as Error).message}`);
      }
      await sleep(pollMs);
    }
  }

  return fail("DAEMON_BUSY_LOCK", `Manifest lock timed out for ${manifestPath}`, true);
}

async function releaseManifestLock(lockPath: string, token: string): Promise<void> {
  try {
    const raw = await readFile(lockPath, "utf-8");
    const parsed = JSON.parse(raw) as { token?: string };
    if (parsed.token !== token) return;
  } catch {
    // Missing or unreadable lock file — nothing to release.
  }

  try {
    await unlink(lockPath);
  } catch {
    // Best-effort cleanup only.
  }
}

async function writeManifestLocked(
  oracleDir: string,
  manifest: OracleManifest,
): Promise<OracleResult<void>> {
  const manifestPath = join(oracleDir, "manifest.json");
  const tmpPath = join(oracleDir, "manifest.tmp.json");

  try {
    await writeFile(tmpPath, JSON.stringify(normalizeManifestForWrite(manifest), null, 2) + "\n", "utf-8");
    await rename(tmpPath, manifestPath);
    return ok(undefined);
  } catch (err: unknown) {
    try {
      await unlink(tmpPath);
    } catch {
      // ignore cleanup failure
    }
    return fail("IO_ERROR", `Failed to write manifest: ${(err as Error).message}`);
  }
}

const SOURCE_LIKE_EXTENSIONS = new Set([
  ".c",
  ".cc",
  ".cpp",
  ".cs",
  ".css",
  ".go",
  ".h",
  ".hpp",
  ".html",
  ".java",
  ".js",
  ".json",
  ".jsx",
  ".mjs",
  ".php",
  ".py",
  ".rb",
  ".rs",
  ".scss",
  ".sh",
  ".sql",
  ".ts",
  ".tsx",
  ".xml",
  ".yaml",
  ".yml",
]);

function isSourceLikeFile(filePath: string): boolean {
  return SOURCE_LIKE_EXTENSIONS.has(extname(filePath).toLowerCase());
}

function normalizeCorpusFiles(params: {
  file_path?: string;
  files?: string | string[];
}): string[] {
  if (params.files !== undefined) {
    return Array.isArray(params.files) ? params.files : [params.files];
  }

  if (params.file_path !== undefined) {
    return [params.file_path];
  }

  return [];
}

function formatBatchCorpusPrompt(entries: Array<{ path: string; content: string }>): string {
  const payload = entries.map(({ path, content }) => (
    `<<<FILE path="${path}">>>\n${content}\n<<<END_FILE>>>`
  )).join("\n\n");

  return [
    "New corpus entries were added. Read and internalize them as a single batch.",
    "",
    payload,
  ].join("\n");
}

async function computeStaticCorpusCharTotal(
  staticEntries: StaticEntry[],
  newContents: Map<string, string>,
): Promise<number> {
  let totalChars = 0;

  for (const entry of staticEntries) {
    const cached = newContents.get(entry.path);
    if (cached !== undefined) {
      totalChars += cached.length;
      continue;
    }

    try {
      totalChars += (await readFile(entry.path, "utf-8")).length;
    } catch {
      // Ignore unreadable pre-existing entries for this advisory estimate.
    }
  }

  return totalChars;
}

// ─── Corpus Loading Pipeline (FEAT-019) ─────────────────────────────────────

/**
 * Compute a deterministic tree hash from a set of per-file hashes.
 *
 * Sorts file paths alphabetically, concatenates "path:hash\n" pairs,
 * then returns sha256 of the result. Deterministic regardless of
 * insertion order in the input record.
 */
export function computeTreeHash(fileHashes: Record<string, string>): string {
  const sorted = Object.keys(fileHashes).sort();
  const payload = sorted.map((p) => `${p}:${fileHashes[p]}`).join("\n");
  return createHash("sha256").update(payload).digest("hex");
}

/**
 * Validate Pythia's bootstrap acknowledgment response.
 *
 * Returns false if the response is short (< 100 chars) AND contains
 * confusion markers, indicating Pythia didn't understand the corpus load.
 */
export function validateBootstrapAck(text: string): boolean {
  if (text.length >= 100) return true;
  const lower = text.toLowerCase();
  const confusionMarkers = ["error", "cannot", "fail", "unable", "don't understand"];
  return !confusionMarkers.some((marker) => lower.includes(marker));
}

/**
 * Build the spawn preamble that establishes Pythia's identity and context.
 *
 * Three modes:
 * - v1 (no inherited wisdom): first-generation preamble
 * - inline (wisdom <= 180K chars): full checkpoint embedded in <inherited_wisdom> tags
 * - summary (wisdom > 180K chars): brief lineage summary (full checkpoint loaded separately in Pass 2)
 */
export function buildSpawnPreamble(opts: {
  oracleName: string;
  project: string;
  nextVersion: number;
  inheritedWisdom?: string | null;
}): string {
  const header = [
    `You are Pythia, a persistent knowledge oracle for the "${opts.project}" project.`,
    `Oracle name: ${opts.oracleName}`,
    `Generation: v${opts.nextVersion}`,
    "",
    "Your role is to maintain deep architectural knowledge across sessions.",
    "You will receive corpus files that form your knowledge base.",
    "When consulted, draw on this grounded knowledge to provide precise, actionable counsel.",
    "",
  ];

  if (!opts.inheritedWisdom) {
    // v1 first-generation preamble
    header.push(
      "This is your first generation. You have no prior checkpoint to inherit.",
      "Build your understanding from the corpus files that follow.",
    );
    return header.join("\n");
  }

  if (opts.inheritedWisdom.length <= MAX_INHERITED_WISDOM_INLINE_CHARS) {
    // Inline embedding
    header.push(
      `You are generation v${opts.nextVersion}, inheriting wisdom from v${opts.nextVersion - 1}.`,
      "The following is your inherited checkpoint — your accumulated knowledge from the prior generation:",
      "",
      "<inherited_wisdom>",
      opts.inheritedWisdom,
      "</inherited_wisdom>",
      "",
      "Build on this foundation with the corpus files that follow.",
    );
    return header.join("\n");
  }

  // Brief summary — full checkpoint loaded as first static chunk in Pass 2
  header.push(
    `You are generation v${opts.nextVersion}, inheriting wisdom from v${opts.nextVersion - 1}.`,
    `Your inherited checkpoint is ${opts.inheritedWisdom.length.toLocaleString()} characters — too large for inline embedding.`,
    "It will be loaded as the first corpus file in the next phase.",
    "Treat that file as your inherited knowledge base.",
  );
  return header.join("\n");
}

/**
 * Compute SHA-256 hash of a string.
 */
function sha256(content: string): string {
  return createHash("sha256").update(content).digest("hex");
}

async function scanStaticEntries(staticEntries: StaticEntry[]): Promise<OracleResult<StaticEntryScanResult>> {
  const scan: StaticEntryScanResult = {
    resolved: [],
    stale_files: [],
    missing_required: [],
    missing_optional: [],
  };

  for (const entry of staticEntries) {
    let content: string;
    try {
      content = await readFile(entry.path, "utf-8");
    } catch (err: unknown) {
      if ((err as NodeJS.ErrnoException).code === "ENOENT") {
        if (entry.required) {
          scan.missing_required.push(entry.path);
        } else {
          scan.missing_optional.push(entry.path);
        }
        continue;
      }

      return fail("IO_ERROR", `Failed to read static entry ${entry.path}: ${(err as Error).message}`);
    }

    const actual_sha256 = sha256(content);
    if (actual_sha256 !== entry.sha256) {
      scan.stale_files.push({
        path: entry.path,
        expected: entry.sha256,
        actual: actual_sha256,
      });
    }

    scan.resolved.push({
      entry,
      content,
      actual_sha256,
    });
  }

  return ok(scan);
}

async function prepareStaticEntriesForSpawn(
  oracleDir: string,
  manifest: OracleManifest,
  opts: { auto_refresh?: boolean } = {},
): Promise<OracleResult<{
  entries: ResolvedCorpusEntry[];
  manifest: OracleManifest;
  stale_file_count: number;
  warnings: string[];
}>> {
  const autoRefresh = opts.auto_refresh ?? false;
  const scanResult = await scanStaticEntries(manifest.static_entries);
  if (!scanResult.ok) {
    return scanResult as OracleResult<{
      entries: ResolvedCorpusEntry[];
      manifest: OracleManifest;
      stale_file_count: number;
      warnings: string[];
    }>;
  }

  const scan = scanResult.data;
  if (scan.missing_required.length > 0) {
    return fail(
      "MISSING_REQUIRED_FILE",
      `Required corpus files are missing for oracle at ${oracleDir}`,
      false,
      {
        error_code: "MISSING_REQUIRED_FILE",
        missing_files: scan.missing_required,
      },
    );
  }

  if (!autoRefresh && scan.stale_files.length > 0) {
    return fail(
      "HASH_MISMATCH_BATCH",
      "Multiple files have stale hashes",
      false,
      {
        error_code: "HASH_MISMATCH_BATCH",
        stale_files: scan.stale_files,
      },
    );
  }

  const warnings: string[] = [];
  let nextManifest = manifest;
  const staleByPath = new Map(scan.stale_files.map((row) => [row.path, row.actual]));
  const missingOptional = new Set(scan.missing_optional);

  if (scan.missing_optional.length > 0 && !autoRefresh) {
    for (const filePath of scan.missing_optional) {
      warnings.push(`Optional static entry missing: ${filePath}`);
    }
  }

  if (autoRefresh && (scan.stale_files.length > 0 || scan.missing_optional.length > 0)) {
    nextManifest = {
      ...manifest,
      static_entries: manifest.static_entries.flatMap((entry) => {
        if (missingOptional.has(entry.path)) {
          return [];
        }

        const refreshedHash = staleByPath.get(entry.path);
        if (refreshedHash !== undefined) {
          return [{
            ...entry,
            sha256: refreshedHash,
          }];
        }

        return [entry];
      }),
    };

    const writeResult = await writeManifest(oracleDir, nextManifest);
    if (!writeResult.ok) {
      return writeResult as OracleResult<{
        entries: ResolvedCorpusEntry[];
        manifest: OracleManifest;
        stale_file_count: number;
        warnings: string[];
      }>;
    }
  }

  return ok({
    manifest: nextManifest,
    stale_file_count: scan.stale_files.length,
    warnings,
    entries: scan.resolved.map(({ entry, content, actual_sha256 }) => ({
      path: entry.path,
      role: entry.role,
      content,
      sha256: actual_sha256,
      bytes: Buffer.byteLength(content, "utf-8"),
      source_type: "static",
    })),
  });
}

/**
 * Resolve all corpus files for a spawn operation (Pass 1).
 *
 * This does ALL I/O and validation before any daemon exists:
 * - Reads and hash-checks static entries
 * - Glob-resolves live sources with max_files and max_sync_bytes caps
 * - Sorts by load_order role priority, then priority/added_at/path within each role
 * - Applies token gate and stdin byte gate
 *
 * Returns a ResolvedCorpus ready for Pass 2 injection.
 */
export async function resolveCorpusForSpawn(
  oracleDir: string,
  opts: { auto_refresh?: boolean } = {},
): Promise<OracleResult<ResolvedCorpus>> {
  // Read manifest
  const manifestResult = await readManifest(oracleDir);
  if (!manifestResult.ok) return manifestResult as OracleResult<ResolvedCorpus>;
  const manifest = manifestResult.data;

  const entries: ResolvedCorpusEntry[] = [];
  const treeHashes: Record<string, string> = {};
  const warnings: string[] = [];

  const staticResult = await prepareStaticEntriesForSpawn(oracleDir, manifest, opts);
  if (!staticResult.ok) {
    return staticResult as OracleResult<ResolvedCorpus>;
  }

  const resolvedManifest = staticResult.data.manifest;
  entries.push(...staticResult.data.entries);
  warnings.push(...staticResult.data.warnings);

  // ── Live sources ────────────────────────────────────────────────────────
  for (const source of resolvedManifest.live_sources) {
    const maxSyncBytes = source.max_sync_bytes ?? DEFAULT_MAX_SYNC_BYTES;
    const fileHashes: Record<string, string> = {};
    let sourceTotalBytes = 0;

    // Resolve include globs, excluding exclude patterns
    let resolvedPaths: string[] = [];
    for (const includePattern of source.include) {
      const fullPattern = join(source.root, includePattern);
      try {
        const matched = globSync(fullPattern, {
          exclude: (source.exclude ?? []).map((ex) => {
            // If exclude pattern is absolute, use as-is; otherwise prefix with root
            return ex.startsWith("/") ? ex : join(source.root, ex);
          }),
        });
        resolvedPaths.push(...matched);
      } catch {
        warnings.push(`Glob failed for pattern "${fullPattern}" in live_source "${source.id}"`);
      }
    }

    // Deduplicate and sort for determinism
    resolvedPaths = [...new Set(resolvedPaths)].sort();

    // Enforce max_files cap
    if (source.max_files && resolvedPaths.length > source.max_files) {
      resolvedPaths = resolvedPaths.slice(0, source.max_files);
      warnings.push(
        `Live source "${source.id}" capped at ${source.max_files} files (${resolvedPaths.length} resolved)`,
      );
    }

    // Read files, enforce max_sync_bytes
    for (const filePath of resolvedPaths) {
      // Skip directories
      try {
        const fileStat = await stat(filePath);
        if (fileStat.isDirectory()) continue;
      } catch {
        continue; // Skip files we can't stat
      }

      let content: string;
      try {
        content = await readFile(filePath, "utf-8");
      } catch {
        if (source.required) {
          return fail("FILE_NOT_FOUND", `Live source file unreadable: ${filePath}`);
        }
        warnings.push(`Skipping unreadable file: ${filePath}`);
        continue;
      }

      const fileBytes = Buffer.byteLength(content, "utf-8");
      if (sourceTotalBytes + fileBytes > maxSyncBytes) {
        warnings.push(
          `Live source "${source.id}" hit max_sync_bytes (${maxSyncBytes}), skipping remaining files`,
        );
        break;
      }
      sourceTotalBytes += fileBytes;

      const hash = sha256(content);
      const relPath = relative(source.root, filePath);
      fileHashes[relPath] = hash;

      entries.push({
        path: filePath,
        role: source.role,
        content,
        sha256: hash,
        bytes: fileBytes,
        source_type: "live",
        source_id: source.id,
      });
    }

    // Compute tree hash for this live source
    treeHashes[source.id] = computeTreeHash(fileHashes);
  }

  // ── Sort by load_order ──────────────────────────────────────────────────
  const roleOrder = new Map<string, number>();
  resolvedManifest.load_order.forEach((role, idx) => roleOrder.set(role, idx));

  entries.sort((a, b) => {
    // Primary: role order from manifest.load_order
    const roleA = roleOrder.get(a.role) ?? 999;
    const roleB = roleOrder.get(b.role) ?? 999;
    if (roleA !== roleB) return roleA - roleB;

    // Secondary: priority ASC (find priority from manifest entries)
    const prioA = findPriority(resolvedManifest, a) ?? Infinity;
    const prioB = findPriority(resolvedManifest, b) ?? Infinity;
    if (prioA !== prioB) return prioA - prioB;

    // Tertiary: added_at ASC (static entries only; live entries use path)
    const addedA = findAddedAt(resolvedManifest, a) ?? "";
    const addedB = findAddedAt(resolvedManifest, b) ?? "";
    if (addedA !== addedB) return addedA.localeCompare(addedB);

    // Quaternary: path ASC
    return a.path.localeCompare(b.path);
  });

  // ── Compute totals ────────────────────────────────────────────────────
  const totalChars = entries.reduce((sum, e) => sum + e.content.length, 0);
  const totalBytes = entries.reduce((sum, e) => sum + e.bytes, 0);
  const estimatedTokens = Math.ceil(totalChars / DEFAULT_CHARS_PER_TOKEN_ESTIMATE);

  // ── Token gate ────────────────────────────────────────────────────────
  const contextWindow = discoverContextWindow("gemini-2.5-pro"); // conservative default
  const available = contextWindow - resolvedManifest.checkpoint_headroom_tokens;
  if (estimatedTokens > available) {
    return fail(
      "CORPUS_CAP_EXCEEDED",
      `Estimated ${estimatedTokens.toLocaleString()} tokens exceeds available ${available.toLocaleString()} (context: ${contextWindow.toLocaleString()} - headroom: ${resolvedManifest.checkpoint_headroom_tokens.toLocaleString()})`,
      false,
      { estimatedTokens, available, contextWindow, headroom: resolvedManifest.checkpoint_headroom_tokens },
    );
  }

  // ── Stdin byte gate ───────────────────────────────────────────────────
  if (totalBytes > MAX_BOOTSTRAP_STDIN_BYTES) {
    return fail(
      "CORPUS_CAP_EXCEEDED",
      `Total corpus ${totalBytes.toLocaleString()} bytes exceeds MAX_BOOTSTRAP_STDIN_BYTES (${MAX_BOOTSTRAP_STDIN_BYTES.toLocaleString()})`,
      false,
      { totalBytes, limit: MAX_BOOTSTRAP_STDIN_BYTES },
    );
  }

  return ok(
    {
      entries,
      total_chars: totalChars,
      total_bytes: totalBytes,
      file_count: entries.length,
      estimated_tokens: estimatedTokens,
      stale_file_count: staticResult.data.stale_file_count,
      tree_hashes: treeHashes,
    },
    warnings.length > 0 ? warnings : undefined,
  );
}

/**
 * Load a resolved corpus into a live daemon (Pass 2).
 *
 * Iterates through entries in load order, sends each to the daemon
 * with injection markers, then sends a final acknowledgment prompt
 * and validates the response.
 */
export async function loadResolvedCorpusIntoDaemon(
  daemonId: string,
  resolvedCorpus: ResolvedCorpus,
  runtime: OracleRuntimeBridge,
): Promise<OracleResult<LoadResult>> {
  let totalCharsInjected = 0;

  for (const entry of resolvedCorpus.entries) {
    const injectionPayload = `[Corpus file: ${entry.path} | role: ${entry.role} | sha256: ${entry.sha256}]\n${entry.content}`;
    try {
      const result = await runtime.askDaemon({
        daemon_id: daemonId,
        question: injectionPayload,
        timeout_ms: 120_000,
      });
      totalCharsInjected += result.chars_in;
    } catch (err: unknown) {
      return fail(
        "BOOTSTRAP_FAILED",
        `Failed to inject corpus file ${entry.path}: ${(err as Error).message}`,
      );
    }
  }

  // Send final acknowledgment prompt
  const ackPrompt = [
    "[Corpus load complete]",
    `Files loaded: ${resolvedCorpus.file_count}`,
    `Total chars: ${resolvedCorpus.total_chars.toLocaleString()}`,
    "",
    "Acknowledge receipt of all corpus files. Confirm you have access to the loaded knowledge base.",
    "If any files were unclear or caused confusion, state that explicitly.",
  ].join("\n");

  let ackResponse: string;
  try {
    const result = await runtime.askDaemon({
      daemon_id: daemonId,
      question: ackPrompt,
      timeout_ms: 60_000,
    });
    ackResponse = result.text;
    totalCharsInjected += result.chars_in;
  } catch (err: unknown) {
    return fail(
      "BOOTSTRAP_FAILED",
      `Failed to get bootstrap acknowledgment: ${(err as Error).message}`,
    );
  }

  const ackOk = validateBootstrapAck(ackResponse);

  return ok({
    files_loaded: resolvedCorpus.file_count,
    total_chars_injected: totalCharsInjected,
    bootstrap_ack_ok: ackOk,
    bootstrap_ack_raw: ackResponse,
  });
}

// ─── Corpus Sort Helpers ────────────────────────────────────────────────────

function findPriority(manifest: OracleManifest, entry: ResolvedCorpusEntry): number | undefined {
  if (entry.source_type === "static") {
    const se = manifest.static_entries.find((s) => s.path === entry.path);
    return se?.priority;
  }
  const ls = manifest.live_sources.find((s) => s.id === entry.source_id);
  return ls?.priority;
}

function findAddedAt(manifest: OracleManifest, entry: ResolvedCorpusEntry): string | undefined {
  if (entry.source_type === "static") {
    const se = manifest.static_entries.find((s) => s.path === entry.path);
    return se?.added_at;
  }
  return undefined; // live entries don't have added_at
}

// ─── spawn_oracle Handler (FEAT-001, FEAT-024) ─────────────────────────────

/** Shape of the .pythia-active/<name>.json marker file. */
interface PythiaActiveMarker {
  oracle_name: string;
  oracle_dir: string;
  project_root: string;
  pool_members_active: number;
  written_at: string;
}

/** Return type of spawn_oracle. */
export interface SpawnOracleResult {
  oracle_name: string;
  version: number;
  pool: Array<{
    daemon_id: string;
    session_name: string;
    status: string;
  }>;
  resumed: boolean;
  corpus_files_loaded: number;
  tokens_remaining: number | null;
}

/**
 * Core spawn_oracle logic — handles the 6-combination parameter matrix.
 *
 * This is the internal handler; registerOracleTools wraps it as an MCP tool.
 */
async function spawnOracleInternal(input: {
  name: string;
  reuse_existing?: boolean;
  force_reload?: boolean;
  auto_refresh?: boolean;
  force?: boolean;
  timeout_ms?: number;
}, audit: { files_loaded: number; stale_file_count: number }): Promise<OracleResult<SpawnOracleResult>> {
  const autoRefresh = input.auto_refresh ?? false;
  const reuseExisting = input.reuse_existing ?? true;
  const forceReload = input.force_reload ?? false;
  const timeoutMs = input.timeout_ms ?? 300_000;
  const runtime = getGeminiRuntime();

  // 1. Look up oracle in registry
  const lookupResult = await lookupOracle(input.name);
  if (!lookupResult.ok) return lookupResult as OracleResult<SpawnOracleResult>;

  const registryEntry = lookupResult.data;

  // If not in registry, check if manifest exists at a conventional path
  if (!registryEntry) {
    return fail(
      "ORACLE_NOT_FOUND",
      `Oracle "${input.name}" not found in registry. Register it first or ensure manifest exists.`,
    );
  }

  if (registryEntry.decommissioned_at) {
    return fail(
      "ORACLE_NOT_FOUND",
      `Oracle "${input.name}" has been decommissioned (${registryEntry.decommissioned_at})`,
    );
  }

  const oracleDir = registryEntry.oracle_dir;
  const projectRoot = registryEntry.project_root;
  const sessionName = `daemon-${input.name}-0`;

  // 2. Check for existing session
  const existingDaemon = runtime.findDaemonBySessionName(sessionName);

  // ── Parameter matrix dispatch ──────────────────────────────────────────

  // Case: reuse_existing=false, session exists → ORACLE_ALREADY_EXISTS
  if (!reuseExisting && existingDaemon) {
    return fail(
      "ORACLE_ALREADY_EXISTS",
      `Oracle "${input.name}" has an active session. Use reuse_existing=true to resume, or decommission first.`,
    );
  }

  // Case: reuse_existing=true, session exists, no force_reload → Resume
  if (reuseExisting && existingDaemon && !forceReload) {
    const manifestResult = await readManifest(oracleDir);
    if (!manifestResult.ok) {
      return manifestResult as OracleResult<SpawnOracleResult>;
    }
    const staticResult = await prepareStaticEntriesForSpawn(oracleDir, manifestResult.data, {
      auto_refresh: autoRefresh,
    });
    if (!staticResult.ok) {
      return staticResult as OracleResult<SpawnOracleResult>;
    }
    audit.stale_file_count = staticResult.data.stale_file_count;

    const stateResult = await readState(oracleDir);
    const state = stateResult.ok ? stateResult.data : null;

    // Reset last_query_at for all non-dismissed/dead pool members so the idle sweep
    // gives a fresh TTL window. Without this, a daemon resumed after >5 min of inactivity
    // is immediately dismissed by the next sweep tick (BUG-2 fix).
    const resumeStateWrite = await writeStateWithRetry(oracleDir, (s) => {
      const now = new Date().toISOString();
      for (const m of s.daemon_pool) {
        if (m.status !== "dismissed" && m.status !== "dead") {
          m.last_query_at = now;
        }
      }
      s.last_spawn_at = now;
      return s;
    });
    if (!resumeStateWrite.ok) {
      return resumeStateWrite as OracleResult<SpawnOracleResult>;
    }

    return ok({
      oracle_name: input.name,
      version: state?.version ?? 1,
      pool: [{
        daemon_id: existingDaemon.daemon_id,
        session_name: sessionName,
        status: "idle",
      }],
      resumed: true,
      corpus_files_loaded: 0,
      tokens_remaining: state?.tokens_remaining ?? null,
    });
  }

  // Case: reuse_existing=true, session exists, force_reload=true → Re-send corpus
  if (reuseExisting && existingDaemon && forceReload) {
    const corpusResult = await resolveCorpusForSpawn(oracleDir, { auto_refresh: autoRefresh });
    if (!corpusResult.ok) return corpusResult as OracleResult<SpawnOracleResult>;
    audit.stale_file_count = corpusResult.data.stale_file_count;

    const loadResult = await loadResolvedCorpusIntoDaemon(
      existingDaemon.daemon_id,
      corpusResult.data,
      runtime,
    );
    if (!loadResult.ok) return loadResult as OracleResult<SpawnOracleResult>;
    audit.files_loaded = corpusResult.data.entries.filter((entry) => entry.source_type === "static").length;

    // Update session_chars_at_spawn in state
    const updateResult = await writeStateWithRetry(oracleDir, (s) => {
      s.session_chars_at_spawn = corpusResult.data.total_chars;
      s.last_spawn_at = new Date().toISOString();
      s.last_bootstrap_ack = {
        ok: loadResult.data.bootstrap_ack_ok,
        raw: loadResult.data.bootstrap_ack_raw,
        checked_at: new Date().toISOString(),
      };
      if (!loadResult.data.bootstrap_ack_ok) {
        s.status = "error";
        s.last_error = "Bootstrap ack validation failed after force reload";
      }
      return s;
    });

    return ok({
      oracle_name: input.name,
      version: updateResult.ok ? updateResult.data.version : 1,
      pool: [{
        daemon_id: existingDaemon.daemon_id,
        session_name: sessionName,
        status: "idle",
      }],
      resumed: true,
      corpus_files_loaded: corpusResult.data.file_count,
      tokens_remaining: updateResult.ok ? updateResult.data.tokens_remaining : null,
    });
  }

  // ── Fresh spawn flow (no existing session, or reuse_existing=true but daemon dead) ──

  // Check if daemon died but state exists (resume via re-spawn)
  const stateExists = existsSync(join(oracleDir, "state.json"));
  let inheritedWisdom: string | null = null;
  let nextVersion = 1;

  if (reuseExisting && !existingDaemon && stateExists) {
    // Daemon died — re-spawn with checkpoint if available
    const stateResult = await readState(oracleDir);
    if (stateResult.ok && stateResult.data.last_checkpoint_path) {
      try {
        inheritedWisdom = await readFile(stateResult.data.last_checkpoint_path, "utf-8");
      } catch {
        // Checkpoint missing — proceed without inherited wisdom
      }
      nextVersion = stateResult.data.version + 1;
    } else if (stateResult.ok) {
      nextVersion = stateResult.data.version;
    }
  }

  // Pass 1: Resolve corpus
  const corpusResult = await resolveCorpusForSpawn(oracleDir, { auto_refresh: autoRefresh });
  if (!corpusResult.ok) return corpusResult as OracleResult<SpawnOracleResult>;
  audit.stale_file_count = corpusResult.data.stale_file_count;

  // Discover context window from current model
  const currentModel = getCurrentModel();
  const contextWindow = discoverContextWindow(currentModel);

  // Build preamble
  const preamble = buildSpawnPreamble({
    oracleName: input.name,
    project: projectRoot,
    nextVersion,
    inheritedWisdom,
  });

  // Spawn daemon
  let spawnResult: { daemon_id: string; resumed: boolean; session_dir: string };
  try {
    spawnResult = await runtime.spawnDaemon({
      session_name: sessionName,
      cwd: projectRoot,
      timeout_ms: timeoutMs,
    });
  } catch (err: unknown) {
    return fail(
      "BOOTSTRAP_FAILED",
      `Failed to spawn daemon: ${(err as Error).message}`,
    );
  }

  // Send preamble as first message
  try {
    await runtime.askDaemon({
      daemon_id: spawnResult.daemon_id,
      question: preamble,
      timeout_ms: 120_000,
    });
  } catch (err: unknown) {
    return fail(
      "BOOTSTRAP_FAILED",
      `Failed to send preamble: ${(err as Error).message}`,
    );
  }

  // Pass 2: Load corpus into daemon
  const loadResult = await loadResolvedCorpusIntoDaemon(
    spawnResult.daemon_id,
    corpusResult.data,
    runtime,
  );
  if (!loadResult.ok) return loadResult as OracleResult<SpawnOracleResult>;
  audit.files_loaded = corpusResult.data.entries.filter((entry) => entry.source_type === "static").length;

  // Initialize or update state
  let finalState: OracleState;
  if (!stateExists || !reuseExisting) {
    const initResult = await initState(oracleDir, input.name);
    if (!initResult.ok) return initResult as OracleResult<SpawnOracleResult>;
    finalState = initResult.data;
  } else {
    const readResult = await readState(oracleDir);
    if (!readResult.ok) return readResult as OracleResult<SpawnOracleResult>;
    finalState = readResult.data;
  }

  // Update state with spawn details
  const stateUpdateResult = await writeStateWithRetry(oracleDir, (s) => {
    const now = new Date().toISOString();
    s.version = nextVersion;
    s.spawned_at = now;
    s.last_spawn_at = now;
    s.discovered_context_window = contextWindow;
    s.session_chars_at_spawn = corpusResult.data.total_chars;
    s.estimated_total_tokens = corpusResult.data.estimated_tokens;
    s.tokens_remaining = contextWindow - corpusResult.data.estimated_tokens;
    s.daemon_pool = [{
      daemon_id: spawnResult.daemon_id,
      session_name: sessionName,
      session_dir: spawnResult.session_dir,
      status: "idle",
      query_count: 0,
      chars_in: 0,  // bootstrap chars tracked separately in session_chars_at_spawn
      chars_out: 0,
      last_synced_interaction_id: null,
      last_query_at: new Date().toISOString(),
      idle_timeout_ms: undefined,
      last_corpus_sync_hash: corpusResult.data.tree_hashes,
      pending_syncs: [],
    }];
    s.last_bootstrap_ack = {
      ok: loadResult.data.bootstrap_ack_ok,
      raw: loadResult.data.bootstrap_ack_raw,
      checked_at: new Date().toISOString(),
    };
    if (!loadResult.data.bootstrap_ack_ok) {
      s.status = "error";
      s.last_error = "Bootstrap ack validation failed";
    } else {
      s.status = "healthy";
      s.last_error = null;
    }
    return s;
  });

  if (!stateUpdateResult.ok) return stateUpdateResult as OracleResult<SpawnOracleResult>;

  // Update manifest live_source last_tree_hash to match what was just loaded into the daemon.
  // Without this, oracle_sync_corpus would see a stale manifest hash after a fresh spawn and
  // push the same content again as a "delta" — double-injecting files the daemon already has
  // from corpus bootstrap (BUG-1 revised fix, per Gemini review).
  try {
    const manifestForSync = await readManifest(oracleDir);
    if (manifestForSync.ok) {
      const treeHashes = corpusResult.data.tree_hashes;
      const hasUpdates = manifestForSync.data.live_sources.some(
        (ls) => treeHashes[ls.id] && treeHashes[ls.id] !== ls.last_tree_hash,
      );
      if (hasUpdates) {
        await writeManifest(oracleDir, {
          ...manifestForSync.data,
          live_sources: manifestForSync.data.live_sources.map((ls) =>
            treeHashes[ls.id] ? { ...ls, last_tree_hash: treeHashes[ls.id] } : ls,
          ),
        });
      }
    }
  } catch { /* non-fatal — sync will handle correction on next call */ }

  // Create .pythia-active marker
  const activeDir = join(projectRoot, ".pythia-active");
  const marker: PythiaActiveMarker = {
    oracle_name: input.name,
    oracle_dir: oracleDir,
    project_root: projectRoot,
    pool_members_active: 1,
    written_at: new Date().toISOString(),
  };
  try {
    await atomicWriteFile(
      join(activeDir, `${input.name}.json`),
      JSON.stringify(marker, null, 2) + "\n",
    );
  } catch {
    // Marker creation failure is non-fatal
  }

  return ok({
    oracle_name: input.name,
    version: nextVersion,
    pool: [{
      daemon_id: spawnResult.daemon_id,
      session_name: sessionName,
      status: "idle",
    }],
    resumed: spawnResult.resumed,
    corpus_files_loaded: corpusResult.data.file_count,
    tokens_remaining: stateUpdateResult.data.tokens_remaining,
  });
}

export async function spawnOracle(input: {
  name: string;
  reuse_existing?: boolean;
  force_reload?: boolean;
  auto_refresh?: boolean;
  force?: boolean;
  timeout_ms?: number;
}): Promise<OracleResult<SpawnOracleResult>> {
  const startedAt = Date.now();
  const audit = {
    files_loaded: 0,
    stale_file_count: 0,
  };

  let result: OracleResult<SpawnOracleResult>;
  try {
    result = await spawnOracleInternal(input, audit);
  } catch (err: unknown) {
    result = fail(
      "BOOTSTRAP_FAILED",
      `Unhandled spawn failure: ${(err as Error).message}`,
    );
  }

  if (
    !result.ok
    && result.error.code === "HASH_MISMATCH_BATCH"
    && Array.isArray((result.error.details as { stale_files?: unknown[] } | undefined)?.stale_files)
  ) {
    audit.stale_file_count = (result.error.details as { stale_files: unknown[] }).stale_files.length;
  }

  appendSpawnAuditLog({
    timestamp: new Date().toISOString(),
    oracle_name: input.name,
    outcome: result.ok ? "success" : "error",
    error_code: result.ok ? undefined : result.error.code,
    stale_file_count: audit.stale_file_count,
    files_loaded: audit.files_loaded,
    duration_ms: Date.now() - startedAt,
  });

  return result;
}

// ─── oracle_sync_corpus Handler (FEAT-002, FEAT-021) ────────────────────────

/** Return type of oracle_sync_corpus. */
export interface SyncCorpusResult {
  source_id: string | "all";
  files_synced: number;
  files_skipped: number;
  bytes_loaded: number;
  tree_hash: string | null;
  members_synced_immediately: number;
  members_queued: number;
}

/**
 * Resolve a single live source's files, compute hashes, and determine delta.
 */
async function resolveLiveSourceDelta(
  source: LiveSource,
  manifest: OracleManifest,
): Promise<OracleResult<{
  files: Array<{ path: string; relPath: string; content: string; hash: string; bytes: number }>;
  treeHash: string;
  fileHashes: Record<string, string>;
  isChanged: boolean;
  totalBytes: number;
}>> {
  const maxSyncBytes = source.max_sync_bytes ?? DEFAULT_MAX_SYNC_BYTES;
  const fileHashes: Record<string, string> = {};
  const files: Array<{ path: string; relPath: string; content: string; hash: string; bytes: number }> = [];
  let totalBytes = 0;

  // Resolve globs
  let resolvedPaths: string[] = [];
  for (const includePattern of source.include) {
    const fullPattern = join(source.root, includePattern);
    try {
      const matched = globSync(fullPattern, {
        exclude: (source.exclude ?? []).map((ex) =>
          ex.startsWith("/") ? ex : join(source.root, ex),
        ),
      });
      resolvedPaths.push(...matched);
    } catch {
      // Glob failure — skip pattern
    }
  }

  resolvedPaths = [...new Set(resolvedPaths)].sort();

  // Enforce max_files
  if (source.max_files && resolvedPaths.length > source.max_files) {
    return fail(
      "CORPUS_CAP_EXCEEDED",
      `Live source "${source.id}" has ${resolvedPaths.length} files, exceeds max_files (${source.max_files})`,
    );
  }

  // Read files, enforce max_sync_bytes
  for (const filePath of resolvedPaths) {
    try {
      const fileStat = await stat(filePath);
      if (fileStat.isDirectory()) continue;
    } catch {
      continue;
    }

    let content: string;
    try {
      content = await readFile(filePath, "utf-8");
    } catch {
      continue;
    }

    const fileBytes = Buffer.byteLength(content, "utf-8");
    if (totalBytes + fileBytes > maxSyncBytes) {
      return fail(
        "CORPUS_CAP_EXCEEDED",
        `Live source "${source.id}" exceeds max_sync_bytes (${maxSyncBytes}) at ${totalBytes + fileBytes} bytes`,
      );
    }
    totalBytes += fileBytes;

    const hash = sha256(content);
    const relPath = relative(source.root, filePath);
    fileHashes[relPath] = hash;
    files.push({ path: filePath, relPath, content, hash, bytes: fileBytes });
  }

  const treeHash = computeTreeHash(fileHashes);
  const isChanged = treeHash !== source.last_tree_hash;

  return ok({ files, treeHash, fileHashes, isChanged, totalBytes });
}

/**
 * Core oracle_sync_corpus logic.
 *
 * Resolves live source files, computes delta against last sync,
 * and dispatches changes to pool members based on their status.
 */
export async function syncCorpus(input: {
  name: string;
  source_id?: string;
}): Promise<OracleResult<SyncCorpusResult>> {
  const runtime = getGeminiRuntime();

  // Look up oracle
  const lookupResult = await lookupOracle(input.name);
  if (!lookupResult.ok) return lookupResult as OracleResult<SyncCorpusResult>;
  if (!lookupResult.data || lookupResult.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${input.name}" not found or decommissioned`);
  }

  const oracleDir = lookupResult.data.oracle_dir;

  // Read manifest and state
  const manifestResult = await readManifest(oracleDir);
  if (!manifestResult.ok) return manifestResult as OracleResult<SyncCorpusResult>;
  const manifest = manifestResult.data;

  const stateResult = await readState(oracleDir);
  if (!stateResult.ok) return stateResult as OracleResult<SyncCorpusResult>;

  // Determine which sources to sync
  const targetSources = input.source_id
    ? manifest.live_sources.filter((s) => s.id === input.source_id)
    : manifest.live_sources;

  if (input.source_id && targetSources.length === 0) {
    return fail("FILE_NOT_FOUND", `Live source "${input.source_id}" not found in manifest`);
  }

  if (targetSources.length === 0) {
    return ok({
      source_id: input.source_id ?? "all",
      files_synced: 0, files_skipped: 0, bytes_loaded: 0,
      tree_hash: null, members_synced_immediately: 0, members_queued: 0,
    });
  }

  let totalFilesSynced = 0;
  let totalFilesSkipped = 0;
  let totalBytesLoaded = 0;
  let lastTreeHash: string | null = null;
  let membersSyncedImmediately = 0;
  let membersQueued = 0;

  for (const source of targetSources) {
    // Resolve delta
    const deltaResult = await resolveLiveSourceDelta(source, manifest);
    if (!deltaResult.ok) return deltaResult as OracleResult<SyncCorpusResult>;

    const { files, treeHash, fileHashes, isChanged, totalBytes } = deltaResult.data;
    lastTreeHash = treeHash;

    if (!isChanged) {
      totalFilesSkipped += files.length;
      continue;
    }

    // Compute per-file delta against last known hashes
    const lastHashes = source.last_file_hashes ?? {};
    const changedFiles = files.filter((f) => lastHashes[f.relPath] !== f.hash);
    const newFiles = files.filter((f) => !(f.relPath in lastHashes));
    const deletedPaths = Object.keys(lastHashes).filter(
      (p) => !fileHashes[p],
    );

    // Build sync payload
    const payloadParts: string[] = [];
    for (const file of changedFiles) {
      payloadParts.push(`[Updated: ${file.relPath} | sha256: ${file.hash}]\n${file.content}`);
    }
    for (const file of newFiles) {
      if (!changedFiles.includes(file)) {
        payloadParts.push(`[New: ${file.relPath} | sha256: ${file.hash}]\n${file.content}`);
      }
    }
    if (deletedPaths.length > 0) {
      payloadParts.push(`[Deleted files: ${deletedPaths.join(", ")}]`);
    }

    if (payloadParts.length === 0) {
      totalFilesSkipped += files.length;
      continue;
    }

    const syncPayload = `[Updated source files for ${source.id}. Read and absorb:]\n\n${payloadParts.join("\n\n")}`;
    const syncPayloadBytes = Buffer.byteLength(syncPayload, "utf-8");

    totalFilesSynced += changedFiles.length + newFiles.filter((f) => !changedFiles.includes(f)).length;
    totalFilesSkipped += files.length - totalFilesSynced;
    totalBytesLoaded += syncPayloadBytes;

    // Dispatch to pool members
    const currentState = stateResult.data;
    let sourceSyncedImmediately = 0;
    let sourceQueued = 0;
    for (const member of currentState.daemon_pool) {
      if (member.status === "dismissed" || member.status === "dead") {
        continue; // Skip — they get current corpus on next spawn
      }

      if (member.status === "idle" && member.daemon_id) {
        // Inject immediately
        try {
          await runtime.askDaemon({
            daemon_id: member.daemon_id,
            question: syncPayload,
            timeout_ms: 120_000,
          });
          membersSyncedImmediately++;
          sourceSyncedImmediately++;
        } catch {
          // Injection failed — queue instead
          membersQueued++;
          sourceQueued++;
          await writeStateWithRetry(oracleDir, (s) => {
            const m = s.daemon_pool.find((p) => p.session_name === member.session_name);
            if (m) {
              m.pending_syncs.push({
                source_id: source.id,
                tree_hash: treeHash,
                payload_ref: syncPayload,
                queued_at: new Date().toISOString(),
              });
            }
            return s;
          });
        }
      } else if (member.status === "busy") {
        // Queue for later drain
        membersQueued++;
        sourceQueued++;
        await writeStateWithRetry(oracleDir, (s) => {
          const m = s.daemon_pool.find((p) => p.session_name === member.session_name);
          if (m) {
            m.pending_syncs.push({
              source_id: source.id,
              tree_hash: treeHash,
              payload_ref: syncPayload,
              queued_at: new Date().toISOString(),
            });
          }
          return s;
        });
      }
    }

    // Only persist sync metadata if at least one member received the delta.
    // If all pool members were dismissed/dead, the manifest hash must stay stale
    // so the next call (after spawn) re-detects the change and delivers it.
    if (sourceSyncedImmediately === 0 && sourceQueued === 0) {
      continue;
    }

    // Update synced members' last_corpus_sync_hash
    await writeStateWithRetry(oracleDir, (s) => {
      for (const m of s.daemon_pool) {
        if (m.status === "idle") {
          if (!m.last_corpus_sync_hash) m.last_corpus_sync_hash = {};
          m.last_corpus_sync_hash[source.id] = treeHash;
          // Clear matching pending_syncs
          m.pending_syncs = m.pending_syncs.filter((ps) => ps.source_id !== source.id);
        }
      }
      return s;
    });

    // Update manifest with sync metadata
    await writeManifest(oracleDir, {
      ...manifest,
      live_sources: manifest.live_sources.map((ls) =>
        ls.id === source.id
          ? { ...ls, last_sync_at: new Date().toISOString(), last_tree_hash: treeHash, last_file_hashes: fileHashes }
          : ls,
      ),
    });
  }

  return ok({
    source_id: input.source_id ?? "all",
    files_synced: totalFilesSynced,
    files_skipped: totalFilesSkipped,
    bytes_loaded: totalBytesLoaded,
    tree_hash: lastTreeHash,
    members_synced_immediately: membersSyncedImmediately,
    members_queued: membersQueued,
  });
}

/**
 * Drain all pending syncs for a pool member before routing a query.
 *
 * Called before ask_daemon dispatches a question to ensure the member
 * has the latest corpus state. Concatenates all queued payloads into
 * a single injection message.
 */
export async function drainPendingSyncs(
  oracleDir: string,
  memberSessionName: string,
  daemonId: string,
  runtime: OracleRuntimeBridge,
): Promise<OracleResult<{ drained: number }>> {
  const stateResult = await readState(oracleDir);
  if (!stateResult.ok) return stateResult as OracleResult<{ drained: number }>;

  const member = stateResult.data.daemon_pool.find(
    (m) => m.session_name === memberSessionName,
  );
  if (!member || member.pending_syncs.length === 0) {
    return ok({ drained: 0 });
  }

  // Concatenate all pending payloads
  const payloads = member.pending_syncs.map((ps) => ps.payload_ref);
  const combinedPayload = payloads.join("\n\n---\n\n");

  // Inject combined payload
  try {
    await runtime.askDaemon({
      daemon_id: daemonId,
      question: combinedPayload,
      timeout_ms: 120_000,
    });
  } catch (err: unknown) {
    return fail(
      "IO_ERROR",
      `Failed to drain pending syncs for ${memberSessionName}: ${(err as Error).message}`,
    );
  }

  const drainedCount = member.pending_syncs.length;

  // Update state: clear pending_syncs, update last_corpus_sync_hash
  await writeStateWithRetry(oracleDir, (s) => {
    const m = s.daemon_pool.find((p) => p.session_name === memberSessionName);
    if (m) {
      for (const ps of m.pending_syncs) {
        if (!m.last_corpus_sync_hash) m.last_corpus_sync_hash = {};
        m.last_corpus_sync_hash[ps.source_id] = ps.tree_hash;
      }
      m.pending_syncs = [];
    }
    return s;
  });

  return ok({ drained: drainedCount });
}

// ─── Pressure Check (FEAT-003, FEAT-020) ────────────────────────────────────

interface PressureCheckResult {
  tokens_remaining: number;
  estimated_total_tokens: number;
  estimated_cluster_tokens: number;
  status: OracleStatus;
  recommendation: OracleRecommendation;
  pool_member_count: number;
  highest_pressure_member: string;
}

/**
 * Compute context pressure for a running oracle pool.
 *
 * Uses the absolute headroom model: checkpoint threshold is a fixed token
 * budget from the top of the context window, not a percentage.
 *
 * MAX aggregation across pool members drives the checkpoint decision —
 * the heaviest-queried daemon determines when we must checkpoint.
 * SUM (cluster_tokens) is provided for observability only.
 */
async function pressureCheck(
  params: { name: string },
): Promise<OracleResult<PressureCheckResult>> {
  // 1. Look up registry entry
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<PressureCheckResult>;
  if (!lookup.data || lookup.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found or decommissioned`);
  }
  const { oracle_dir } = lookup.data;

  // 2. Read manifest (for checkpoint_headroom_tokens)
  const manifestResult = await readManifest(oracle_dir);
  if (!manifestResult.ok) return manifestResult;
  const manifest = manifestResult.data;

  // 3. Read state
  const stateResult = await readState(oracle_dir);
  if (!stateResult.ok) return stateResult;
  const state = stateResult.data;

  // 4. Filter active pool members
  const activeMembers = state.daemon_pool.filter(
    (m) => m.status !== "dismissed" && m.status !== "dead",
  );
  if (activeMembers.length === 0) {
    return {
      ok: false,
      error: {
        code: "PRESSURE_UNAVAILABLE" as OracleErrorCode,
        message: "No active pool members — oracle has no running daemons",
        retryable: false,
      },
    };
  }

  // 5. discovered_context_window must be set before pressure can be computed
  if (state.discovered_context_window === null) {
    return {
      ok: false,
      error: {
        code: "PRESSURE_UNAVAILABLE" as OracleErrorCode,
        message: "Context window not yet discovered; oracle must be fully spawned first",
        retryable: false,
      },
    };
  }

  // 6. Per-member token estimates
  //    Each member has seen: session_chars_at_spawn (bootstrap) + its own chars_in + chars_out
  const sessionCharsAtSpawn = state.session_chars_at_spawn ?? 0;
  const cpt = state.chars_per_token_estimate;
  if (!cpt || cpt <= 0) {
    return {
      ok: false,
      error: {
        code: "STATE_INVALID" as OracleErrorCode,
        message: `chars_per_token_estimate is ${cpt} — cannot compute pressure`,
        retryable: false,
      },
    };
  }
  const memberTokens = activeMembers.map(
    (m) => (sessionCharsAtSpawn + m.chars_in + m.chars_out) / cpt,
  );

  // MAX drives checkpoint; SUM for observability
  const estimated_total_tokens = Math.max(...memberTokens);
  const estimated_cluster_tokens = memberTokens.reduce((a, b) => a + b, 0);
  const tokens_remaining = state.discovered_context_window - estimated_total_tokens;

  // 7. Absolute headroom status transitions
  const headroom = manifest.checkpoint_headroom_tokens;
  let newStatus: OracleStatus;
  let recommendation: OracleRecommendation;

  if (tokens_remaining > headroom) {
    newStatus = "healthy";
    recommendation = "healthy";
  } else if (tokens_remaining >= headroom / 2) {
    newStatus = "warning";
    recommendation = "checkpoint_soon";
  } else {
    newStatus = "critical";
    recommendation = "checkpoint_now";
  }

  // 8. Identify highest-pressure pool member (max tokens used = min remaining)
  const maxIdx = memberTokens.indexOf(estimated_total_tokens);
  const highest_pressure_member = activeMembers[maxIdx].session_name;

  // 9. Persist updated pressure metrics to state
  const writeResult = await writeStateWithRetry(oracle_dir, (s) => ({
    ...s,
    estimated_total_tokens: Math.round(estimated_total_tokens),
    estimated_cluster_tokens: Math.round(estimated_cluster_tokens),
    tokens_remaining: Math.round(tokens_remaining),
    status: newStatus,
    updated_at: new Date().toISOString(),
  }));
  if (!writeResult.ok) return writeResult;

  return {
    ok: true,
    data: {
      tokens_remaining: Math.round(tokens_remaining),
      estimated_total_tokens: Math.round(estimated_total_tokens),
      estimated_cluster_tokens: Math.round(estimated_cluster_tokens),
      status: newStatus,
      recommendation,
      pool_member_count: activeMembers.length,
      highest_pressure_member,
    },
  };
}

// ─── Log Learning (FEAT-005, FEAT-023) ──────────────────────────────────────

const BATCH_MAX_ENTRIES = 10;
const BATCH_MAX_BYTES = 256 * 1024; // 256 KB
const BATCH_DEBOUNCE_MS = 30_000;   // 30 seconds

const VALID_INTERACTION_TYPES = new Set<string>([
  "consultation", "feedback", "sync_event", "session_note",
]);

interface LearningsBatch {
  count: number;
  bytes: number;
  filePath: string;
  projectRoot: string;
  timer: ReturnType<typeof setTimeout> | null;
}

const _learningsBatch = new Map<string, LearningsBatch>(); // keyed by oracle_name
let _shutdownHookRegistered = false;

interface LogLearningParams {
  name: string;
  question?: string;
  counsel?: string;
  decision?: string | null;
  type?: string;
  interaction_scope?: string;
  quality_signal?: 1 | 2 | 3 | 4 | 5 | null;
  ion_delegated?: boolean;
  ion_query?: string;
  ion_response?: string;
  references?: string;
  implemented?: boolean;
  outcome?: string;
  divergence?: string;
  force?: boolean;
}

interface LogLearningResult {
  entry_id: string;
  file_path: string;
  version: number;
  committed: boolean;
}

/** Synchronous git commit — used on process shutdown where async is unavailable. */
function flushBatchSync(name: string, batch: LearningsBatch): void {
  if (batch.count === 0) return;
  const count = batch.count;
  if (batch.timer) { clearTimeout(batch.timer); batch.timer = null; }
  batch.count = 0;
  batch.bytes = 0;
  try {
    execSync(
      `git add "${batch.filePath}" && git commit -m "oracle(${name}): log ${count} interactions"`,
      { cwd: batch.projectRoot, stdio: "pipe" },
    );
  } catch { /* JSONL is already safe on disk; git failure is non-fatal */ }
}

/** Register SIGTERM/SIGINT/exit flush once for the whole process lifetime. */
function registerShutdownHook(): void {
  if (_shutdownHookRegistered) return;
  _shutdownHookRegistered = true;
  const flush = () => {
    for (const [name, batch] of _learningsBatch) {
      flushBatchSync(name, batch);
    }
  };
  process.on("exit", flush);
  process.on("SIGTERM", () => { flush(); process.exit(0); });
  process.on("SIGINT",  () => { flush(); process.exit(0); });
}

/** Async batch flush — used during normal operation. */
async function flushBatchAsync(name: string, batch: LearningsBatch): Promise<boolean> {
  if (batch.count === 0) return false;
  const count = batch.count;
  if (batch.timer) { clearTimeout(batch.timer); batch.timer = null; }
  batch.count = 0;
  batch.bytes = 0;
  try {
    // execSync is fine here — commits are infrequent and latency doesn't matter
    execSync(
      `git add "${batch.filePath}" && git commit -m "oracle(${name}): log ${count} interactions"`,
      { cwd: batch.projectRoot, stdio: "pipe" },
    );
    return true;
  } catch {
    return false;
  }
}

/**
 * Append an interaction entry to the learnings JSONL file for the named oracle.
 * Immediately safe on disk; git commit is batched by count, byte size, debounce, or force flag.
 */
async function logLearning(
  params: LogLearningParams,
): Promise<OracleResult<LogLearningResult>> {
  // 1. Validate interaction type
  const interactionType = (params.type ?? "consultation") as InteractionType;
  if (!VALID_INTERACTION_TYPES.has(interactionType)) {
    return fail(
      "MANIFEST_INVALID",
      `Invalid interaction type "${params.type}". Valid: ${[...VALID_INTERACTION_TYPES].join(", ")}`,
    );
  }

  // 2. Validate ion_delegated requirements
  if (params.ion_delegated) {
    if (!params.ion_query?.trim()) {
      return fail("MANIFEST_INVALID", "ion_delegated=true requires a non-empty ion_query");
    }
    if (!params.ion_response?.trim()) {
      return fail("MANIFEST_INVALID", "ion_delegated=true requires a non-empty ion_response");
    }
  }

  // 3. Look up registry
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<LogLearningResult>;
  if (!lookup.data || lookup.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found or decommissioned`);
  }
  const { oracle_dir, project_root } = lookup.data;

  // 4. Read current state
  const stateResult = await readState(oracle_dir);
  if (!stateResult.ok) return stateResult as OracleResult<LogLearningResult>;
  const state = stateResult.data;

  // 5. Build entry ID — monotonic seq counter from state
  const isFeedback = interactionType === "feedback";
  const seq = state.next_seq;
  const seqPadded = String(seq).padStart(3, "0");
  const entry_id = isFeedback
    ? `v${state.version}-q${seqPadded}-fb`
    : `v${state.version}-q${seqPadded}`;

  // 6. Learnings file path
  const learningsDir = join(oracle_dir, "learnings");
  const filePath = join(learningsDir, `v${state.version}-interactions.jsonl`);

  // 7. Snapshot pressure context from state (best-effort; null → 0)
  const tokens_remaining_at_query = state.tokens_remaining ?? 0;
  const activeMembers = state.daemon_pool.filter(
    (m) => m.status !== "dismissed" && m.status !== "dead",
  );
  // MAX matches pressureCheck semantics — reflects the highest-pressure member
  const chars_in_at_query = activeMembers.length > 0
    ? Math.max(...activeMembers.map((m) => m.chars_in))
    : 0;

  // 8. Build InteractionEntry — only include defined optional fields
  const counsel = params.counsel;
  const entry: InteractionEntry = {
    id: entry_id,
    seq,
    entry_schema_version: CURRENT_ENTRY_SCHEMA_VERSION,
    type: interactionType,
    oracle_name: params.name,
    version: state.version,
    query_count: state.query_count + 1,
    timestamp: new Date().toISOString(),
    trace_id: randomUUID(),
    span_id: randomUUID(),
    parent_span_id: null,
    tokens_remaining_at_query,
    chars_in_at_query,
    ...(params.interaction_scope !== undefined && {
      interaction_scope: params.interaction_scope as InteractionScope,
    }),
    ...(params.question !== undefined && { question: params.question }),
    ...(params.ion_delegated !== undefined && { ion_delegated: params.ion_delegated }),
    ...(params.ion_query !== undefined && { ion_query: params.ion_query }),
    ...(params.ion_response !== undefined && { ion_response: params.ion_response }),
    ...(counsel !== undefined && {
      counsel,
      counsel_sha256: createHash("sha256").update(counsel).digest("hex"),
    }),
    ...(params.decision !== undefined && { decision: params.decision }),
    ...(params.quality_signal !== undefined && { quality_signal: params.quality_signal }),
    ...(params.references !== undefined && { references: params.references }),
    ...(params.implemented !== undefined && { implemented: params.implemented }),
    ...(params.outcome !== undefined && { outcome: params.outcome }),
    ...(params.divergence !== undefined && { divergence: params.divergence }),
  };

  // 9. Reserve seq in state first — prevents duplicate entry_ids if two calls race.
  //    If state write fails, nothing has been appended yet (clean failure).
  await mkdir(learningsDir, { recursive: true });
  const writeResult = await writeStateWithRetry(oracle_dir, (s) => ({
    ...s,
    next_seq: s.next_seq + 1,
    query_count: s.query_count + 1,
    updated_at: new Date().toISOString(),
  }));
  if (!writeResult.ok) return writeResult as OracleResult<LogLearningResult>;

  // 10. Append JSONL — seq is now durably reserved; a failed append wastes seq N (acceptable)
  const line = JSON.stringify(entry) + "\n";
  await writeFile(filePath, line, { flag: "a" });

  // 11. Batch commit logic
  registerShutdownHook();
  let batch = _learningsBatch.get(params.name);
  if (!batch) {
    batch = {
      count: 0,
      bytes: 0,
      filePath,
      projectRoot: project_root,
      timer: null,
    };
    _learningsBatch.set(params.name, batch);
  }
  batch.count++;
  batch.bytes += line.length;

  let committed = false;
  if (params.force || batch.count >= BATCH_MAX_ENTRIES || batch.bytes >= BATCH_MAX_BYTES) {
    committed = await flushBatchAsync(params.name, batch);
  } else {
    // Reset debounce window
    if (batch.timer) clearTimeout(batch.timer);
    batch.timer = setTimeout(() => {
      void flushBatchAsync(params.name, batch!);
    }, BATCH_DEBOUNCE_MS);
  }

  return {
    ok: true,
    data: { entry_id, file_path: filePath, version: state.version, committed },
  };
}

// ─── Checkpoint (FEAT-004) ───────────────────────────────────────────────────

const CHECKPOINT_PROMPT = `Write your checkpoint inside <checkpoint> tags. Cover:
(1) All static corpus files loaded and key findings from each.
    DO NOT summarize source code -- summarize the architectural decisions
    and constraints that the code expresses.
(2) Every question asked this session and your answer summary
(3) Every architectural/strategic decision made based on your counsel
(4) Your top 10 cross-cutting insights from the full corpus
(5) Gaps, contradictions, or uncertainties detected
Be exhaustive -- this is your legacy for your successor.`;

interface CheckpointResult {
  checkpoint_path: string;
  bytes: number;
  sha256: string;
  version: number;
}

/**
 * Extract checkpoint content from a Gemini response using cascading fallback:
 * 1. <checkpoint>...</checkpoint> tags (ideal)
 * 2. Scrub common LLM wrapper patterns (code fences, preambles)
 * 3. Full response with a warning logged
 */
function extractCheckpointContent(
  response: string,
): { content: string; warn?: string } {
  // 1. Try <checkpoint>...</checkpoint> tags
  const tagMatch = response.match(/<checkpoint>([\s\S]*?)<\/checkpoint>/i);
  if (tagMatch) {
    return { content: tagMatch[1].trim() };
  }

  // 2. Scrub common LLM wrapper patterns
  let scrubbed = response
    .replace(/^```(?:markdown)?\s*/i, "")
    .replace(/\s*```\s*$/, "")
    .replace(/^(?:here (?:is|'s)(?: my)? checkpoint:?\s*|I(?:'ll| will) write my checkpoint:?\s*)/i, "")
    .trim();

  if (scrubbed.length > 200) {
    return {
      content: scrubbed,
      warn: "Checkpoint tags not found in response; used scrubbed fallback",
    };
  }

  // 3. Last resort: full response
  return {
    content: response.trim(),
    warn: "Checkpoint tags not found; using full response as checkpoint content",
  };
}

async function runCheckpoint(params: {
  name: string;
  timeout_ms?: number;
  commit?: boolean;
}): Promise<OracleResult<CheckpointResult>> {
  const timeoutMs = params.timeout_ms ?? 600_000;
  const doCommit = params.commit ?? true;
  const runtime = getGeminiRuntime();

  // 1. Look up registry
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<CheckpointResult>;
  if (!lookup.data || lookup.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found or decommissioned`);
  }
  const { oracle_dir, project_root } = lookup.data;

  // 2. Read manifest for headroom threshold
  const manifestResult = await readManifest(oracle_dir);
  if (!manifestResult.ok) return manifestResult as OracleResult<CheckpointResult>;
  const manifest = manifestResult.data;

  // 3. Acquire operation lock
  const lockResult = await acquireOperationLock(oracle_dir, "checkpoint");
  if (!lockResult.ok) return lockResult as OracleResult<CheckpointResult>;
  const { lockToken } = lockResult.data;

  // 4. Start heartbeat (extend every 60s, TTL 10min)
  const heartbeat = startLockHeartbeat({
    oracleDir: oracle_dir,
    operation: "checkpoint",
    lockToken,
    extendEveryMs: 60_000,
    ttlMs: 600_000,
  });

  try {
    // 5. Read state and verify pre-conditions
    const stateResult = await readState(oracle_dir);
    if (!stateResult.ok) return stateResult as OracleResult<CheckpointResult>;
    const state = stateResult.data;

    // Too late to checkpoint — not enough tokens remaining for Gemini to respond
    const minRequired = manifest.checkpoint_headroom_tokens / 4;
    if (state.tokens_remaining !== null && state.tokens_remaining < minRequired) {
      return fail(
        "CHECKPOINT_FAILED",
        "Too late for checkpoint -- use oracle_salvage instead",
      );
    }

    // Find an active daemon to ask
    const activeMember = state.daemon_pool.find(
      (m) => m.status !== "dismissed" && m.status !== "dead" && m.daemon_id !== null,
    );
    if (!activeMember || !activeMember.daemon_id) {
      return fail("DAEMON_NOT_FOUND", `Oracle "${params.name}" has no active daemon in pool`);
    }

    // 6. Send checkpoint prompt to Pythia
    let askResult: { text: string; chars_in: number; chars_out: number };
    try {
      askResult = await runtime.askDaemon({
        daemon_id: activeMember.daemon_id,
        question: CHECKPOINT_PROMPT,
        timeout_ms: timeoutMs,
      });
    } catch (err: unknown) {
      // Gemini context-limit or fatal error
      const msg = (err as Error).message ?? String(err);
      await writeStateWithRetry(oracle_dir, (s) => ({
        ...s,
        status: "error" as const,
        last_error: `Checkpoint failed: ${msg}`,
        updated_at: new Date().toISOString(),
      }));
      return fail("CHECKPOINT_FAILED", `Gemini error during checkpoint: ${msg}`);
    }

    // 7. Extract checkpoint content (cascading)
    const { content, warn } = extractCheckpointContent(askResult.text);

    // 8. Persist checkpoint file
    const checkpointsDir = join(oracle_dir, "checkpoints");
    await mkdir(checkpointsDir, { recursive: true });
    const checkpointPath = join(checkpointsDir, `v${state.version}-checkpoint.md`);
    await atomicWriteFile(checkpointPath, content);

    const bytes = Buffer.byteLength(content, "utf8");
    const sha256 = createHash("sha256").update(content).digest("hex");

    // 9. Update manifest: add checkpoint to static_entries
    const newEntry: StaticEntry = {
      path: checkpointPath,
      role: "checkpoint",
      required: true,
      sha256,
      added_at: new Date().toISOString(),
      priority: 0,
    };
    const updatedManifest: OracleManifest = {
      ...manifest,
      static_entries: [
        ...manifest.static_entries.filter((e) => e.role !== "checkpoint"),
        newEntry,
      ],
    };
    const manifestWriteResult = await writeManifest(oracle_dir, updatedManifest);
    if (!manifestWriteResult.ok) return manifestWriteResult as OracleResult<CheckpointResult>;

    // 10. Update state: last_checkpoint_path
    const warnings: string[] = warn ? [warn] : [];
    const stateWriteResult = await writeStateWithRetry(oracle_dir, (s) => ({
      ...s,
      last_checkpoint_path: checkpointPath,
      updated_at: new Date().toISOString(),
    }));
    if (!stateWriteResult.ok) return stateWriteResult as OracleResult<CheckpointResult>;

    // 11. Git commit if requested
    if (doCommit) {
      const manifestPath = join(oracle_dir, "manifest.json");
      try {
        execSync(
          `git add "${checkpointPath}" "${manifestPath}" && ` +
          `git commit -m "oracle(${params.name}): v${state.version} checkpoint (${state.query_count} consultations)"`,
          { cwd: project_root, stdio: "pipe" },
        );
      } catch { /* git failure is non-fatal — checkpoint is safe on disk */ }
    }

    const result: OracleResult<CheckpointResult> = {
      ok: true,
      data: { checkpoint_path: checkpointPath, bytes, sha256, version: state.version },
      ...(warnings.length > 0 && { warnings }),
    };
    return result;

  } finally {
    // Always clean up lock — even on error
    await heartbeat.stop();
    await releaseLock(oracle_dir, lockToken);
  }
}

// ─── Salvage (FEAT-008) ──────────────────────────────────────────────────────

const SALVAGE_SYNTHESIS_PROMPT = (name: string, version: number) =>
  `You are a senior AI assistant synthesizing a knowledge checkpoint from a completed work session.
Below is the complete interaction log for Oracle "${name}" Generation ${version}.
Based on these interactions, write a comprehensive checkpoint covering:
(1) All architectural and strategic decisions made during this session
(2) Key insights and findings from each major topic area
(3) Open questions and areas of uncertainty
(4) What the next generation should know immediately upon starting
Write your checkpoint inside <checkpoint> tags. Be exhaustive — this is the inherited wisdom for the next generation.`;

interface SalvageResult {
  checkpoint_path: string;
  source: "salvage";
  entries_processed: number;
}

/**
 * Synthesize a checkpoint from the interactions JSONL log using a fresh
 * single-shot Gemini call (NOT the oracle daemon — it may be dead or exhausted).
 * Falls back to stub checkpoints when no interactions exist.
 */
async function runSalvage(
  params: { name: string },
): Promise<OracleResult<SalvageResult>> {
  // 1. Look up registry
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<SalvageResult>;
  if (!lookup.data || lookup.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found or decommissioned`);
  }
  const { oracle_dir, project_root } = lookup.data;

  // 2. Read manifest + state
  const manifestResult = await readManifest(oracle_dir);
  if (!manifestResult.ok) return manifestResult as OracleResult<SalvageResult>;
  const manifest = manifestResult.data;

  const stateResult = await readState(oracle_dir);
  if (!stateResult.ok) return stateResult as OracleResult<SalvageResult>;
  const state = stateResult.data;

  const version = state.version;
  const checkpointsDir = join(oracle_dir, "checkpoints");
  await mkdir(checkpointsDir, { recursive: true });
  const checkpointPath = join(checkpointsDir, `v${version}-checkpoint.md`);

  // 3. Read interactions log
  const interactionsPath = join(oracle_dir, "learnings", `v${version}-interactions.jsonl`);
  let interactionsContent = "";
  let entriesProcessed = 0;
  try {
    interactionsContent = await readFile(interactionsPath, "utf-8");
    entriesProcessed = interactionsContent.trim().split("\n").filter(Boolean).length;
  } catch { /* file may not exist yet */ }

  let checkpointContent: string;

  if (entriesProcessed > 0) {
    // 4a. Synthesize checkpoint from interactions via fresh Gemini call
    const prompt = SALVAGE_SYNTHESIS_PROMPT(params.name, version);
    const synthResult = await executeWithFallback({
      baseArgs: ["-p", "", "--output-format", "text"],
      stdin: `${prompt}\n\nINTERACTIONS:\n${interactionsContent}`,
      cwd: project_root,
      timeout_ms: 300_000,
    });
    if (!synthResult.success) {
      return fail("CHECKPOINT_FAILED", `Salvage synthesis failed: ${synthResult.timed_out ? "timeout" : (synthResult.stderr ?? "unknown error")}`);
    }
    const { content } = extractCheckpointContent(synthResult.stdout ?? "");
    checkpointContent = content;
  } else {
    // 4b. No interactions — stub checkpoint from prior generation
    const prevCheckpointPath = join(checkpointsDir, `v${version - 1}-checkpoint.md`);
    let prevContent = "";
    try {
      prevContent = await readFile(prevCheckpointPath, "utf-8");
    } catch { /* no prior checkpoint */ }

    if (prevContent) {
      checkpointContent =
        `No new architectural decisions were recorded during Generation ${version}. ` +
        `All wisdom from Generation ${version - 1} remains current.\n\n` +
        `--- Inherited from v${version - 1} ---\n\n${prevContent}`;
    } else {
      checkpointContent =
        `Generation ${version} had no consultations and no prior checkpoint to inherit.`;
    }
  }

  // 5. Save checkpoint
  await atomicWriteFile(checkpointPath, checkpointContent);
  // Compute sha256 from the on-disk file after write, not from the in-memory string.
  // atomicWriteFile uses a tmp→rename approach that may apply newline normalization or
  // encoding differences — hashing the persisted bytes ensures manifest sha256 matches
  // what readFile returns later (BUG-4 fix).
  const onDiskContent = await readFile(checkpointPath, "utf-8");
  const sha256 = createHash("sha256").update(onDiskContent).digest("hex");

  // 6. Update manifest static_entries
  const newEntry: StaticEntry = {
    path: checkpointPath,
    role: "checkpoint",
    required: true,
    sha256,
    added_at: new Date().toISOString(),
    priority: 0,
  };
  const updatedManifest: OracleManifest = {
    ...manifest,
    static_entries: [
      ...manifest.static_entries.filter((e) => e.role !== "checkpoint"),
      newEntry,
    ],
  };
  const manifestWriteResult = await writeManifest(oracle_dir, updatedManifest);
  if (!manifestWriteResult.ok) return manifestWriteResult as OracleResult<SalvageResult>;

  return { ok: true, data: { checkpoint_path: checkpointPath, source: "salvage", entries_processed: entriesProcessed } };
}

// ─── Reconstitute (FEAT-009) ─────────────────────────────────────────────────

const DRAIN_POLL_MS = 500;
const DRAIN_MAX_MS = 5 * 60 * 1000; // 5-minute safety valve (decision #45)

interface ReconstituteResult {
  previous_version: number;
  new_version: number;
  new_daemon_id: string;
  checkpoint_path: string | null;
  loaded_artifacts: {
    static_files: number;
    live_source_files: number;
    total_chars: number;
  };
}

/**
 * Atomic generation transition for a Pythia oracle.
 *
 * Full cutover model (decision #44, #45):
 * 1. Lock + set status to "preserving" (gates new queries)
 * 2. Drain in-flight queries (bounded by longest in-flight; 5-min safety valve)
 * 3. Checkpoint phase: runCheckpoint → fallback to runSalvage → abort if both fail
 * 4. Soft-dismiss all pool members (session data preserved on disk)
 * 5. Increment version in manifest + state
 * 6. Resolve corpus with reconstitute_sync_mode filtering per live_source
 * 7. Spawn v(N+1) with inherited_wisdom from checkpoint in preamble
 * 8. Load corpus into new daemon
 * 9. Write new state: version N+1, fresh pool, reset counters
 */
async function runReconstitute(params: {
  name: string;
  checkpoint_first?: boolean;
  dismiss_old?: boolean;
}): Promise<OracleResult<ReconstituteResult>> {
  const checkpointFirst = params.checkpoint_first ?? true;
  const runtime = getGeminiRuntime();

  // 1. Registry lookup
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<ReconstituteResult>;
  if (!lookup.data || lookup.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found or decommissioned`);
  }
  const { oracle_dir, project_root } = lookup.data;

  // 2. Acquire operation lock
  const lockResult = await acquireOperationLock(oracle_dir, "reconstitute");
  if (!lockResult.ok) return lockResult as OracleResult<ReconstituteResult>;
  const { lockToken } = lockResult.data;

  const heartbeat = startLockHeartbeat({
    oracleDir: oracle_dir,
    operation: "reconstitute",
    lockToken,
    extendEveryMs: 60_000,
    ttlMs: 600_000,
  });

  try {
    // 3. Read state and manifest
    const stateResult = await readState(oracle_dir);
    if (!stateResult.ok) return stateResult as OracleResult<ReconstituteResult>;
    const oldState = stateResult.data;

    const manifestResult = await readManifest(oracle_dir);
    if (!manifestResult.ok) return manifestResult as OracleResult<ReconstituteResult>;
    const manifest = manifestResult.data;

    const previousVersion = oldState.version;

    // 4. Set status to "preserving" — gates new queries (decision #45)
    await writeStateWithRetry(oracle_dir, (s) => ({
      ...s,
      status: "preserving" as const,
      updated_at: new Date().toISOString(),
    }));

    // 5. Drain phase — wait for all busy members to complete
    const drainDeadline = Date.now() + DRAIN_MAX_MS;
    while (true) {
      const s = await readState(oracle_dir);
      if (!s.ok) return s as OracleResult<ReconstituteResult>;
      const busyCount = s.data.daemon_pool.filter((m) => m.status === "busy").length;
      if (busyCount === 0) break;
      if (Date.now() >= drainDeadline) {
        await writeStateWithRetry(oracle_dir, (st) => ({
          ...st,
          status: "error" as const,
          last_error: "Drain phase timed out after 5 minutes during reconstitution",
          updated_at: new Date().toISOString(),
        }));
        return fail(
          "RECONSTITUTE_FAILED",
          "Drain phase timed out — in-flight queries did not complete within 5 minutes",
        );
      }
      await sleep(DRAIN_POLL_MS);
    }

    // 6. Checkpoint phase with cascading fallback (decision #44)
    let checkpointPath: string | null = null;
    if (checkpointFirst) {
      const ckptResult = await runCheckpoint({
        name: params.name,
        commit: true,
      });
      if (ckptResult.ok) {
        checkpointPath = ckptResult.data.checkpoint_path;
      } else {
        // Checkpoint failed — fallback to salvage
        const salvageResult = await runSalvage({ name: params.name });
        if (salvageResult.ok) {
          checkpointPath = salvageResult.data.checkpoint_path;
        } else {
          // Both failed — hard abort, leave v(N) alive
          await writeStateWithRetry(oracle_dir, (s) => ({
            ...s,
            status: "error" as const,
            last_error: `Reconstitution aborted: checkpoint failed (${ckptResult.error.message}), salvage also failed (${salvageResult.error.message})`,
            updated_at: new Date().toISOString(),
          }));
          return fail(
            "RECONSTITUTE_FAILED",
            `Both checkpoint and salvage failed during reconstitution. ` +
            `Oracle v${previousVersion} is still alive. ` +
            `Checkpoint error: ${ckptResult.error.message}`,
          );
        }
      }
    }

    // 7. Soft-dismiss all pool members (session data preserved on disk)
    const refreshedState = await readState(oracle_dir);
    const currentState = refreshedState.ok ? refreshedState.data : oldState;
    for (const member of currentState.daemon_pool) {
      if (member.daemon_id && member.status !== "dismissed" && member.status !== "dead") {
        try {
          await runtime.dismissDaemon({ daemon_id: member.daemon_id, hard: false });
        } catch { /* best-effort — session may already be gone */ }
      }
    }

    // 8. Increment version in manifest.
    // Re-read manifest from disk first — checkpoint_first may have run runSalvage which
    // rewrites the checkpoint file and updates the manifest. Using the in-memory `manifest`
    // copy from function entry would overwrite those updates with stale data (BUG-3 fix).
    const freshManifestResult = await readManifest(oracle_dir);
    const currentManifest = freshManifestResult.ok ? freshManifestResult.data : manifest;
    const newVersion = previousVersion + 1;
    const updatedManifest: OracleManifest = {
      ...currentManifest,
      version: newVersion,
    };
    // Ensure checkpoint is in static_entries (may already be there from runCheckpoint/salvage)
    if (checkpointPath) {
      const alreadyInManifest = updatedManifest.static_entries.some(
        (e) => e.path === checkpointPath && e.role === "checkpoint",
      );
      if (!alreadyInManifest) {
        let sha256 = "";
        try {
          const content = await readFile(checkpointPath, "utf-8");
          sha256 = createHash("sha256").update(content).digest("hex");
        } catch { /* file may not exist */ }
        updatedManifest.static_entries = [
          ...updatedManifest.static_entries.filter((e) => e.role !== "checkpoint"),
          { path: checkpointPath, role: "checkpoint", required: true, sha256, added_at: new Date().toISOString(), priority: 0 },
        ];
      }
      // Do NOT add vN-interactions.jsonl — checkpoint supersedes learnings (spec §3.2e)
    }
    await writeManifest(oracle_dir, updatedManifest);

    // 9. Read checkpoint content for inherited_wisdom preamble
    let inheritedWisdom = "";
    if (checkpointPath) {
      try { inheritedWisdom = await readFile(checkpointPath, "utf-8"); } catch { /* skip */ }
    }

    // 10. Resolve corpus with reconstitute_sync_mode filtering
    const corpusResult = await resolveCorpusForSpawn(oracle_dir);
    if (!corpusResult.ok) return corpusResult as OracleResult<ReconstituteResult>;
    const fullCorpus = corpusResult.data;

    // Filter live source entries: hash_gated_delta skips unchanged sources
    const filteredEntries = fullCorpus.entries.filter((entry) => {
      if (entry.source_type === "static") return true;
      const source = manifest.live_sources.find((s) => s.id === entry.source_id);
      if (!source) return true;
      const mode = source.reconstitute_sync_mode ?? "hash_gated_delta";
      if (mode === "full_rescan") return true;
      // hash_gated_delta: skip if tree hash unchanged
      const currentHash = fullCorpus.tree_hashes[source.id ?? ""];
      return !source.last_tree_hash || currentHash !== source.last_tree_hash;
    });

    const filteredCorpus = {
      ...fullCorpus,
      entries: filteredEntries,
      file_count: filteredEntries.length,
      total_chars: filteredEntries.reduce((sum, e) => sum + e.bytes, 0),
      total_bytes: filteredEntries.reduce((sum, e) => sum + e.bytes, 0),
    };

    // 11. Build preamble with inherited_wisdom
    const currentModel = getCurrentModel();
    const contextWindow = discoverContextWindow(currentModel);
    const sessionName = `daemon-${params.name}-0`; // fresh session, not resuming

    const preamble = buildSpawnPreamble({
      oracleName: params.name,
      project: project_root,
      nextVersion: newVersion,
      inheritedWisdom,
    });

    // 12. Spawn fresh v(N+1) daemon
    let spawnResult: { daemon_id: string; resumed: boolean; session_dir: string };
    try {
      spawnResult = await runtime.spawnDaemon({
        session_name: sessionName,
        cwd: project_root,
        timeout_ms: 60_000,
      });
    } catch (err: unknown) {
      return fail("RECONSTITUTE_FAILED", `Failed to spawn v${newVersion} daemon: ${(err as Error).message}`);
    }

    // Send preamble
    try {
      await runtime.askDaemon({
        daemon_id: spawnResult.daemon_id,
        question: preamble,
        timeout_ms: 120_000,
      });
    } catch (err: unknown) {
      return fail("RECONSTITUTE_FAILED", `Failed to send preamble to v${newVersion}: ${(err as Error).message}`);
    }

    // 13. Load corpus into new daemon
    const loadResult = await loadResolvedCorpusIntoDaemon(
      spawnResult.daemon_id,
      filteredCorpus,
      runtime,
    );
    if (!loadResult.ok) return loadResult as OracleResult<ReconstituteResult>;

    // 14. Write new state: version N+1, fresh pool, reset counters
    const corpusChars = filteredCorpus.total_chars;
    const cpt = oldState.chars_per_token_estimate;
    const estimatedBootstrapTokens = cpt > 0 ? Math.round(corpusChars / cpt) : 0;
    const tokensRemaining = contextWindow !== null
      ? contextWindow - estimatedBootstrapTokens
      : null;

    const newPoolMember = {
      daemon_id: spawnResult.daemon_id,
      session_name: sessionName,
      session_dir: spawnResult.session_dir,
      status: "idle" as const,
      query_count: 0,
      chars_in: 0,
      chars_out: 0,
      last_synced_interaction_id: null,
      last_query_at: new Date().toISOString(),
      last_corpus_sync_hash: fullCorpus.tree_hashes,
      pending_syncs: [],
    };

    await writeStateWithRetry(oracle_dir, (s) => ({
      ...s,
      version: newVersion,
      spawned_at: new Date().toISOString(),
      discovered_context_window: contextWindow,
      daemon_pool: [newPoolMember],
      session_chars_at_spawn: corpusChars,
      estimated_total_tokens: estimatedBootstrapTokens,
      estimated_cluster_tokens: estimatedBootstrapTokens,
      tokens_remaining: tokensRemaining,
      query_count: 0,
      last_checkpoint_path: checkpointPath,
      status: "healthy" as const,
      lock_held_by: null,
      lock_expires_at: null,
      last_error: null,
      last_bootstrap_ack: loadResult.data.bootstrap_ack_ok
        ? { ok: true, raw: loadResult.data.bootstrap_ack_raw, checked_at: new Date().toISOString() }
        : null,
      next_seq: 1,
      generation_since_reground: 0,
      state_version: s.state_version,
      updated_at: new Date().toISOString(),
    }));

    // 15. Git commit
    try {
      const manifestPath = join(oracle_dir, "manifest.json");
      execSync(
        `git add "${manifestPath}" && ` +
        `git commit -m "oracle(${params.name}): reconstitute v${previousVersion} → v${newVersion}"`,
        { cwd: project_root, stdio: "pipe" },
      );
    } catch { /* non-fatal */ }

    const staticCount = filteredEntries.filter((e) => e.source_type === "static").length;
    const liveCount = filteredEntries.filter((e) => e.source_type === "live").length;

    return {
      ok: true,
      data: {
        previous_version: previousVersion,
        new_version: newVersion,
        new_daemon_id: spawnResult.daemon_id,
        checkpoint_path: checkpointPath,
        loaded_artifacts: {
          static_files: staticCount,
          live_source_files: liveCount,
          total_chars: corpusChars,
        },
      },
    };

  } finally {
    await heartbeat.stop();
    await releaseLock(oracle_dir, lockToken);
  }
}

// ─── Decommission (FEAT-011, FEAT-012, FEAT-013, FEAT-035) ──────────────────

/**
 * Minimal RFC 6238 TOTP verifier — pure Node.js crypto, no external dep.
 *
 * secret: base32-encoded TOTP secret (e.g. from pythia-auth enrollment)
 * code:   6-digit code from authenticator app
 * window: number of 30s steps to allow on either side (default: 1 = ±30s clock skew)
 */
function verifyTotp(secret: string, code: string, window = 1): boolean {
  // RFC 4648 base32 decode
  const BASE32_CHARS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  const cleaned = secret.toUpperCase().replace(/=+$/, "").replace(/\s/g, "");
  let bits = 0;
  let bitsCount = 0;
  const bytes: number[] = [];
  for (const ch of cleaned) {
    const val = BASE32_CHARS.indexOf(ch);
    if (val < 0) continue;
    bits = (bits << 5) | val;
    bitsCount += 5;
    if (bitsCount >= 8) {
      bytes.push((bits >> (bitsCount - 8)) & 0xff);
      bitsCount -= 8;
    }
  }
  const keyBuf = Buffer.from(bytes);

  const now = Math.floor(Date.now() / 1000);
  const step = 30;
  const T = Math.floor(now / step);

  for (let offset = -window; offset <= window; offset++) {
    const counter = T + offset;
    // Big-endian 8-byte counter buffer
    const counterBuf = Buffer.alloc(8);
    counterBuf.writeUInt32BE(Math.floor(counter / 0x100000000), 0);
    counterBuf.writeUInt32BE(counter >>> 0, 4);

    const hmacResult = createHmac("sha1", keyBuf).update(counterBuf).digest();

    const offset4 = hmacResult[hmacResult.length - 1] & 0x0f;
    const otp = (
      ((hmacResult[offset4]     & 0x7f) << 24) |
      ((hmacResult[offset4 + 1] & 0xff) << 16) |
      ((hmacResult[offset4 + 2] & 0xff) <<  8) |
       (hmacResult[offset4 + 3] & 0xff)
    ) % 1_000_000;

    if (otp.toString().padStart(6, "0") === code.trim()) return true;
  }
  return false;
}

/**
 * Read TOTP secret for oracle `name` from ~/.pythia/keys/<name>.totp (plaintext base32).
 * Production: pythia-auth handles enrollment and Keychain storage.
 * MCP server always reads this file (not Keychain) — Touch ID only on pythia-auth side.
 */
async function readTotpSecret(oracleName: string): Promise<string | null> {
  const keyPath = join(PYTHIA_KEYS_DIR, `${oracleName}.totp`);
  try {
    const raw = await readFile(keyPath, "utf-8");
    return raw.trim();
  } catch {
    return null;
  }
}

interface DecommissionRequestResult {
  oracle_name: string;
  version: number;
  query_count: number;
  token: string;
  expires_at: string;
  checklist: string;
}

interface DecommissionCancelResult {
  oracle_name: string;
  cancelled_at: string;
}

interface DecommissionExecuteResult {
  oracle_name: string;
  decommissioned_at: string;
  final_checkpoint_path: string | null;
}

async function decommissionRequest(params: {
  name: string;
  reason: string;
}): Promise<OracleResult<DecommissionRequestResult>> {
  // 1. Registry lookup
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<DecommissionRequestResult>;
  if (!lookup.data) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found`);
  }
  if (lookup.data.decommissioned_at) {
    return fail("DECOMMISSION_REFUSED", `Oracle "${params.name}" is already decommissioned`);
  }
  const { oracle_dir } = lookup.data;

  // 2. Read state
  const stateResult = await readState(oracle_dir);
  if (!stateResult.ok) return stateResult as OracleResult<DecommissionRequestResult>;
  const state = stateResult.data;

  // 3. Generate token with 10-minute TTL (stored in-memory only)
  const token = randomUUID();
  const expiresAt = Date.now() + 600_000;
  const runtime = getGeminiRuntime();
  runtime.decommissionTokens.set(token, {
    token,
    oracle_name: params.name,
    expires_at: expiresAt,
  });

  // 4. Log session_note interaction
  const seq = state.next_seq;
  const entryId = `v${state.version}-q${String(seq).padStart(3, "0")}`;
  const note: InteractionEntry = {
    id: entryId,
    seq,
    entry_schema_version: CURRENT_ENTRY_SCHEMA_VERSION,
    type: "session_note",
    oracle_name: params.name,
    version: state.version,
    query_count: state.query_count,
    timestamp: new Date().toISOString(),
    trace_id: randomUUID(),
    span_id: randomUUID(),
    parent_span_id: null,
    tokens_remaining_at_query: state.tokens_remaining ?? 0,
    chars_in_at_query: 0,
    question: `Decommission requested. Reason: ${params.reason}`,
  };
  const interactionsPath = join(oracle_dir, "learnings", `v${state.version}-interactions.jsonl`);
  try {
    await mkdir(join(oracle_dir, "learnings"), { recursive: true });
    await writeFile(interactionsPath, JSON.stringify(note) + "\n", { flag: "a" });
  } catch { /* non-fatal */ }

  // Increment next_seq in state
  await writeStateWithRetry(oracle_dir, (s) => ({
    ...s, next_seq: s.next_seq + 1, updated_at: new Date().toISOString(),
  }));

  const expiresAtIso = new Date(expiresAt).toISOString();
  const confirmPhrase = `DELETE ${params.name} generation ${state.version} containing ${state.query_count} interactions`;
  const checklist = `Decommission requested for oracle "${params.name}" v${state.version}.
Token expires in 10 minutes.

Required steps before oracle_decommission_execute:
1. Run /pythia quality and /pythia status
2. Run pythia-auth in your terminal to get TOTP code
3. Type confirmation phrase: "${confirmPhrase}"
4. Wait 5 minutes (cooling-off period)
5. Call oracle_decommission_execute with token, totp_code, and confirmation_phrase

Token: ${token}
Expires: ${expiresAtIso}`;

  return {
    ok: true,
    data: {
      oracle_name: params.name,
      version: state.version,
      query_count: state.query_count,
      token,
      expires_at: expiresAtIso,
      checklist,
    },
  };
}

async function decommissionCancel(params: {
  name: string;
  token: string;
}): Promise<OracleResult<DecommissionCancelResult>> {
  const runtime = getGeminiRuntime();

  // 1. Validate token
  const stored = runtime.decommissionTokens.get(params.token);
  if (!stored || stored.oracle_name !== params.name) {
    return fail("DECOMMISSION_REFUSED", `No active decommission request for oracle "${params.name}"`);
  }

  // 2. Remove token
  runtime.decommissionTokens.delete(params.token);

  // 3. Log session_note
  const lookup = await lookupOracle(params.name);
  if (lookup.ok && lookup.data && !lookup.data.decommissioned_at) {
    const { oracle_dir } = lookup.data;
    const stateResult = await readState(oracle_dir);
    if (stateResult.ok) {
      const state = stateResult.data;
      const seq = state.next_seq;
      const note: InteractionEntry = {
        id: `v${state.version}-q${String(seq).padStart(3, "0")}`,
        seq,
        entry_schema_version: CURRENT_ENTRY_SCHEMA_VERSION,
        type: "session_note",
        oracle_name: params.name,
        version: state.version,
        query_count: state.query_count,
        timestamp: new Date().toISOString(),
        trace_id: randomUUID(),
        span_id: randomUUID(),
        parent_span_id: null,
        tokens_remaining_at_query: state.tokens_remaining ?? 0,
        chars_in_at_query: 0,
        question: "Decommission cancelled by user",
      };
      try {
        const interactionsPath = join(oracle_dir, "learnings", `v${state.version}-interactions.jsonl`);
        await writeFile(interactionsPath, JSON.stringify(note) + "\n", { flag: "a" });
        await writeStateWithRetry(oracle_dir, (s) => ({
          ...s, next_seq: s.next_seq + 1, updated_at: new Date().toISOString(),
        }));
      } catch { /* non-fatal */ }
    }
  }

  const cancelledAt = new Date().toISOString();
  return { ok: true, data: { oracle_name: params.name, cancelled_at: cancelledAt } };
}

async function decommissionExecute(params: {
  name: string;
  token: string;
  totp_code: string;
  confirmation_phrase: string;
}): Promise<OracleResult<DecommissionExecuteResult>> {
  const runtime = getGeminiRuntime();

  // Validation step 1: Token lookup
  const stored = runtime.decommissionTokens.get(params.token);
  if (!stored) {
    return fail("DECOMMISSION_TOKEN_EXPIRED", "Decommission token not found or expired");
  }
  if (Date.now() > stored.expires_at) {
    runtime.decommissionTokens.delete(params.token);
    return fail("DECOMMISSION_TOKEN_EXPIRED", "Decommission token has expired");
  }
  if (stored.oracle_name !== params.name) {
    return fail("DECOMMISSION_REFUSED", `Token is for oracle "${stored.oracle_name}", not "${params.name}"`);
  }

  // Validation step 2: TOTP
  const totpSecret = await readTotpSecret(params.name);
  if (totpSecret) {
    if (!verifyTotp(totpSecret, params.totp_code)) {
      return fail("TOTP_INVALID", "TOTP code is invalid — run pythia-auth to get current code");
    }
  }
  // If no TOTP secret provisioned yet, skip TOTP check (oracle not yet enrolled)
  // This allows decommission of oracles created before pythia-auth enrollment was implemented

  // Validation step 3: Registry + state for confirmation phrase
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<DecommissionExecuteResult>;
  if (!lookup.data) return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found`);
  if (lookup.data.decommissioned_at) {
    return fail("DECOMMISSION_REFUSED", `Oracle "${params.name}" is already decommissioned`);
  }
  const { oracle_dir, project_root } = lookup.data;

  const stateResult = await readState(oracle_dir);
  if (!stateResult.ok) return stateResult as OracleResult<DecommissionExecuteResult>;
  const state = stateResult.data;

  // Validation step 3: Confirmation phrase (case-sensitive exact match)
  const expectedPhrase = `DELETE ${params.name} generation ${state.version} containing ${state.query_count} interactions`;
  if (params.confirmation_phrase !== expectedPhrase) {
    return fail(
      "CONFIRMATION_PHRASE_MISMATCH",
      `Phrase mismatch. Expected exactly: "${expectedPhrase}"`,
    );
  }

  // Consume token — all validation passed
  runtime.decommissionTokens.delete(params.token);

  // Step 4: Acquire lock
  const lockResult = await acquireOperationLock(oracle_dir, "decommission");
  if (!lockResult.ok) return lockResult as OracleResult<DecommissionExecuteResult>;
  const { lockToken } = lockResult.data;
  const heartbeat = startLockHeartbeat({
    oracleDir: oracle_dir,
    operation: "decommission",
    lockToken,
    extendEveryMs: 60_000,
    ttlMs: 300_000,
  });

  let finalCheckpointPath: string | null = null;

  try {
    // Step 5: Best-effort checkpoint (do NOT abort if this fails)
    const ckptResult = await runCheckpoint({ name: params.name, commit: true });
    if (ckptResult.ok) {
      finalCheckpointPath = ckptResult.data.checkpoint_path;
    } else {
      const salvageResult = await runSalvage({ name: params.name });
      if (salvageResult.ok) finalCheckpointPath = salvageResult.data.checkpoint_path;
      // Both failed — continue decommission anyway
    }

    // Step 6: Hard-dismiss all pool members
    const freshState = await readState(oracle_dir);
    const pool = freshState.ok ? freshState.data.daemon_pool : state.daemon_pool;
    for (const member of pool) {
      if (member.daemon_id) {
        try {
          await runtime.dismissDaemon({ daemon_id: member.daemon_id, hard: true });
        } catch { /* best-effort */ }
      }
    }

    // Step 7: Mark state as decommissioned
    const decommissionedAt = new Date().toISOString();
    await writeStateWithRetry(oracle_dir, (s) => ({
      ...s,
      status: "decommissioned" as OracleStatus,
      daemon_pool: [],
      last_error: null,
      updated_at: decommissionedAt,
    }));

    // Step 8: Archive registry entry (set decommissioned_at, do NOT delete)
    await updateRegistryEntry(params.name, { decommissioned_at: decommissionedAt });

    // Step 9: Remove .pythia-active/<name>.json marker if it exists
    try {
      const { homedir: getHomedir } = await import("node:os");
      const activeDir = join(getHomedir(), ".pythia-active");
      const markerFile = join(activeDir, `${params.name}.json`);
      await unlink(markerFile).catch(() => {});
      const remaining = await readdir(activeDir).catch(() => []);
      if (remaining.length === 0) await rmdir(activeDir).catch(() => {});
    } catch { /* non-fatal */ }

    // Git commit: archive the state
    try {
      const stateFile = join(oracle_dir, "state.json");
      execSync(
        `git add "${stateFile}" && ` +
        `git commit -m "oracle(${params.name}): decommissioned v${state.version}"`,
        { cwd: project_root, stdio: "pipe" },
      );
    } catch { /* non-fatal */ }

    return {
      ok: true,
      data: {
        oracle_name: params.name,
        decommissioned_at: new Date().toISOString(),
        final_checkpoint_path: finalCheckpointPath,
      },
    };

  } finally {
    await heartbeat.stop();
    await releaseLock(oracle_dir, lockToken);
  }
}

// ─── Quality Report (FEAT-010) ───────────────────────────────────────────────

/**
 * Count "code-like" tokens in a string:
 * camelCase identifiers, snake_case, file paths (dots/slashes), proper nouns with dots.
 */
function countCodeTokens(text: string): { code: number; total: number } {
  const words = text.split(/\s+/).filter(Boolean);
  let code = 0;
  for (const w of words) {
    if (
      /[a-z][A-Z]/.test(w) ||            // camelCase
      /[A-Z]{2,}/.test(w) ||             // ALLCAPS acronym
      /[._/\\]/.test(w) ||               // path/file separator
      /^[a-z]+_[a-z]/.test(w) ||         // snake_case
      /\(\)$/.test(w) ||                  // function call
      /^\d+$/.test(w)                     // pure numbers (line refs, counts)
    ) {
      code++;
    }
  }
  return { code, total: words.length };
}

/**
 * Compute P50 (median) of a number array. Returns 0 for empty arrays.
 */
function p50(values: number[]): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[mid - 1] + sorted[mid]) / 2
    : sorted[mid];
}

/**
 * Compute a quality report for a given oracle version by analyzing its interactions log.
 */
async function computeQualityReport(params: {
  name: string;
  version?: number;
}): Promise<OracleResult<QualityReport>> {
  // 1. Registry + state lookup
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<QualityReport>;
  if (!lookup.data || lookup.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found or decommissioned`);
  }
  const { oracle_dir } = lookup.data;

  const stateResult = await readState(oracle_dir);
  if (!stateResult.ok) return stateResult as OracleResult<QualityReport>;
  const state = stateResult.data;

  const manifestResult = await readManifest(oracle_dir);
  if (!manifestResult.ok) return manifestResult as OracleResult<QualityReport>;

  const version = params.version ?? state.version;

  // 2. Read interactions log
  const interactionsPath = join(oracle_dir, "learnings", `v${version}-interactions.jsonl`);
  let lines: string[] = [];
  try {
    const raw = await readFile(interactionsPath, "utf-8");
    lines = raw.trim().split("\n").filter(Boolean);
  } catch { /* no interactions yet */ }

  // Only consultation entries with counsel contribute to metrics
  const consultations: InteractionEntry[] = [];
  for (const line of lines) {
    try {
      const entry = JSON.parse(line) as InteractionEntry;
      if (entry.type === "consultation" && entry.counsel) consultations.push(entry);
    } catch { /* skip malformed */ }
  }

  const queryCount = consultations.length;

  if (queryCount === 0) {
    return {
      ok: true,
      data: {
        oracle_name: params.name,
        version,
        query_count: 0,
        avg_answer_length_early: 0,
        avg_answer_length_late: 0,
        length_trend_pct_change: 0,
        code_symbol_density_early: 0,
        code_symbol_density_late: 0,
        flags: [],
      },
    };
  }

  // 3. Split into early and late halves
  const half = Math.ceil(queryCount / 2);
  const earlySet = consultations.slice(0, half);
  const lateSet = consultations.slice(half);

  // 4. Answer length metrics
  const avgLen = (set: InteractionEntry[]) =>
    set.reduce((s, e) => s + (e.counsel?.length ?? 0), 0) / (set.length || 1);
  const avgLengthEarly = avgLen(earlySet);
  const avgLengthLate = avgLen(lateSet);
  const lengthTrendPct = avgLengthEarly > 0
    ? ((avgLengthLate - avgLengthEarly) / avgLengthEarly) * 100
    : 0;

  // 5. Code-symbol density metrics
  const avgDensity = (set: InteractionEntry[]) => {
    let totalCode = 0; let totalWords = 0;
    for (const e of set) {
      const { code, total } = countCodeTokens(e.counsel ?? "");
      totalCode += code; totalWords += total;
    }
    return totalWords > 0 ? totalCode / totalWords : 0;
  };
  const densityEarly = avgDensity(earlySet);
  const densityLate = avgDensity(lateSet);

  // 6. Degradation onset detection
  const flags: DegradationFlag[] = [];
  let degradationOnsetQuery: string | undefined;
  let degradationOnsetTokensRemaining: number | undefined;

  // Sliding window: detect first query where length AND density both drop
  const avgLenAll = avgLen(consultations);
  const lenThreshold = avgLenAll * 0.6;  // 40% drop triggers flag
  const { code: earlyCode, total: earlyTotal } = (() => {
    let c = 0; let t = 0;
    for (const e of earlySet) { const r = countCodeTokens(e.counsel ?? ""); c += r.code; t += r.total; }
    return { code: c, total: t };
  })();
  const baselineDensity = earlyTotal > 0 ? earlyCode / earlyTotal : 0;
  const densityThreshold = baselineDensity * 0.5; // 50% density drop triggers flag

  for (const entry of consultations) {
    const len = entry.counsel?.length ?? 0;
    const { code, total } = countCodeTokens(entry.counsel ?? "");
    const density = total > 0 ? code / total : 0;

    if (len < lenThreshold && !flags.some((f) => f.type === "length_drop")) {
      flags.push({
        type: "length_drop",
        query_id: entry.id,
        tokens_remaining: entry.tokens_remaining_at_query,
        description: `Response length dropped to ${len} chars (avg: ${Math.round(avgLenAll)}, threshold: ${Math.round(lenThreshold)})`,
      });
      if (!degradationOnsetQuery) {
        degradationOnsetQuery = entry.id;
        degradationOnsetTokensRemaining = entry.tokens_remaining_at_query;
      }
    }

    if (density < densityThreshold && baselineDensity > 0.05 && !flags.some((f) => f.type === "vagueness")) {
      flags.push({
        type: "vagueness",
        query_id: entry.id,
        tokens_remaining: entry.tokens_remaining_at_query,
        description: `Code-symbol density dropped to ${(density * 100).toFixed(1)}% (baseline: ${(baselineDensity * 100).toFixed(1)}%, threshold: ${(densityThreshold * 100).toFixed(1)}%)`,
      });
      if (!degradationOnsetQuery) {
        degradationOnsetQuery = entry.id;
        degradationOnsetTokensRemaining = entry.tokens_remaining_at_query;
      }
    }

    // Incorporate manual flags from the entry itself (self_contradiction, hallucination)
    for (const flag of entry.flags ?? []) {
      if (["self_contradiction", "hallucination"].includes(flag)) {
        flags.push({
          type: flag as DegradationFlag["type"],
          query_id: entry.id,
          tokens_remaining: entry.tokens_remaining_at_query,
          description: `Manual flag: ${flag}`,
        });
      }
    }
  }

  // 7. Suggested headroom computation (v1: no cross-version history yet)
  let suggestedHeadroomTokens: number | undefined;
  if (degradationOnsetTokensRemaining !== undefined && state.discovered_context_window) {
    const p50val = p50([degradationOnsetTokensRemaining]);
    const maxHeadroom = state.discovered_context_window * 0.5;
    suggestedHeadroomTokens = Math.round(Math.max(100_000, Math.min(p50val + 50_000, maxHeadroom)));
  }

  const report: QualityReport = {
    oracle_name: params.name,
    version,
    query_count: queryCount,
    degradation_onset_query: degradationOnsetQuery,
    degradation_onset_tokens_remaining: degradationOnsetTokensRemaining,
    avg_answer_length_early: Math.round(avgLengthEarly),
    avg_answer_length_late: Math.round(avgLengthLate),
    length_trend_pct_change: Math.round(lengthTrendPct * 10) / 10,
    code_symbol_density_early: Math.round(densityEarly * 1000) / 1000,
    code_symbol_density_late: Math.round(densityLate * 1000) / 1000,
    suggested_headroom_tokens: suggestedHeadroomTokens,
    flags,
  };

  return { ok: true, data: report };
}

interface OracleInitResult {
  name: string;
  oracle_dir: string;
  files_registered: number;
  corpus_truncated: boolean;
  skipped_files?: string[];
}

interface OracleHealthResult {
  total_files: number;
  stale_files: string[];
  missing_files: string[];
  last_spawn_timestamp: string | null;
  status: "active" | "idle" | "dead";
}

interface OracleRefreshResult {
  files_updated: number;
  files_removed: number;
}

function resolveProjectFile(projectRoot: string, filePath: string): string {
  return filePath.startsWith("/") ? filePath : join(projectRoot, filePath);
}

function withinDiscoveryDepth(projectRoot: string, filePath: string, maxDepth = 3): boolean {
  const relativePath = relative(projectRoot, filePath);
  if (relativePath.startsWith("..")) {
    return false;
  }

  const segments = relativePath.split(/[\\/]/u).filter(Boolean);
  return Math.max(segments.length - 1, 0) <= maxDepth;
}

async function autoDiscoverOracleFiles(projectRoot: string): Promise<string[]> {
  const readmePath = join(projectRoot, "README.md");
  const discovered = new Set<string>();

  if (existsSync(readmePath)) {
    discovered.add(readmePath);
  }

  for (const pattern of ORACLE_INIT_DISCOVERY_PATTERNS.slice(1)) {
    const matches = globSync(pattern, {
      cwd: projectRoot,
    });
    for (const match of matches) {
      const absolutePath = join(projectRoot, match);
      if (withinDiscoveryDepth(projectRoot, absolutePath)) {
        discovered.add(absolutePath);
      }
    }
  }

  const readmeFirst = discovered.has(readmePath) ? [readmePath] : [];
  const remainder = [...discovered]
    .filter((filePath) => filePath !== readmePath);

  const withSizes = await Promise.all(
    remainder.map(async (filePath) => {
      const fileStat = await stat(filePath);
      return { filePath, size: fileStat.size };
    }),
  );

  withSizes.sort((left, right) => {
    if (left.size !== right.size) {
      return left.size - right.size;
    }

    return left.filePath.localeCompare(right.filePath);
  });

  return [...readmeFirst, ...withSizes.map((entry) => entry.filePath)];
}

async function readRegistryOrEmpty(): Promise<OracleRegistry> {
  const registryResult = await readRegistry();
  if (registryResult.ok) {
    return registryResult.data;
  }

  if (registryResult.error.code !== "FILE_NOT_FOUND") {
    throw new Error(registryResult.error.message);
  }

  return {
    schema_version: 1,
    oracles: {},
  };
}

async function oracleInit(params: {
  name: string;
  description: string;
  files?: string[];
}): Promise<OracleResult<OracleInitResult>> {
  const projectRoot = process.cwd();
  const oracleDir = join(PYTHIA_ORACLES_DIR, params.name);
  const createdAt = new Date().toISOString();

  let registry: OracleRegistry;
  try {
    registry = await readRegistryOrEmpty();
  } catch (err: unknown) {
    return fail("IO_ERROR", `Failed to prepare registry: ${(err as Error).message}`);
  }

  const existing = registry.oracles[params.name];
  if (existing && !existing.decommissioned_at) {
    return fail("ORACLE_ALREADY_EXISTS", `Oracle "${params.name}" already exists`);
  }

  if (existsSync(oracleDir)) {
    return fail("ORACLE_ALREADY_EXISTS", `Oracle directory already exists: ${oracleDir}`);
  }

  let candidateFiles: string[];
  try {
    candidateFiles = params.files?.length
      ? params.files.map((filePath) => resolveProjectFile(projectRoot, filePath))
      : await autoDiscoverOracleFiles(projectRoot);
  } catch (err: unknown) {
    return fail("IO_ERROR", `Failed to discover oracle files: ${(err as Error).message}`);
  }

  candidateFiles = [...new Set(candidateFiles)];

  const staticEntries: StaticEntry[] = [];
  const skippedFiles: string[] = [];
  let totalChars = 0;

  for (const filePath of candidateFiles) {
    let content: string;
    try {
      content = await readFile(filePath, "utf-8");
    } catch (err: unknown) {
      return fail("FILE_NOT_FOUND", `Oracle init file not found: ${filePath}`, false, {
        error_code: "FILE_NOT_FOUND",
        path: filePath,
      });
    }

    if (content.length > ORACLE_INIT_CORPUS_CHAR_CAP || totalChars + content.length > ORACLE_INIT_CORPUS_CHAR_CAP) {
      skippedFiles.push(filePath);
      continue;
    }

    totalChars += content.length;
    staticEntries.push({
      id: randomUUID(),
      path: filePath,
      role: "core_research",
      required: false,
      sha256: sha256(content),
      added_at: createdAt,
    });
  }

  const manifest: OracleManifest = {
    schema_version: CURRENT_MANIFEST_SCHEMA_VERSION,
    name: params.name,
    description: params.description,
    project: basename(projectRoot),
    version: 1,
    checkpoint_headroom_tokens: 200_000,
    pool_size: 1,
    static_entries: staticEntries,
    live_sources: [],
    load_order: ORACLE_DEFAULT_LOAD_ORDER,
    created_at: createdAt,
  };

  const initialState: OracleState = {
    schema_version: CURRENT_STATE_SCHEMA_VERSION,
    oracle_name: params.name,
    version: 1,
    spawned_at: null,
    last_spawn_at: null,
    discovered_context_window: null,
    daemon_pool: [],
    session_chars_at_spawn: null,
    chars_per_token_estimate: DEFAULT_CHARS_PER_TOKEN_ESTIMATE,
    token_count_method: "estimate",
    estimated_total_tokens: null,
    estimated_cluster_tokens: null,
    tokens_remaining: null,
    query_count: 0,
    last_checkpoint_path: null,
    status: "healthy",
    lock_held_by: null,
    lock_expires_at: null,
    last_error: null,
    last_bootstrap_ack: null,
    next_seq: 1,
    generation_since_reground: 0,
    state_version: 1,
    updated_at: createdAt,
  };

  try {
    await mkdir(oracleDir, { recursive: true });
    ensurePythiaLogsDir();
    await atomicWriteFile(join(oracleDir, "manifest.json"), JSON.stringify(normalizeManifestForWrite(manifest), null, 2) + "\n");
    await atomicWriteFile(join(oracleDir, "state.json"), JSON.stringify(initialState, null, 2) + "\n");
    registry.oracles[params.name] = {
      name: params.name,
      oracle_dir: oracleDir,
      project_root: projectRoot,
      description: params.description,
      created_at: createdAt,
    };
    await atomicWriteFile(REGISTRY_PATH, JSON.stringify(registry, null, 2) + "\n");
  } catch (err: unknown) {
    return fail("IO_ERROR", `Failed to initialize oracle: ${(err as Error).message}`);
  }

  const result: OracleInitResult = {
    name: params.name,
    oracle_dir: oracleDir,
    files_registered: staticEntries.length,
    corpus_truncated: skippedFiles.length > 0,
  };

  if (skippedFiles.length > 0) {
    result.skipped_files = skippedFiles;
  }

  return ok(result);
}

async function oracleHealth(params: { name: string }): Promise<OracleResult<OracleHealthResult>> {
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<OracleHealthResult>;
  if (!lookup.data || lookup.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found or decommissioned`);
  }

  const manifestResult = await readManifest(lookup.data.oracle_dir);
  if (!manifestResult.ok) return manifestResult as OracleResult<OracleHealthResult>;
  const stateResult = await readState(lookup.data.oracle_dir);
  if (!stateResult.ok) return stateResult as OracleResult<OracleHealthResult>;

  const scanResult = await scanStaticEntries(manifestResult.data.static_entries);
  if (!scanResult.ok) return scanResult as OracleResult<OracleHealthResult>;

  const state = stateResult.data;
  const hasLiveMember = state.daemon_pool.some(
    (member) => member.status !== "dead" && member.status !== "dismissed",
  );
  const allDead = state.daemon_pool.length > 0 && state.daemon_pool.every((member) => member.status === "dead");
  const status = hasLiveMember ? "active" : allDead ? "dead" : "idle";

  return ok({
    total_files: manifestResult.data.static_entries.length,
    stale_files: scanResult.data.stale_files.map((entry) => entry.path),
    missing_files: [
      ...scanResult.data.missing_required,
      ...scanResult.data.missing_optional,
    ],
    last_spawn_timestamp: state.last_spawn_at ?? null,
    status,
  });
}

async function oracleRefresh(params: {
  name: string;
  force?: boolean;
}): Promise<OracleResult<OracleRefreshResult>> {
  const force = params.force ?? false;
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<OracleRefreshResult>;
  if (!lookup.data || lookup.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found or decommissioned`);
  }

  const manifestResult = await readManifest(lookup.data.oracle_dir);
  if (!manifestResult.ok) return manifestResult as OracleResult<OracleRefreshResult>;
  const manifest = manifestResult.data;

  const scanResult = await scanStaticEntries(manifest.static_entries);
  if (!scanResult.ok) return scanResult as OracleResult<OracleRefreshResult>;
  const scan = scanResult.data;

  if (scan.missing_required.length > 0) {
    return fail(
      "MISSING_REQUIRED_FILE",
      `Required corpus files are missing for oracle "${params.name}"`,
      false,
      {
        error_code: "MISSING_REQUIRED_FILE",
        missing_files: scan.missing_required,
      },
    );
  }

  const staleByPath = new Map(scan.stale_files.map((entry) => [entry.path, entry.actual]));
  const resolvedByPath = new Map(scan.resolved.map((entry) => [entry.entry.path, entry.actual_sha256]));
  const missingOptional = new Set(scan.missing_optional);
  let filesUpdated = 0;
  let filesRemoved = 0;

  const nextManifest: OracleManifest = {
    ...manifest,
    static_entries: manifest.static_entries.flatMap((entry) => {
      if (missingOptional.has(entry.path)) {
        filesRemoved += 1;
        return [];
      }

      const nextHash = force
        ? resolvedByPath.get(entry.path)
        : staleByPath.get(entry.path);

      if (nextHash !== undefined && nextHash !== entry.sha256) {
        filesUpdated += 1;
        return [{
          ...entry,
          sha256: nextHash,
        }];
      }

      if (force && nextHash !== undefined) {
        filesUpdated += 1;
        return [{
          ...entry,
          sha256: nextHash,
        }];
      }

      return [entry];
    }),
  };

  const writeResult = await writeManifest(lookup.data.oracle_dir, nextManifest);
  if (!writeResult.ok) {
    return writeResult as OracleResult<OracleRefreshResult>;
  }

  return ok({
    files_updated: filesUpdated,
    files_removed: filesRemoved,
  });
}

// ─── Corpus Management (FEAT-006, FEAT-007) ───────────────────────────────────

interface AddToCorpusResult {
  added: number;
  already_present: boolean;
  already_present_paths: string[];
  corpus_total_chars: number;
  entries: StaticEntry[];
  entry?: StaticEntry;
  loaded_into_daemon: boolean;
  warned: number;
}

interface UpdateEntryResult {
  old_sha256: string;
  new_sha256: string;
  updated_at: string;
}

async function addToCorpus(params: {
  name: string;
  file_path?: string;
  files?: string | string[];
  role: CorpusRole;
  required?: boolean;
  priority?: number;
  load_now?: boolean;
  dedupe?: boolean;
}): Promise<OracleResult<AddToCorpusResult>> {
  const required = params.required ?? true;
  const priority = params.priority ?? 10;
  const loadNow = params.load_now ?? false;
  const dedupe = params.dedupe ?? true;
  const warnings: string[] = [];

  // 1. Registry lookup
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<AddToCorpusResult>;
  if (!lookup.data || lookup.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found or decommissioned`);
  }
  const { oracle_dir, project_root } = lookup.data;
  const manifestPath = join(oracle_dir, "manifest.json");
  const inputFiles = Array.from(new Set(normalizeCorpusFiles(params)));

  if (inputFiles.length === 0) {
    return fail("MANIFEST_INVALID", "Provide file_path or files when adding corpus entries");
  }

  // 2. Verify files exist and preload content
  const fileContents = new Map<string, string>();
  const fileShas = new Map<string, string>();
  for (const filePath of inputFiles) {
    try {
      await stat(filePath);
    } catch {
      return fail("FILE_NOT_FOUND", `File not found: ${filePath}`);
    }

    const content = await readFile(filePath, "utf-8");
    fileContents.set(filePath, content);
    fileShas.set(filePath, createHash("sha256").update(content).digest("hex"));
  }

  const sourceLikeFiles = inputFiles.filter((filePath) => isSourceLikeFile(filePath));
  if (sourceLikeFiles.length > 0) {
    warnings.push(
      `SOURCE_FILE_WARNING: Source-like files detected (${sourceLikeFiles.join(", ")}). ` +
      "This API uses corpus roles, so verify the selected role is intentional."
    );
  }

  // 3. Lock and update manifest atomically
  const manifestLock = await acquireManifestLock(manifestPath);
  if (!manifestLock.ok) return manifestLock as OracleResult<AddToCorpusResult>;

  let entries: StaticEntry[] = [];
  let alreadyPresentPaths: string[] = [];
  let newEntries: StaticEntry[] = [];
  let corpusTotalChars = 0;

  try {
    const manifestResult = await readManifest(oracle_dir);
    if (!manifestResult.ok) return manifestResult as OracleResult<AddToCorpusResult>;
    const manifest = manifestResult.data;
    const existingByPath = new Map(manifest.static_entries.map((entry) => [entry.path, entry]));

    for (const filePath of inputFiles) {
      const existing = existingByPath.get(filePath);
      if (dedupe && existing) {
        alreadyPresentPaths.push(filePath);
        entries.push(existing);
        continue;
      }

      const newEntry: StaticEntry = {
        path: filePath,
        role: params.role,
        required,
        sha256: fileShas.get(filePath) ?? "",
        added_at: new Date().toISOString(),
        priority,
      };
      entries.push(newEntry);
      newEntries.push(newEntry);
    }

    const replacementPaths = new Set(newEntries.map((entry) => entry.path));
    const updatedManifest: OracleManifest = {
      ...manifest,
      static_entries: [
        ...manifest.static_entries.filter((entry) => !replacementPaths.has(entry.path)),
        ...newEntries,
      ],
    };
    const writeResult = await writeManifestLocked(oracle_dir, updatedManifest);
    if (!writeResult.ok) return writeResult as OracleResult<AddToCorpusResult>;

    corpusTotalChars = await computeStaticCorpusCharTotal(updatedManifest.static_entries, fileContents);
  } finally {
    await releaseManifestLock(manifestLock.data.lockPath, manifestLock.data.token);
  }

  if (corpusTotalChars > 1_500_000) {
    warnings.push(
      `CORPUS_SIZE_WARNING: Corpus now totals ${corpusTotalChars.toLocaleString()} chars, above the 1,500,000 char advisory threshold.`
    );
  }

  // 4. Optional: inject into running daemon as a single batch
  let loadedIntoDaemon = false;
  if (loadNow && newEntries.length > 0) {
    const stateResult = await readState(oracle_dir);
    if (stateResult.ok) {
      const activeMember = stateResult.data.daemon_pool.find(
        (m) => m.daemon_id && m.status === "idle",
      );
      if (activeMember?.daemon_id) {
        try {
          const runtime = getGeminiRuntime();
          const injectionEntries = newEntries.map((entry) => ({
            path: entry.path,
            content: fileContents.get(entry.path) ?? "",
          }));
          await runtime.askDaemon({
            daemon_id: activeMember.daemon_id,
            question: formatBatchCorpusPrompt(injectionEntries),
            timeout_ms: 120_000,
          });
          loadedIntoDaemon = true;
        } catch { /* best-effort */ }
      }
    }
  }

  // 5. Git commit
  try {
    const entryLabel = newEntries.length === 1
      ? newEntries[0].path.split("/").pop()
      : `${newEntries.length} entries`;
    execSync(
      `git add "${manifestPath}" && ` +
      `git commit -m "oracle(${params.name}): add ${params.role} corpus entry ${entryLabel}"`,
      { cwd: project_root, stdio: "pipe" },
    );
  } catch { /* non-fatal */ }

  const result: AddToCorpusResult = {
    added: newEntries.length,
    already_present: inputFiles.length === 1 && alreadyPresentPaths.length === 1,
    already_present_paths: alreadyPresentPaths,
    corpus_total_chars: corpusTotalChars,
    entries,
    entry: inputFiles.length === 1 ? entries[0] : undefined,
    loaded_into_daemon: loadedIntoDaemon,
    warned: warnings.length,
  };

  return ok(result, warnings.length > 0 ? warnings : undefined);
}

async function updateEntry(params: {
  name: string;
  file_path: string;
  reason: string;
  expected_old_sha256?: string;
  role?: CorpusRole;
  required?: boolean;
  commit?: boolean;
}): Promise<OracleResult<UpdateEntryResult>> {
  const commit = params.commit ?? true;

  // 1. Registry lookup
  const lookup = await lookupOracle(params.name);
  if (!lookup.ok) return lookup as OracleResult<UpdateEntryResult>;
  if (!lookup.data || lookup.data.decommissioned_at) {
    return fail("ORACLE_NOT_FOUND", `Oracle "${params.name}" not found or decommissioned`);
  }
  const { oracle_dir, project_root } = lookup.data;

  // 2. Verify file exists on disk
  try { await stat(params.file_path); } catch {
    return fail("FILE_NOT_FOUND", `File not found: ${params.file_path}`);
  }

  // 3. Read manifest
  const manifestResult = await readManifest(oracle_dir);
  if (!manifestResult.ok) return manifestResult as OracleResult<UpdateEntryResult>;
  const manifest = manifestResult.data;

  // 4. Find existing entry
  const existingIdx = manifest.static_entries.findIndex((e) => e.path === params.file_path);
  if (existingIdx === -1) {
    return fail("FILE_NOT_FOUND", `File "${params.file_path}" is not in the corpus manifest`);
  }
  const existing = manifest.static_entries[existingIdx];

  // 5. Optional sha256 guard (stale update protection)
  if (params.expected_old_sha256 && params.expected_old_sha256 !== existing.sha256) {
    return fail(
      "HASH_MISMATCH",
      `Stale update — manifest has ${existing.sha256.slice(0, 12)}…, you expected ${params.expected_old_sha256.slice(0, 12)}…`,
    );
  }

  const oldSha256 = existing.sha256;

  // 6. Recompute sha256 from disk
  const content = await readFile(params.file_path, "utf-8");
  const newSha256 = createHash("sha256").update(content).digest("hex");

  // 7. Update entry
  const updatedEntry: StaticEntry = {
    ...existing,
    sha256: newSha256,
    ...(params.role !== undefined ? { role: params.role } : {}),
    ...(params.required !== undefined ? { required: params.required } : {}),
  };
  const updatedEntries = [...manifest.static_entries];
  updatedEntries[existingIdx] = updatedEntry;

  const updatedManifest: OracleManifest = { ...manifest, static_entries: updatedEntries };
  const writeResult = await writeManifest(oracle_dir, updatedManifest);
  if (!writeResult.ok) return writeResult as OracleResult<UpdateEntryResult>;

  const updatedAt = new Date().toISOString();

  // 8. Optional git commit
  if (commit) {
    try {
      const basename = params.file_path.split("/").pop() ?? params.file_path;
      const manifestPath = join(oracle_dir, "manifest.json");
      execSync(
        `git add "${manifestPath}" && ` +
        `git commit -m "oracle(${params.name}): update entry ${basename} -- ${params.reason}"`,
        { cwd: project_root, stdio: "pipe" },
      );
    } catch { /* non-fatal */ }
  }

  return { ok: true, data: { old_sha256: oldSha256, new_sha256: newSha256, updated_at: updatedAt } };
}

// ─── MCP Tool Registration ──────────────────────────────────────────────────

function toolErrorResponse(error: { code: OracleErrorCode; message: string; retryable: boolean; details?: unknown }) {
  return {
    content: [{
      type: "text" as const,
      text: JSON.stringify({
        error: error.code,
        message: error.message,
        retryable: error.retryable,
        details: error.details,
      }, null, 2),
    }],
    isError: true,
  };
}

/**
 * Register all Pythia oracle MCP tools on the given server.
 * Called from server.ts alongside registerGeminiTools().
 */
export function registerOracleTools(server: McpServer): void {
  server.tool(
    "oracle_init",
    "Create a new Pythia oracle with zero manual bootstrap steps. Auto-discovers research files when files are omitted, writes manifest/state, and registers the oracle.",
    {
      name: z.string().min(1).describe("Oracle name"),
      description: z.string().min(1).describe("Human-readable description for the oracle"),
      files: z.array(z.string().min(1)).optional().describe("Optional corpus file paths; auto-discovery runs when omitted"),
    },
    async (params) => {
      const result = await oracleInit(params);
      if (!result.ok) {
        return toolErrorResponse(result.error);
      }
      const body: Record<string, unknown> = { ...result.data };
      if (result.warnings?.length) {
        body.warnings = result.warnings;
      }
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(body, null, 2),
        }],
      };
    },
  );

  server.tool(
    "spawn_oracle",
    "Spawn or resume a Pythia oracle — a persistent Gemini knowledge daemon grounded in a project corpus. " +
    "With reuse_existing=true (default), resumes an existing session at zero cost. " +
    "Set reuse_existing=false for a fresh spawn. Set force_reload=true to re-send corpus to a live session.",
    {
      name: z
        .string()
        .min(1)
        .describe("Oracle name — must match a registered oracle in the Pythia registry"),
      reuse_existing: z
        .boolean()
        .optional()
        .describe("Resume existing session if found (default: true). Set false for fresh spawn."),
      force_reload: z
        .boolean()
        .optional()
        .describe("Re-send full corpus to a live session (default: false). Only with reuse_existing=true."),
      auto_refresh: z
        .boolean()
        .optional()
        .describe("Re-hash stale files and prune missing optional files before spawning (default: false)"),
      force: z
        .boolean()
        .optional()
        .describe("Reserved for future use (default: false)"),
      timeout_ms: z
        .number()
        .min(10_000)
        .max(600_000)
        .optional()
        .describe("Spawn timeout in milliseconds (default: 300000 = 5 min)"),
    },
    async (params) => {
      const result = await spawnOracle(params);
      if (!result.ok) {
        return toolErrorResponse(result.error);
      }
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(result.data, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_health",
    "Read-only corpus health check. Reports stale files, missing files, last spawn timestamp, and derived daemon status without mutating any oracle state.",
    {
      name: z.string().min(1).describe("Oracle name"),
    },
    async (params) => {
      const result = await oracleHealth(params);
      if (!result.ok) {
        return toolErrorResponse(result.error);
      }
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(result.data, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_refresh",
    "Refresh manifest hashes for stale static files and remove missing optional files. With force=true, re-hashes every current static entry.",
    {
      name: z.string().min(1).describe("Oracle name"),
      force: z.boolean().optional().describe("Re-hash all static entries, even when their hashes are already current"),
    },
    async (params) => {
      const result = await oracleRefresh(params);
      if (!result.ok) {
        return toolErrorResponse(result.error);
      }
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(result.data, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_sync_corpus",
    "Sync live source files to a running Pythia oracle. Resolves globs, computes delta " +
    "against last sync, and dispatches changes to pool members. No-op if tree hash unchanged. " +
    "Idle members get immediate injection; busy members get queued for drain before next query.",
    {
      name: z
        .string()
        .min(1)
        .describe("Oracle name — must match a registered oracle"),
      source_id: z
        .string()
        .optional()
        .describe("Specific live_source ID to sync. If omitted, syncs all live sources."),
    },
    async (params) => {
      const result = await syncCorpus(params);
      if (!result.ok) {
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify({
              error: result.error.code,
              message: result.error.message,
              retryable: result.error.retryable,
            }, null, 2),
          }],
          isError: true,
        };
      }
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(result.data, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_pressure_check",
    "Compute context pressure for a running Pythia oracle. Returns tokens_remaining, " +
    "estimated_total_tokens (MAX across pool), estimated_cluster_tokens (SUM for observability), " +
    "status, and recommendation. Updates state.json with current pressure metrics. " +
    "Returns PRESSURE_UNAVAILABLE if no active pool members or context window unknown.",
    {
      name: z
        .string()
        .min(1)
        .describe("Oracle name — must match a registered oracle"),
    },
    async (params) => {
      const result = await pressureCheck(params);
      if (!result.ok) {
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify({
              error: result.error.code,
              message: result.error.message,
              retryable: result.error.retryable,
            }, null, 2),
          }],
          isError: true,
        };
      }
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(result.data, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_log_learning",
    "Append an interaction entry (consultation, feedback, sync_event, or session_note) to the " +
    "named oracle's JSONL learnings file. Write is immediate and safe on disk; git commits are " +
    "batched (flush at 10 entries, 256 KB, 30-second debounce, or force=true). " +
    "Increments oracle query_count and next_seq in state.",
    {
      name: z.string().min(1).describe("Oracle name"),
      question: z.string().optional().describe("The question posed to the oracle (consultations)"),
      counsel: z.string().optional().describe("Pythia's full response"),
      decision: z.string().nullable().optional().describe("Decision made based on counsel"),
      type: z
        .enum(["consultation", "feedback", "sync_event", "session_note"])
        .optional()
        .describe("Interaction type (default: consultation)"),
      interaction_scope: z
        .enum(["architectural", "operational", "other"])
        .optional()
        .describe("Scope classification"),
      quality_signal: z
        .union([z.literal(1), z.literal(2), z.literal(3), z.literal(4), z.literal(5)])
        .nullable()
        .optional()
        .describe("Quality rating 1-5"),
      ion_delegated: z
        .boolean()
        .optional()
        .describe("Whether this query was delegated to an ion (Codex/Claude)"),
      ion_query: z.string().optional().describe("Query sent to the ion (required if ion_delegated=true)"),
      ion_response: z.string().optional().describe("Ion's response (required if ion_delegated=true)"),
      references: z.string().optional().describe("Consultation ID this feedback closes (feedback type)"),
      implemented: z.boolean().optional().describe("Whether counsel was implemented (feedback type)"),
      outcome: z.string().optional().describe("What actually happened (feedback type)"),
      divergence: z.string().optional().describe("How reality differed from counsel (feedback type)"),
      force: z.boolean().optional().describe("Force immediate git commit regardless of batch thresholds"),
    },
    async (params) => {
      const result = await logLearning(params);
      if (!result.ok) {
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify({
              error: result.error.code,
              message: result.error.message,
              retryable: result.error.retryable,
            }, null, 2),
          }],
          isError: true,
        };
      }
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(result.data, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_checkpoint",
    "Request a checkpoint from a running Pythia oracle. Sends a structured prompt asking Pythia " +
    "to synthesize its full context into a <checkpoint> document, then saves it to " +
    "<oracle_dir>/checkpoints/v<N>-checkpoint.md and adds it to the manifest. " +
    "Acquires an operation lock to prevent concurrent checkpoints. " +
    "Rejects with CHECKPOINT_FAILED if tokens_remaining < headroom/4 (use oracle_salvage instead).",
    {
      name: z.string().min(1).describe("Oracle name"),
      timeout_ms: z
        .number()
        .int()
        .positive()
        .optional()
        .describe("Timeout for Gemini response (default: 600000ms — checkpoints take 2-3 min)"),
      commit: z
        .boolean()
        .optional()
        .describe("Git commit checkpoint and manifest after saving (default: true)"),
    },
    async (params) => {
      const result = await runCheckpoint(params);
      if (!result.ok) {
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify({
              error: result.error.code,
              message: result.error.message,
              retryable: result.error.retryable,
            }, null, 2),
          }],
          isError: true,
        };
      }
      const body: Record<string, unknown> = { ...result.data };
      if (result.warnings?.length) body.warnings = result.warnings;
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(body, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_salvage",
    "Emergency checkpoint for a Pythia oracle when the daemon is dead or context is exhausted. " +
    "Uses a fresh Gemini call (not the oracle daemon) to synthesize the interactions log into a " +
    "checkpoint document, then saves it to <oracle_dir>/checkpoints/v<N>-checkpoint.md. " +
    "Falls back to inheriting the prior generation checkpoint when no interactions exist.",
    {
      name: z.string().min(1).describe("Oracle name"),
    },
    async (params) => {
      const result = await runSalvage(params);
      if (!result.ok) {
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify({
              error: result.error.code,
              message: result.error.message,
              retryable: result.error.retryable,
            }, null, 2),
          }],
          isError: true,
        };
      }
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(result.data, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_reconstitute",
    "Perform an atomic generation transition for a Pythia oracle. Locks the oracle (gating new " +
    "queries), drains in-flight queries, checkpoints (or salvages) the current generation, " +
    "soft-dismisses all pool members, increments the version, resolves corpus with delta-sync " +
    "filtering, spawns a new v(N+1) daemon, and loads the corpus. The oracle emerges at " +
    "version N+1 with a clean slate but full inherited wisdom.",
    {
      name: z.string().min(1).describe("Oracle name"),
      checkpoint_first: z
        .boolean()
        .optional()
        .describe("Attempt oracle_checkpoint before reconstituting (default: true)"),
      dismiss_old: z
        .boolean()
        .optional()
        .describe("Soft-dismiss old pool members after version transition (default: true)"),
      timeout_ms: z
        .number()
        .int()
        .positive()
        .optional()
        .describe("Timeout for the lock acquisition wait (default: 30000ms)"),
    },
    async (params) => {
      const result = await runReconstitute(params);
      if (!result.ok) {
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify({
              error: result.error.code,
              message: result.error.message,
              retryable: result.error.retryable,
            }, null, 2),
          }],
          isError: true,
        };
      }
      const body: Record<string, unknown> = { ...result.data };
      if (result.warnings?.length) body.warnings = result.warnings;
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(body, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_quality_report",
    "Analyze a Pythia oracle's interaction log for context degradation signals. " +
    "Computes answer-length trend, code-symbol density trend, detects onset of degradation, " +
    "and suggests an improved checkpoint_headroom_tokens value. " +
    "Call after a checkpoint to decide whether to reconstitute or adjust headroom.",
    {
      name: z.string().min(1).describe("Oracle name"),
      version: z
        .number()
        .int()
        .positive()
        .optional()
        .describe("Version to analyze (default: current version from state)"),
    },
    async (params) => {
      const result = await computeQualityReport(params);
      if (!result.ok) {
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify({
              error: result.error.code,
              message: result.error.message,
              retryable: result.error.retryable,
            }, null, 2),
          }],
          isError: true,
        };
      }
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(result.data, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_add_to_corpus",
    "Add one or more files to a Pythia oracle's static corpus manifest. " +
    "Verifies each file exists, computes sha256, and appends to manifest.static_entries atomically. " +
    "Set load_now=true to inject all new file contents into the running daemon in a single batch.",
    {
      name: z.string().min(1).describe("Oracle name"),
      files: z
        .union([z.string().min(1), z.array(z.string().min(1)).min(1)])
        .optional()
        .describe("Absolute path or list of absolute paths to add"),
      file_path: z
        .string()
        .min(1)
        .optional()
        .describe("Deprecated single-path alias for backwards compatibility"),
      role: z
        .enum(["core_research", "prompt_architecture", "pain_signals", "learnings", "checkpoint", "other"])
        .describe("Corpus role for this entry"),
      required: z
        .boolean()
        .optional()
        .describe("Whether this file is required for spawn (default: true)"),
      priority: z
        .number()
        .int()
        .optional()
        .describe("Sort order within role group — lower = earlier (default: 10)"),
      load_now: z
        .boolean()
        .optional()
        .describe("Inject file content into running daemon immediately (default: false)"),
      dedupe: z
        .boolean()
        .optional()
        .describe("Skip if file path already in manifest (default: true)"),
    },
    async (params) => {
      const result = await addToCorpus(params);
      if (!result.ok) {
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify({
              error: result.error.code,
              message: result.error.message,
              retryable: result.error.retryable,
            }, null, 2),
          }],
          isError: true,
        };
      }
      const body: Record<string, unknown> = { ...result.data };
      if (result.warnings?.length) body.warnings = result.warnings;
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(body, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_update_entry",
    "Update a corpus manifest entry with a new sha256 (from disk). " +
    "Provides stale-update protection via expected_old_sha256 guard. " +
    "Recomputes sha256 from the file on disk — does NOT accept a sha256 parameter.",
    {
      name: z.string().min(1).describe("Oracle name"),
      file_path: z.string().min(1).describe("Absolute path to the file (must already be in manifest)"),
      reason: z.string().min(1).describe("Human-readable reason for this update (goes into git commit message)"),
      expected_old_sha256: z
        .string()
        .length(64)
        .optional()
        .describe("Expected current manifest sha256 — prevents stale updates"),
      role: z
        .enum(["core_research", "prompt_architecture", "pain_signals", "learnings", "checkpoint", "other"])
        .optional()
        .describe("Update the role for this entry"),
      required: z
        .boolean()
        .optional()
        .describe("Update the required flag for this entry"),
      commit: z
        .boolean()
        .optional()
        .describe("Git commit manifest after update (default: true)"),
    },
    async (params) => {
      const result = await updateEntry(params);
      if (!result.ok) {
        return {
          content: [{
            type: "text" as const,
            text: JSON.stringify({
              error: result.error.code,
              message: result.error.message,
              retryable: result.error.retryable,
            }, null, 2),
          }],
          isError: true,
        };
      }
      return {
        content: [{
          type: "text" as const,
          text: JSON.stringify(result.data, null, 2),
        }],
      };
    },
  );

  server.tool(
    "oracle_decommission_request",
    "Initiate a decommission request for a Pythia oracle. Generates a time-limited token " +
    "(10 min TTL) stored in-memory only — never persisted to disk. Returns a checklist of " +
    "required steps including running pythia-auth for the TOTP code before executing.",
    {
      name: z.string().min(1).describe("Oracle name"),
      reason: z.string().min(1).describe("Human-readable reason for decommissioning"),
    },
    async (params) => {
      const result = await decommissionRequest(params);
      if (!result.ok) {
        return {
          content: [{ type: "text" as const, text: JSON.stringify({ error: result.error.code, message: result.error.message, retryable: result.error.retryable }, null, 2) }],
          isError: true,
        };
      }
      return { content: [{ type: "text" as const, text: JSON.stringify(result.data, null, 2) }] };
    },
  );

  server.tool(
    "oracle_decommission_cancel",
    "Cancel an active decommission request. Invalidates the token and logs a session_note.",
    {
      name: z.string().min(1).describe("Oracle name"),
      token: z.string().uuid().describe("Token returned by oracle_decommission_request"),
    },
    async (params) => {
      const result = await decommissionCancel(params);
      if (!result.ok) {
        return {
          content: [{ type: "text" as const, text: JSON.stringify({ error: result.error.code, message: result.error.message, retryable: result.error.retryable }, null, 2) }],
          isError: true,
        };
      }
      return { content: [{ type: "text" as const, text: JSON.stringify(result.data, null, 2) }] };
    },
  );

  server.tool(
    "oracle_decommission_execute",
    "Execute decommission after all 7 validation gates pass: valid token + TOTP code + " +
    "exact confirmation phrase. Hard-dismisses all daemon sessions, marks oracle as " +
    "decommissioned, and archives the registry entry. Oracle data directory is preserved on disk.",
    {
      name: z.string().min(1).describe("Oracle name"),
      token: z.string().uuid().describe("Token from oracle_decommission_request"),
      totp_code: z.string().length(6).describe("6-digit TOTP code from pythia-auth"),
      confirmation_phrase: z.string().min(1).describe(
        "Exact phrase: \"DELETE <name> generation <N> containing <Q> interactions\"",
      ),
    },
    async (params) => {
      const result = await decommissionExecute(params);
      if (!result.ok) {
        return {
          content: [{ type: "text" as const, text: JSON.stringify({ error: result.error.code, message: result.error.message, retryable: result.error.retryable }, null, 2) }],
          isError: true,
        };
      }
      return { content: [{ type: "text" as const, text: JSON.stringify(result.data, null, 2) }] };
    },
  );
}

// ─── Utility ────────────────────────────────────────────────────────────────

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ─── Exports for Testing ────────────────────────────────────────────────────

export {
  REGISTRY_PATH,
  CURRENT_STATE_SCHEMA_VERSION,
  CURRENT_MANIFEST_SCHEMA_VERSION,
  CURRENT_ENTRY_SCHEMA_VERSION,
  DEFAULT_LOCK_TTL_MS,
  DEFAULT_LOCK_WAIT_TIMEOUT_MS,
  pressureCheck,
  logLearning,
  runCheckpoint,
  extractCheckpointContent,
  runSalvage,
  runReconstitute,
  computeQualityReport,
  oracleInit,
  oracleHealth,
  oracleRefresh,
  addToCorpus,
  updateEntry,
  decommissionRequest,
  decommissionCancel,
  decommissionExecute,
  verifyTotp,
  BATCH_MAX_ENTRIES,
  BATCH_MAX_BYTES,
};

export type { OracleRegistry };
