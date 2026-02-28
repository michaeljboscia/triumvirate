/**
 * Gemini MCP server tool definitions.
 *
 * Tools:
 *   send_message         - Send structured inter-agent request to Gemini CLI (returns ACK immediately)
 *   get_response         - Wait for and retrieve the response from a sent message
 *   spawn_daemon         - Spawn a persistent Gemini session for multi-turn interaction
 *   ask_daemon           - Send a question to a running Gemini daemon
 *   dismiss_daemon       - Clean up a Gemini daemon session
 *   list_daemons         - List active Gemini daemon sessions
 *   list_scratchpad      - List inter-agent scratchpad files
 *   write_scratchpad     - Write to the inter-agent scratchpad
 *   list_jobs            - List active Gemini jobs
 *   summarize_transcript - Summarize transcript text (for pre-compact hooks)
 *
 * Daemon mode uses `gemini -p "" --output-format text --include-directories ~` with a
 * unique session dir per daemon under ~/.gemini/daemon-sessions/. Subsequent asks use
 * `gemini -r latest -p "" --include-directories ~` in that same dir. Session isolation
 * is preserved via unique cwd; --include-directories expands file access to all of home.
 * No PTY, no sentinel protocol, no pre-warming needed.
 */

import { z } from "zod";
import { mkdtempSync, mkdirSync, rmSync, existsSync } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { join, basename } from "node:path";
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { detectContext } from "../shared/context-detector.js";
import { findLatestSessionLog } from "../shared/session-log-finder.js";
import { formatMessage, validateContext } from "../shared/message-formatter.js";
import { executeCli, formatError, spawnCliAsync, type OnProgress } from "../shared/cli-executor.js";
import type { ExecutionResult } from "../shared/types.js";
import { logToOutbox } from "../shared/outbox-logger.js";
import { createJob, completeJob, getJob, waitForJob, listJobs } from "../shared/job-store.js";
import {
  ensureScratchpad,
  sweepScratchpad,
  reapDaemonFiles,
  listScratchpad,
  writeScratchpad,
} from "../shared/scratchpad-reaper.js";
import {
  DEFAULT_TIMEOUT_MS,
  MAX_TIMEOUT_MS,
  REQUEST_TYPES,
} from "../shared/types.js";
import {
  getCurrentModel,
  getAvailableModels,
  reportExhausted,
  isQuotaError,
  getQuotaStatus,
} from "./model-fallback.js";

const GEMINI_CLI = process.env.GEMINI_CLI_PATH || "gemini" // set GEMINI_CLI_PATH env var if gemini is not in PATH;

// ─── Model-aware execution helpers ───────────────────────────────────────────

interface CliOptions {
  baseArgs: string[];
  stdin?: string;
  cwd?: string;
  timeout_ms?: number;
  onProgress?: OnProgress;
}

/**
 * Execute Gemini CLI with automatic model fallback on quota exhaustion.
 * Tries each available model in the chain until one succeeds or all are exhausted.
 */
async function executeWithFallback(opts: CliOptions): Promise<ExecutionResult> {
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
      // Continue to next model in chain
      continue;
    }

    // Non-quota failure — return immediately (no point retrying with different model)
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
 * The job ID registered by the caller remains stable throughout.
 */
function spawnWithFallback(opts: CliOptions): {
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

  // Wrap the result: if first attempt hits quota, fall back synchronously
  const result = firstAttempt.then(async (r) => {
    if (!r.success && isQuotaError(r.stderr, r.stdout)) {
      reportExhausted(model);
      
      const nextModels = getAvailableModels();
      // If the only available model is the one we just failed on (because getAvailableModels guarantees at least one),
      // do not blindly retry it. Just return the exhaustion failure.
      if (nextModels.length === 1 && nextModels[0] === model) {
        r.stderr = `All models quota-exhausted. Chain: ${model}. Quota resets ~1 hour after exhaustion.\n\nFinal error:\n${r.stderr}`;
        return r;
      }
      
      // Retry with the updated model chain (executeWithFallback skips exhausted)
      return executeWithFallback({ ...opts });
    }
    return r;
  });

  return { process: proc, result };
}

// ─── Gemini Session Store ─────────────────────────────────────────────────────
// Daemon mode: stateless subprocesses with session continuity via `-r latest`.
// Each daemon gets a unique tmpdir; Gemini stores sessions keyed by that dirname.
// No PTY, no sentinel protocol, no TUI fighting.

