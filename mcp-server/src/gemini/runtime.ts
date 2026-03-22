/**
 * GeminiRuntime — Singleton managing Gemini daemon session lifecycle.
 *
 * Extracted from tools.ts (Phase 1, Step 1.1 — FEAT-014, FEAT-015).
 * Provides the OracleRuntimeBridge interface consumed by both tools.ts
 * (existing inter-agent MCP tools) and oracle-tools.ts (Pythia oracle tools).
 *
 * Key invariants:
 * - Single instance per MCP server process (singleton via getGeminiRuntime())
 * - toolMutex serializes all pool state mutations (decision #47)
 * - ppidWatchdog detects parent death from SIGKILL (decision #48)
 * - idleSweepInterval sweeps oracle pool idle members (FEAT-022, no-op until Phase 2)
 * - decommissionTokens are in-memory only, never persisted (FEAT-035)
 */

import { Mutex } from "async-mutex";
import { mkdirSync, rmSync, existsSync, readdirSync, statSync, readFileSync, writeFileSync, renameSync } from "node:fs";
import { homedir } from "node:os";
import { join, basename } from "node:path";
import { executeCli, spawnCliAsync, type OnProgress } from "../shared/cli-executor.js";
import type { ExecutionResult } from "../shared/types.js";
import {
  getCurrentModel,
  getAvailableModels,
  reportExhausted,
  isQuotaError,
  getQuotaStatus,
} from "./model-fallback.js";

// ─── Constants ───────────────────────────────────────────────────────────────

export const GEMINI_CLI = process.env.GEMINI_CLI_PATH || "gemini";

// ─── Exported Types ──────────────────────────────────────────────────────────

export interface CliOptions {
  baseArgs: string[];
  stdin?: string;
  cwd?: string;
  timeout_ms?: number;
  onProgress?: OnProgress;
}

export interface GeminiSession {
  id: string;
  sessionDir: string;   // unique tmpdir — scopes Gemini's session storage
  cwd: string;          // project directory for context
  created_at: number;
  last_used: number;
  status: "idle" | "busy" | "dead";
  log_written: boolean; // prevents double-write on retry/double-dismiss
}

/**
 * OracleRuntimeBridge — the interface oracle-tools.ts and tools.ts use
 * to interact with daemon lifecycle without touching internal state.
 */
export interface OracleRuntimeBridge {
  spawnDaemon(input: {
    session_name?: string;
    cwd?: string;
    timeout_ms?: number;
    onProgress?: OnProgress;
  }): Promise<{ daemon_id: string; resumed: boolean; session_dir: string }>;

  askDaemon(input: {
    daemon_id: string;
    question: string;
    timeout_ms?: number;
    onProgress?: OnProgress;
  }): Promise<{ text: string; chars_in: number; chars_out: number }>;

  dismissDaemon(input: {
    daemon_id: string;
    hard?: boolean;
  }): Promise<void>;

  getDaemonSessionDir(daemon_id: string): string | null;

  findDaemonBySessionName(session_name: string): {
    daemon_id: string;
    session_dir: string;
  } | null;
}

// ─── Model-aware execution helpers ──────────────────────────────────────────
// These are used by both the runtime (daemon lifecycle) and tools.ts
// (send_message, summarize_transcript, dismiss session log writing).

/**
 * Execute Gemini CLI with automatic model fallback on quota exhaustion.
 * Tries each available model in the chain until one succeeds or all are exhausted.
 */
export async function executeWithFallback(opts: CliOptions): Promise<ExecutionResult> {
  const models = getAvailableModels();
  let lastResult: ExecutionResult | null = null;

  for (const model of models) {
    const result = await executeCli({
      command: GEMINI_CLI,
      args: ["--model", model, ...opts.baseArgs],
      stdin: opts.stdin,
      cwd: opts.cwd,
      timeout_ms: opts.timeout_ms,
      onProgress: opts.onProgress,
    });

    lastResult = result;

    if (result.success) return result;

    if (isQuotaError(result.stderr, result.stdout)) {
      reportExhausted(model);
      continue;
    }

    // Non-quota failure — return immediately
    return result;
  }

  // All models exhausted
  if (lastResult) {
    lastResult.stderr = `All models quota-exhausted. Chain: ${getAvailableModels().join(", ")}. Quota resets ~1 hour after exhaustion.\n\nFinal error:\n${lastResult.stderr}`;
    return lastResult;
  }

  return {
    success: false,
    stdout: "",
    stderr: `All models quota-exhausted. Chain: ${getAvailableModels().join(", ") || "none"}. Quota resets ~1 hour after exhaustion.`,
    exit_code: 1,
    duration_ms: 0,
    timed_out: false,
    retried: false,
    command: `${GEMINI_CLI} ${opts.baseArgs.join(" ")}`,
  };
}