interface GeminiSession {
  id: string;
  sessionDir: string;   // unique tmpdir — scopes Gemini's session storage
  cwd: string;          // project directory for context
  created_at: number;
  last_used: number;
  status: "idle" | "busy" | "dead";
}

let _sessionCounter = 0;
const _sessions = new Map<string, GeminiSession>();

function _genSessionId(): string {
  return `gd_${Date.now().toString(36)}_${++_sessionCounter}`;
}

function _cleanupSession(session: GeminiSession): void {
  // Remove session tmpdir
  try {
    if (existsSync(session.sessionDir)) {
      rmSync(session.sessionDir, { recursive: true, force: true });
    }
  } catch { /* non-fatal */ }

  // Remove Gemini's session files stored at ~/.gemini/tmp/<dirname>/
  try {
    const dirName = basename(session.sessionDir);
    const geminiSessionDir = join(homedir(), ".gemini", "tmp", dirName);
    if (existsSync(geminiSessionDir)) {
      rmSync(geminiSessionDir, { recursive: true, force: true });
    }
  } catch { /* non-fatal */ }
}

/** Build a progress callback that sends MCP logging messages */
function makeProgressLogger(server: McpServer, target: string): OnProgress {
  return (event) => {
    switch (event.type) {
      case "spawned":
        server.sendLoggingMessage({
          level: "info",
          data: `SPAWNED: Started ${target} CLI (pid ${event.pid}). Writing message...`,
        });
        break;
      case "heartbeat":
        server.sendLoggingMessage({
          level: "info",
          data: `WORKING: ${target} is processing... (${Math.round(event.elapsed_ms / 1000)}s elapsed)`,
        });
        break;
      case "stdout_data":
        server.sendLoggingMessage({
          level: "info",
          data: `RESPONDING: ${target} is sending data back...`,
        });
        break;
      case "timeout":
        server.sendLoggingMessage({
          level: "warning",
          data: `TIMEOUT: Sending ${event.action} to ${target} after ${Math.round(event.elapsed_ms / 1000)}s`,
        });
        break;
      case "retry":
        server.sendLoggingMessage({
          level: "warning",
          data: `RETRY: First attempt failed, retrying (attempt ${event.attempt})...`,
        });
        break;
      case "done":
        server.sendLoggingMessage({
          level: event.success ? "info" : "error",
          data: event.success
            ? `DONE: ${target} responded in ${Math.round(event.elapsed_ms / 1000)}s`
            : `FAILED: ${target} did not respond successfully (${Math.round(event.elapsed_ms / 1000)}s)`,
        });
        break;
    }
  };
}