/**
 * Async (SYN/ACK) spawn with transparent model fallback.
 * Returns the initial process immediately (for ACK/PID), but the result promise
 * will retry with the next model in the background if quota is hit.
 */
export function spawnWithFallback(opts: CliOptions): {
  process: import("node:child_process").ChildProcess;
  result: Promise<ExecutionResult>;
} {
  const model = getCurrentModel();

  const { process: proc, result: firstAttempt } = spawnCliAsync({
    command: GEMINI_CLI,
    args: ["--model", model, ...opts.baseArgs],
    stdin: opts.stdin,
    cwd: opts.cwd,
    timeout_ms: opts.timeout_ms,
    onProgress: opts.onProgress,
  });

  const result = firstAttempt.then(async (r) => {
    if (!r.success && isQuotaError(r.stderr, r.stdout)) {
      reportExhausted(model);

      const nextModels = getAvailableModels();
      if (nextModels.length === 1 && nextModels[0] === model) {
        r.stderr = `All models quota-exhausted. Chain: ${model}. Quota resets ~1 hour after exhaustion.\n\nFinal error:\n${r.stderr}`;
        return r;
      }

      return executeWithFallback({ ...opts });
    }
    return r;
  });

  return { process: proc, result };
}

// ─── GeminiRuntime Singleton ─────────────────────────────────────────────────

class GeminiRuntime implements OracleRuntimeBridge {
  private _sessions = new Map<string, GeminiSession>();
  private _sessionCounter = 0;

  /** Decommission tokens — in-memory only, never persisted to disk (FEAT-035) */
  readonly decommissionTokens = new Map<
    string,
    { token: string; oracle_name: string; expires_at: number }
  >();

  /** Protects all async read-modify-write sequences on pool state (decision #47) */
  readonly toolMutex = new Mutex();

  /** Sweeps oracle pools for idle members every 60s (FEAT-022, no-op until Phase 2) */
  idleSweepInterval: NodeJS.Timeout;

  /** Polls process.ppid every 5s to detect parent death (decision #48) */
  ppidWatchdog: NodeJS.Timeout;

  constructor() {
    // Start idle sweep — currently a no-op, oracle pools don't exist yet
    this.idleSweepInterval = setInterval(() => this._sweepIdleSessions(), 60_000);
    this.idleSweepInterval.unref(); // Don't prevent process exit

    // Start PPID watchdog — detect parent (Claude Code) death
    const parentPid = process.ppid;
    this.ppidWatchdog = setInterval(() => {
      if (process.ppid !== parentPid) {
        this._onParentDeath();
      }
    }, 5_000);
    this.ppidWatchdog.unref(); // Don't prevent process exit

    // Startup orphan sweep
    this._sweepOrphanSessions();

    // Shutdown hooks — clear intervals
    const cleanup = () => {
      clearInterval(this.idleSweepInterval);
      clearInterval(this.ppidWatchdog);
    };
    process.on("SIGTERM", cleanup);
    process.on("SIGINT", cleanup);
  }

  // ─── Private Helpers ─────────────────────────────────────────────────────

  private _genSessionId(): string {
    return `gd_${Date.now().toString(36)}_${++this._sessionCounter}`;
  }

  private _sanitizeSessionName(name: string): string {
    return (
      name
        .toLowerCase()
        .replace(/[^a-z0-9-]/g, "-")
        .replace(/-+/g, "-")
        .replace(/^-/, "")
        .slice(0, 40)
        .replace(/-$/, "") || "session"
    );
  }

  private _cleanupSession(session: GeminiSession): void {
    // Remove session marker dir. Native transcripts preserved for soft dismiss
    // (30-day retention per SESSION_LOG_SPEC). Hard dismiss handles tmp separately.
    try {
      if (existsSync(session.sessionDir)) {
        rmSync(session.sessionDir, { recursive: true, force: true });
      }
    } catch {
      /* non-fatal */
    }
  }

  private _sweepIdleSessions(): void {
    // FEAT-022: Sweep all oracle pools for idle members and soft-dismiss them.
    // Reads registry + state files directly (cannot import oracle-tools.ts — circular dep).
    const registryPath = process.env.PYTHIA_REGISTRY_PATH ||
      join(homedir(), ".pythia", "registry.json");

    let registry: { oracles?: Record<string, { oracle_dir: string; decommissioned_at?: string }> };
    try {
      registry = JSON.parse(readFileSync(registryPath, "utf-8"));
    } catch {
      return; // registry doesn't exist yet — nothing to sweep
    }

    const oracles = registry.oracles ?? {};
    const now = Date.now();

    for (const entry of Object.values(oracles)) {
      if (entry.decommissioned_at) continue;
      const stateFile = join(entry.oracle_dir, "state.json");

      let state: {
        daemon_pool?: Array<{
          daemon_id: string | null;
          session_name: string;
          status: string;
          last_query_at: string | null;
          idle_timeout_ms?: number;
        }>;
        oracle_name?: string;
        state_version?: number;
      };
      try {
        state = JSON.parse(readFileSync(stateFile, "utf-8"));
      } catch {
        continue; // unreadable state — skip
      }

      if (!state.daemon_pool) continue;

      let mutated = false;
      const DEFAULT_IDLE_MS = 300_000; // 5 minutes

      for (const member of state.daemon_pool) {
        if (member.status === "dismissed" || member.status === "dead") continue;
        if (!member.daemon_id) continue;
        if (!member.last_query_at) continue;

        const idleTimeoutMs = member.idle_timeout_ms ?? DEFAULT_IDLE_MS;
        const idleElapsedMs = now - Date.parse(member.last_query_at);

        if (idleElapsedMs > idleTimeoutMs) {
          const elapsedMinutes = Math.round(idleElapsedMs / 60_000);
          console.error(
            `[pythia] Idle-dismissed pool member ${member.session_name} ` +
            `for oracle ${state.oracle_name ?? "?"} (idle ${elapsedMinutes}m)`,
          );

          // Soft dismiss — best-effort, swallow errors
          void this.dismissDaemon({ daemon_id: member.daemon_id, hard: false }).catch(() => {});

          member.status = "dismissed";
          member.daemon_id = null;
          mutated = true;
        }
      }

      // Atomic write state back (simplified — no CAS needed for sweep; last-write-wins is fine)
      if (mutated) {
        try {
          const updated = {
            ...state,
            state_version: (state.state_version ?? 0) + 1,
            updated_at: new Date().toISOString(),
          };
          const tmp = stateFile + ".sweep.tmp";
          writeFileSync(tmp, JSON.stringify(updated, null, 2) + "\n", "utf-8");
          renameSync(tmp, stateFile);
        } catch { /* non-fatal */ }
      }
    }
  }

  private _onParentDeath(): void {
    // Parent process died — Claude Code was SIGKILL'd.
    // Mark all busy sessions as dead. In-flight gemini CLI processes will
    // finish on their own (they're separate PIDs), but their results will be lost.
    for (const session of this._sessions.values()) {
      if (session.status === "busy") {
        session.status = "dead";
      }
    }
    clearInterval(this.ppidWatchdog);
    clearInterval(this.idleSweepInterval);
  }

  private _sweepOrphanSessions(): void {
    // Startup orphan sweep: check for session dirs from previous crashes.
    // These are daemon-sessions/ entries with no in-memory session.
    // We don't delete them — they may be intentionally hibernated.
    // Future: check for PID files and kill orphaned processes.
  }

  // ─── Public Accessors ────────────────────────────────────────────────────
  // Used by tools.ts for status checks, list_daemons formatting, etc.

  getSession(daemonId: string): GeminiSession | undefined {
    return this._sessions.get(daemonId);
  }