export function registerGeminiTools(server: McpServer): void {
  // ─── send_message ──────────────────────────────────────────────
  server.tool(
    "send_message",
    "Send a structured inter-agent request to Gemini. Returns IMMEDIATELY with a job_id confirming Gemini CLI was spawned (SYN/ACK). Use get_response(job_id) to retrieve the actual response. Auto-populates protocol headers. IMPORTANT: After receiving the ACK, you MUST call get_response with the returned job_id to get Gemini's answer.",
    {
      request_type: z
        .enum(REQUEST_TYPES)
        .describe(
          "Type of request: question, review, debug, architecture, or other"
        ),
      question: z
        .string()
        .min(1)
        .describe(
          "The specific question or request for Gemini (1-3 sentences recommended)"
        ),
      context: z
        .string()
        .optional()
        .describe(
          "Optional brief context summary. Full context should be in the session log."
        ),
      cwd: z
        .string()
        .optional()
        .describe(
          "Working directory for context detection. Defaults to the MCP server's cwd."
        ),
      session_log: z
        .string()
        .optional()
        .describe(
          "Override session log path (relative from repo root). Auto-detected if omitted."
        ),
      timeout_ms: z
        .number()
        .min(1000)
        .max(MAX_TIMEOUT_MS)
        .optional()
        .describe(
          `Timeout in milliseconds (default: ${DEFAULT_TIMEOUT_MS}, max: ${MAX_TIMEOUT_MS})`
        ),
    },
    async (params) => {
      const progress = makeProgressLogger(server, "Gemini");

      // Detect context
      const ctx = detectContext("gemini", params.cwd);

      // Find or use provided session log
      if (params.session_log) {
        ctx.session_log = params.session_log;
      } else {
        ctx.session_log = findLatestSessionLog(ctx.cwd, ctx.taxonomy);
      }

      // Validate context
      const errors = validateContext(ctx);
      if (errors.length > 0) {
        return {
          content: [
            {
              type: "text" as const,
              text: `Context validation failed:\n${errors.map((e) => `  - ${e}`).join("\n")}\n\nDetected context:\n${JSON.stringify(ctx, null, 2)}`,
            },
          ],
          isError: true,
        };
      }

      // Format the protocol message
      const message = formatMessage({
        context: ctx,
        request_type: params.request_type,
        question: params.question,
        additional_context: params.context,
      });

      // Spawn Gemini CLI asynchronously — returns immediately.
      // spawnWithFallback injects --model and retries transparently on quota exhaustion.
      const { process: proc, result: resultPromise } = spawnWithFallback({
        baseArgs: ["-p", "", "--output-format", "text"],
        stdin: message,
        timeout_ms: params.timeout_ms || DEFAULT_TIMEOUT_MS,
        onProgress: progress,
      });

      // Check that the process actually spawned
      if (!proc.pid) {
        return {
          content: [
            {
              type: "text" as const,
              text: `SPAWN FAILED: Could not start Gemini CLI at ${GEMINI_CLI}. Check that the binary exists and is executable.`,
            },
          ],
          isError: true,
        };
      }

      // Register the job
      let jobId: string;
      try {
        jobId = createJob("gemini", proc);
      } catch (err: any) {
        proc.kill("SIGTERM");
        return {
          content: [
            {
              type: "text" as const,
              text: err.message,
            },
          ],
          isError: true,
        };
      }

      // Wire up completion: when the CLI finishes, update the job store and log to outbox
      resultPromise.then((result) => {
        completeJob(jobId, result);
        logToOutbox({
          timestamp: ctx.timestamp,
          target: "gemini",
          request_type: params.request_type,
          question: params.question,
          message,
          result: result.success ? "sent" : result.timed_out ? "timeout" : "failed",
          response: result.success ? result.stdout : undefined,
          error: !result.success ? formatError(result) : undefined,
          duration_ms: result.duration_ms,
        });
      });

      // Return ACK immediately — Gemini is running
      return {
        content: [
          {
            type: "text" as const,
            text: `ACK: Gemini CLI spawned (pid ${proc.pid}), processing your ${params.request_type} request.\n\nJob ID: ${jobId}\n\nCall get_response("${jobId}") to retrieve Gemini's answer.`,
          },
        ],
      };
    }
  );

  // ─── get_response ──────────────────────────────────────────────
  server.tool(
    "get_response",
    "Wait for and retrieve the response from a previously sent inter-agent message. Pass the job_id returned by send_message. Blocks until the sibling agent responds or times out.",
    {
      job_id: z
        .string()
        .min(1)
        .describe("The job ID returned by send_message"),
      timeout_ms: z
        .number()
        .min(1000)
        .max(MAX_TIMEOUT_MS)
        .optional()
        .describe(
          "How long to wait for the response (default: waits until the job's original timeout)"
        ),
    },
    async (params) => {
      // Check if job exists
      const current = getJob(params.job_id);
      if (!current) {
        return {
          content: [
            {
              type: "text" as const,
              text: `Job not found: ${params.job_id}\n\nThe job may have expired (jobs expire after 10 minutes) or the ID may be incorrect.\n\nActive jobs: ${JSON.stringify(listJobs().map((j) => ({ id: j.id, target: j.target, status: j.status, elapsed: Math.round(j.elapsed_ms / 1000) + "s" })), null, 2)}`,
            },
          ],
          isError: true,
        };
      }

      // If already done, return immediately
      if (current.status !== "running") {
        return formatJobResponse(current);
      }

      // Wait for completion
      const completed = await waitForJob(params.job_id, params.timeout_ms);
      if (!completed) {
        return {
          content: [
            {
              type: "text" as const,
              text: `Job ${params.job_id} disappeared while waiting. This shouldn't happen.`,
            },
          ],
          isError: true,
        };
      }

      return formatJobResponse(completed);
    }
  );

  // ─── spawn_daemon ───────────────────────────────────────────────
  server.tool(
    "spawn_daemon",
    "Spawn a persistent Gemini session for multi-turn interaction. Uses `gemini -p --output-format text` in a unique session directory. Subsequent ask_daemon() calls use `gemini -r latest` to continue the conversation. Each ask is a fresh process (~2-3s) with full conversation continuity. Returns a daemon_id.",
    {
      cwd: z
        .string()
        .optional()
        .describe("Working directory for context. Defaults to MCP server cwd."),
      timeout_ms: z
        .number()
        .min(5000)
        .max(120_000)
        .optional()
        .describe("Timeout for initial session creation (default: 60s)."),
    },
    async (params) => {
      const projectDir = params.cwd || process.cwd();
      const swept = sweepScratchpad(projectDir);
      ensureScratchpad(projectDir);

      // Create a unique session directory under ~ so Gemini's workspace root
      // expands to user home directory, allowing reads of any file under home.
      // (tmpdir() resolves to /private/var/folders/... which Gemini can't traverse out of)
      const daemonSessionsDir = join(homedir(), ".gemini", "daemon-sessions");
      mkdirSync(daemonSessionsDir, { recursive: true });
      const sessionDir = mkdtempSync(join(daemonSessionsDir, "daemon-"));

      const result = await executeWithFallback({
        baseArgs: ["-p", "", "--output-format", "text", "--approval-mode", "yolo", "--include-directories", homedir()],
        stdin: "You are a helpful research and coding assistant. I will send follow-up questions. Acknowledge with: Ready.",
        cwd: homedir(),
        timeout_ms: params.timeout_ms || 60_000,
        onProgress: makeProgressLogger(server, "Gemini"),
      });

      if (!result.success) {
        try { rmSync(sessionDir, { recursive: true, force: true }); } catch { /* non-fatal */ }
        return {
          content: [{ type: "text" as const, text: `Failed to start Gemini session:\n${formatError(result)}` }],
          isError: true,
        };
      }

      const sessionId = _genSessionId();
      _sessions.set(sessionId, {
        id: sessionId,
        sessionDir,
        cwd: projectDir,
        created_at: Date.now(),
        last_used: Date.now(),
        status: "idle",
      });

      const sweptNote = swept.length > 0 ? `\n\nReaper swept ${swept.length} stale scratchpad file(s).` : "";
      return {
        content: [
          {
            type: "text" as const,
            text:
              `Gemini daemon ready.\n\n` +
              `Daemon ID: ${sessionId}\n` +
              `Scratchpad: ${projectDir}/.claude/scratchpad/\n\n` +
              `Use ask_daemon("${sessionId}", "your question") to interact.\n` +
              `Use dismiss_daemon("${sessionId}") when done.` +
              sweptNote,
          },
        ],
      };
    }
  );

  // ─── ask_daemon ─────────────────────────────────────────────────
  server.tool(
    "ask_daemon",
    "Send a question to a running Gemini daemon and wait for its response. Uses `gemini -r latest --output-format text` to continue the session with full conversation history. Each call is a fresh process (~2-3s). The daemon must be idle.",
    {
      daemon_id: z.string().min(1).describe("The daemon ID returned by spawn_daemon"),
      question: z.string().min(1).describe("The question or instruction to send to the daemon"),
      timeout_ms: z
        .number()
        .min(1000)
        .max(MAX_TIMEOUT_MS)
        .optional()
        .describe("How long to wait for a response (default: 2 minutes)"),
    },
    async (params) => {
      const session = _sessions.get(params.daemon_id);
      if (!session) {
        return { content: [{ type: "text" as const, text: `Daemon not found: ${params.daemon_id}` }], isError: true };
      }
      if (session.status === "dead") {
        return { content: [{ type: "text" as const, text: `Daemon ${params.daemon_id} is dead.` }], isError: true };
      }
      if (session.status === "busy") {
        return { content: [{ type: "text" as const, text: `Daemon ${params.daemon_id} is busy — wait for the current request to complete.` }], isError: true };
      }

      session.status = "busy";
      session.last_used = Date.now();

      try {
        const result = await executeWithFallback({
          baseArgs: ["-r", "latest", "-p", "", "--output-format", "text", "--approval-mode", "yolo", "--include-directories", homedir()],
          stdin: params.question,
          cwd: homedir(),
          timeout_ms: params.timeout_ms || 120_000,
          onProgress: makeProgressLogger(server, "Gemini"),
        });

        session.status = "idle";

        if (!result.success) {
          // If we still get a quota error after executeWithFallback, it means ALL models are exhausted.
          // The daemon is unusable until quota resets.
          if (isQuotaError(result.stderr, result.stdout)) {
            session.status = "dead";
            return {
              content: [{
                type: "text" as const,
                text: `Quota exhausted on all available models. ${getQuotaStatus()}\n\nQuota resets in ~1 hour. Dismiss the daemon.`,
              }],
              isError: true,
            };
          }
          session.status = "dead";
          return { content: [{ type: "text" as const, text: `Gemini failed:\n${formatError(result)}` }], isError: true };
        }

        return {
          content: [{ type: "text" as const, text: `Gemini daemon response:\n\n${result.stdout}` }],
        };
      } catch (err: any) {
        session.status = "dead";
        return { content: [{ type: "text" as const, text: `ask_daemon failed: ${err.message}` }], isError: true };
      }
    }
  );

  // ─── dismiss_daemon ─────────────────────────────────────────────
  server.tool(
    "dismiss_daemon",
    "Dismiss a Gemini daemon — removes it from the active session registry and cleans up session files.",
    {
      daemon_id: z.string().min(1).describe("The daemon ID to dismiss"),
    },
    async (params) => {
      const session = _sessions.get(params.daemon_id);
      if (!session) {
        return {
          content: [
            {
              type: "text" as const,
              text: `Daemon not found: ${params.daemon_id}. It may have already been dismissed.`,
            },
          ],
        };
      }
      const reaped = reapDaemonFiles(session.cwd, params.daemon_id);
      _cleanupSession(session);
      _sessions.delete(params.daemon_id);
      const reapedNote = reaped.length > 0
        ? `\nReaper removed ${reaped.length} scratchpad file(s): ${reaped.join(", ")}`
        : "";
      return {
        content: [
          {
            type: "text" as const,
            text: `Gemini daemon ${params.daemon_id} dismissed. Session directory cleaned up.${reapedNote}`,
          },
        ],
      };
    }
  );

  // ─── list_daemons ───────────────────────────────────────────────
  server.tool(
    "list_daemons",
    "List all active Gemini daemon sessions with their status, age, and session directory. Non-blocking.",
    {},
    async () => {
      if (_sessions.size === 0) {
        return {
          content: [{ type: "text" as const, text: "No active Gemini daemons." }],
        };
      }
      const summary = Array.from(_sessions.values()).map((s) => ({
        id: s.id,
        status: s.status,
        sessionDir: s.sessionDir,
        age: `${Math.round((Date.now() - s.created_at) / 1000)}s`,
        cwd: s.cwd,
      }));
      return {
        content: [
          {
            type: "text" as const,
            text: `Active Gemini daemons (${_sessions.size}):\n\n${JSON.stringify(summary, null, 2)}`,
          },
        ],
      };
    }
  );

  // ─── list_scratchpad ────────────────────────────────────────────
  server.tool(
    "list_scratchpad",
    "List all files currently in the inter-agent scratchpad for a project. Shows filename, age, and size. The scratchpad is at <project-root>/.claude/scratchpad/ and is shared by all three agents.",
    {
      cwd: z
        .string()
        .optional()
        .describe("Project root to find the scratchpad. Defaults to MCP server cwd."),
    },
    async (params) => {
      const projectDir = params.cwd || process.cwd();
      sweepScratchpad(projectDir); // opportunistic Reaper pass
      const files = listScratchpad(projectDir);
      if (files.length === 0) {
        return {
          content: [{ type: "text" as const, text: `Scratchpad is empty (${projectDir}/.claude/scratchpad/).` }],
        };
      }
      const summary = files.map((f) => ({
        file: f.filename,
        age: `${Math.round(f.age_ms / 60_000)}m`,
        size: `${f.size_bytes}b`,
      }));
      return {
        content: [
          {
            type: "text" as const,
            text: `Scratchpad files (${files.length}):\n\n${JSON.stringify(summary, null, 2)}`,
          },
        ],
      };
    }
  );

  // ─── write_scratchpad ───────────────────────────────────────────
  server.tool(
    "write_scratchpad",
    "Write a progress note or artifact to the inter-agent scratchpad. All agents can read files written here. Use for status updates, findings, or intermediate results during a daemon session.",
    {
      topic: z
        .string()
        .min(1)
        .describe("Short topic label for the filename (e.g., 'pain-signal-analysis', 'progress-update')"),
      content: z
        .string()
        .min(1)
        .describe("Markdown content to write to the scratchpad file"),
      cwd: z
        .string()
        .optional()
        .describe("Project root to find the scratchpad. Defaults to MCP server cwd."),
    },
    async (params) => {
      const projectDir = params.cwd || process.cwd();
      try {
        const filepath = writeScratchpad(projectDir, "gemini", params.topic, params.content);
        return {
          content: [
            {
              type: "text" as const,
              text: `Written to scratchpad: ${filepath}`,
            },
          ],
        };
      } catch (err: any) {
        return {
          content: [{ type: "text" as const, text: `Failed to write scratchpad: ${err.message}` }],
          isError: true,
        };
      }
    }
  );

  // ─── list_jobs ─────────────────────────────────────────────────
  server.tool(
    "list_jobs",
    "List all active Gemini inter-agent jobs. Returns job ID, status (running/completed/failed/expired), elapsed time, and PID. Non-blocking — call anytime to check on outstanding requests.",
    {},
    async () => {
      const jobs = listJobs();
      if (jobs.length === 0) {
        return {
          content: [{ type: "text" as const, text: "No active Gemini jobs." }],
        };
      }
      const summary = jobs.map((j) => ({
        id: j.id,
        target: j.target,
        status: j.status,
        elapsed: `${Math.round(j.elapsed_ms / 1000)}s`,
        pid: j.pid,
      }));
      return {
        content: [
          {
            type: "text" as const,
            text: `Active Gemini jobs (${jobs.length}):\n\n${JSON.stringify(summary, null, 2)}`,
          },
        ],
      };
    }
  );

  // ─── summarize_transcript ──────────────────────────────────────
  server.tool(
    "summarize_transcript",
    "Send transcript text to Gemini for summarization. Useful for pre-compact hooks that need to compress conversation history before context window fills up. This tool blocks until complete (no async pattern — summarization is typically fast).",
    {
      transcript: z
        .string()
        .min(1)
        .describe("The transcript text to summarize"),
      output_format: z
        .enum(["markdown", "json", "plain"])
        .optional()
        .describe("Desired output format (default: markdown)"),
      timeout_ms: z
        .number()
        .min(1000)
        .max(MAX_TIMEOUT_MS)
        .optional()
        .describe(
          `Timeout in milliseconds (default: ${DEFAULT_TIMEOUT_MS}, max: ${MAX_TIMEOUT_MS})`
        ),
    },
    async (params) => {
      const progress = makeProgressLogger(server, "Gemini");
      const format = params.output_format || "markdown";
      const prompt = `Summarize the following conversation transcript. Focus on:
1. Key decisions made
2. Actions taken and their results
3. Outstanding items and next steps
4. Any errors or blockers encountered

Output format: ${format}

TRANSCRIPT:
${params.transcript}`;

      const result = await executeWithFallback({
        baseArgs: ["-p", "", "--output-format", "text"],
        stdin: prompt,
        timeout_ms: params.timeout_ms || DEFAULT_TIMEOUT_MS,
        onProgress: progress,
      });

      if (result.success) {
        return {
          content: [
            {
              type: "text" as const,
              text: result.stdout,
            },
          ],
        };
      }

      return {
        content: [
          {
            type: "text" as const,
            text: `Summarization failed.\n\n${formatError(result)}`,
          },
        ],
        isError: true,
      };
    }
  );
}