  getSessions(): ReadonlyMap<string, GeminiSession> {
    return this._sessions;
  }

  /**
   * Mark a session's log as written. Called by tools.ts after session log write
   * to prevent double-writes on retry or double-dismiss.
   */
  setSessionLogWritten(daemonId: string): void {
    const session = this._sessions.get(daemonId);
    if (session) session.log_written = true;
  }

  // ─── Bridge Methods ──────────────────────────────────────────────────────

  async spawnDaemon(input: {
    session_name?: string;
    cwd?: string;
    timeout_ms?: number;
    onProgress?: OnProgress;
  }): Promise<{ daemon_id: string; resumed: boolean; session_dir: string }> {
    // Phase 1: Under mutex — check for existing, prepare session metadata
    const prepared = await this.toolMutex.runExclusive(() => {
      const projectDir = input.cwd || process.cwd();
      const daemonSessionsDir = join(homedir(), ".gemini", "daemon-sessions");
      mkdirSync(daemonSessionsDir, { recursive: true });

      const sessionId = this._genSessionId();
      const sessionDirName = input.session_name
        ? `daemon-${this._sanitizeSessionName(input.session_name)}`
        : `daemon-${sessionId}`;
      const sessionDir = join(daemonSessionsDir, sessionDirName);

      // Guard: already active in memory
      const existingActive = Array.from(this._sessions.values()).find(
        (s) => s.sessionDir === sessionDir
      );
      if (existingActive) {
        return {
          type: "existing" as const,
          daemon_id: existingActive.id,
          session_dir: sessionDir,
        };
      }

      // Check for resume (session + tmp dirs both exist on disk)
      const geminiTmpDir = join(homedir(), ".gemini", "tmp", sessionDirName);
      const isResuming = existsSync(sessionDir) && existsSync(geminiTmpDir);

      if (isResuming) {
        this._sessions.set(sessionId, {
          id: sessionId,
          sessionDir,
          cwd: projectDir,
          created_at: Date.now(),
          last_used: Date.now(),
          status: "idle",
          log_written: false,
        });
        return {
          type: "resumed" as const,
          daemon_id: sessionId,
          session_dir: sessionDir,
        };
      }

      // New session — create dir (bootstrap happens outside mutex)
      mkdirSync(sessionDir, { recursive: true });
      return {
        type: "new" as const,
        daemon_id: sessionId,
        session_dir: sessionDir,
        sessionDirName,
        projectDir,
      };
    });

    // Fast returns for existing/resumed sessions
    if (prepared.type === "existing") {
      return { daemon_id: prepared.daemon_id, resumed: true, session_dir: prepared.session_dir };
    }
    if (prepared.type === "resumed") {
      return { daemon_id: prepared.daemon_id, resumed: true, session_dir: prepared.session_dir };
    }

    // Phase 2: Outside mutex — bootstrap (can take 30-60s)
    const result = await executeWithFallback({
      baseArgs: [
        "-p", "", "--output-format", "text",
        "--approval-mode", "yolo",
        "--include-directories", homedir(),
      ],
      stdin:
        "You are a helpful research and coding assistant. I will send follow-up questions. " +
        "Acknowledge with: Ready.\n\n" +
        "IMPORTANT: When asked to write a session log at the end of this session, write the " +
        "markdown file to the exact path provided. Do not run git commands — the system handles " +
        "that after you write the file. Follow the SESSION_LOG_SPEC format that will be included " +
        "in the request.",
      cwd: prepared.session_dir,
      timeout_ms: input.timeout_ms || 60_000,
      onProgress: input.onProgress,
    });

    if (!result.success) {
      try {
        rmSync(prepared.session_dir, { recursive: true, force: true });
      } catch {
        /* non-fatal */
      }
      throw new Error(`Failed to start Gemini session:\n${result.stderr || result.stdout}`);
    }

    // Phase 3: Under mutex — register the session
    await this.toolMutex.runExclusive(() => {
      this._sessions.set(prepared.daemon_id, {
        id: prepared.daemon_id,
        sessionDir: prepared.session_dir,
        cwd: prepared.projectDir,
        created_at: Date.now(),
        last_used: Date.now(),
        status: "idle",
        log_written: false,
      });
    });

    return {
      daemon_id: prepared.daemon_id,
      resumed: false,
      session_dir: prepared.session_dir,
    };
  }

  async askDaemon(input: {
    daemon_id: string;
    question: string;
    timeout_ms?: number;
    onProgress?: OnProgress;
  }): Promise<{ text: string; chars_in: number; chars_out: number }> {
    // Phase 1: Under mutex — validate and set busy
    const session = await this.toolMutex.runExclusive(() => {
      const s = this._sessions.get(input.daemon_id);
      if (!s) throw new Error(`Daemon not found: ${input.daemon_id}`);
      if (s.status === "busy") {
        throw new Error(
          `Daemon ${input.daemon_id} is busy — wait for the current request to complete.`
        );
      }
      // Accept both "idle" and "dead" — dead triggers implicit revive attempt
      s.status = "busy";
      s.last_used = Date.now();
      return s;
    });

    // Phase 2: Outside mutex — execute CLI (can take minutes)
    try {
      const result = await executeWithFallback({
        baseArgs: [
          "-r", "latest", "-p", "",
          "--output-format", "text",
          "--approval-mode", "yolo",
          "--include-directories", homedir(),
        ],
        stdin: input.question,
        cwd: session.sessionDir,
        timeout_ms: input.timeout_ms || 120_000,
        onProgress: input.onProgress,
      });

      // Phase 3: Under mutex — update status
      await this.toolMutex.runExclusive(() => {
        session.status = result.success ? "idle" : "dead";
      });

      if (!result.success) {
        if (isQuotaError(result.stderr, result.stdout)) {
          throw new Error(
            `Quota exhausted on all available models. ${getQuotaStatus()}\n\nQuota resets in ~1 hour. Dismiss the daemon.`
          );
        }
        throw new Error(`Gemini failed:\n${result.stderr || result.stdout}`);
      }

      return {
        text: result.stdout,
        chars_in: input.question.length,
        chars_out: result.stdout.length,
      };
    } catch (err) {
      // Ensure session is marked dead on any failure
      await this.toolMutex
        .runExclusive(() => {
          session.status = "dead";
        })
        .catch(() => {});
      throw err;
    }
  }

  async dismissDaemon(input: { daemon_id: string; hard?: boolean }): Promise<void> {
    await this.toolMutex.runExclusive(() => {
      const session = this._sessions.get(input.daemon_id);
      if (!session) return; // Already dismissed — idempotent

      if (session.status === "busy") {
        throw new Error(
          `Daemon ${input.daemon_id} is busy — wait for the current request to complete before dismissing.`
        );
      }

      // Remove from active sessions
      this._sessions.delete(input.daemon_id);

      if (input.hard) {
        // Hard dismiss: delete marker dir AND native transcripts. Cannot be resumed.
        this._cleanupSession(session);
        try {
          const geminiTmpDir = join(
            homedir(),
            ".gemini",
            "tmp",
            basename(session.sessionDir)
          );
          if (existsSync(geminiTmpDir)) {
            rmSync(geminiTmpDir, { recursive: true, force: true });
          }
        } catch {
          /* non-fatal */
        }
      }
      // Soft dismiss (default): session files preserved for future resumption
    });
  }

  getDaemonSessionDir(daemonId: string): string | null {
    const session = this._sessions.get(daemonId);
    return session ? session.sessionDir : null;
  }

  findDaemonBySessionName(sessionName: string): {
    daemon_id: string;
    session_dir: string;
  } | null {
    const sanitized = this._sanitizeSessionName(sessionName);
    const targetDir = `daemon-${sanitized}`;
    for (const session of this._sessions.values()) {
      if (basename(session.sessionDir) === targetDir) {
        return { daemon_id: session.id, session_dir: session.sessionDir };
      }
    }
    return null;
  }
}

// ─── Singleton ───────────────────────────────────────────────────────────────

let _instance: GeminiRuntime | null = null;

/**
 * Returns the singleton GeminiRuntime instance.
 * Created on first call — the singleton lives for the lifetime of the MCP server process.
 */
export function getGeminiRuntime(): GeminiRuntime {
  if (!_instance) {
    _instance = new GeminiRuntime();
  }
  return _instance;
}