/** Format a completed job into an MCP tool response */
function formatJobResponse(job: ReturnType<typeof getJob>) {
  if (!job) {
    return {
      content: [{ type: "text" as const, text: "Job not found." }],
      isError: true,
    };
  }

  if (job.status === "running") {
    return {
      content: [
        {
          type: "text" as const,
          text: `STILL RUNNING: Gemini is still processing (${Math.round(job.elapsed_ms / 1000)}s elapsed, pid ${job.pid}).\n\nCall get_response("${job.id}") again to check.`,
        },
      ],
    };
  }

  if (job.status === "expired") {
    return {
      content: [
        {
          type: "text" as const,
          text: `EXPIRED: Job ${job.id} exceeded maximum lifetime and was terminated.`,
        },
      ],
      isError: true,
    };
  }

  if (job.result?.success) {
    return {
      content: [
        {
          type: "text" as const,
          text: `${job.result.stdout}\n\n---\n[Job: ${job.id}] [Duration: ${Math.round(job.result.duration_ms / 1000)}s]`,
        },
      ],
    };
  }

  return {
    content: [
      {
        type: "text" as const,
        text: `Failed to get response from Gemini.\n\n${job.result ? formatError(job.result) : "No result available."}\n\n[Job: ${job.id}]`,
      },
    ],
    isError: true,
  };
}
