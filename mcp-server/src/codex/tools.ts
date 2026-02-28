/**
 * Codex MCP server tool definitions.
 *
 * Tools:
 *   send_message  - Send structured inter-agent request to Codex (returns ACK immediately)
 *   get_response  - Wait for and retrieve the response from a sent message
 *   code_review   - Review code changes via `codex review` (blocking — scope flags, no stdin)
 *
 * CRITICAL DISTINCTION:
 *   send_message uses `codex exec` with stdin (general-purpose messaging, async ACK)
 *   code_review uses `codex review` with scope flags (git-aware code review, blocking)
 *   These are different Codex subcommands with different interfaces.
 */

import { z } from "zod";
import { existsSync } from "node:fs";
import { execSync } from "node:child_process";
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { detectContext } from "../shared/context-detector.js";
import { findLatestSessionLog } from "../shared/session-log-finder.js";
import { formatMessage, validateContext } from "../shared/message-formatter.js";
import { executeCli, formatError, spawnCliAsync, type OnProgress } from "../shared/cli-executor.js";
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

const CODEX_CLI = process.env.CODEX_CLI_PATH || "codex" // set CODEX_CLI_PATH env var if codex is not in PATH;

// ─── Thread-based session store ───────────────────────────────────────────────
// Codex daemon mode uses `codex exec --json` + `codex exec resume <thread_id> --json`.
// Codex persists conversation history to disk by thread ID — we just track the ID.
// No PTY, no sentinels, no TUI. Each ask spawns a fresh process (~7s) and exits cleanly.

interface CodexSession {
  id: string;
  thread_id: string;
  cwd: string;
  created_at: number;
  last_used: number;
  status: "idle" | "busy" | "dead";
}

let _sessionCounter = 0;
const _sessions = new Map<string, CodexSession>();

function _genSessionId(): string {
  return `cd_${Date.now().toString(36)}_${++_sessionCounter}`;
}

/** Parse a JSONL stream for the thread.started event's thread_id */
function _parseThreadId(jsonl: string): string | null {
  for (const line of jsonl.split("\n")) {
    try {
      const evt = JSON.parse(line);
      if (evt.type === "thread.started" && typeof evt.thread_id === "string") {
        return evt.thread_id;
      }
    } catch { /* skip non-JSON lines */ }
  }
  return null;
}

/** Parse a JSONL stream for all agent_message texts (concatenated) */
function _parseAgentMessage(jsonl: string): string | null {
  const parts: string[] = [];
  for (const line of jsonl.split("\n")) {
    try {
      const evt = JSON.parse(line);
      if (
        evt.type === "item.completed" &&
        evt.item?.type === "agent_message" &&
        typeof evt.item?.text === "string"
      ) {
        parts.push(evt.item.text);
      }
    } catch { /* skip non-JSON lines */ }
  }
  return parts.length > 0 ? parts.join("\n") : null;
}

/** Check if a directory is a valid git repo root */
function isGitRepo(dir: string): boolean {
  try {
    execSync("git rev-parse --git-dir", {
      cwd: dir,
      timeout: 5000,
      stdio: ["pipe", "pipe", "pipe"],
    });
    return true;
  } catch {
    return false;
  }
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
          text: `STILL RUNNING: Codex is still processing (${Math.round(job.elapsed_ms / 1000)}s elapsed, pid ${job.pid}).\n\nCall get_response("${job.id}") again to check.`,
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
        text: `Failed to get response from Codex.\n\n${job.result ? formatError(job.result) : "No result available."}\n\n[Job: ${job.id}]`,
      },
    ],
    isError: true,
  };
}

export function registerCodexTools(server: McpServer): void {
  // ─── send_message ──────────────────────────────────────────────
  server.tool(
    "send_message",
    "Send a structured inter-agent request to Codex. Returns IMMEDIATELY with a job_id confirming Codex CLI was spawned (SYN/ACK). Use get_response(job_id) to retrieve the actual response. Uses `codex exec -` (stdin) with --skip-git-repo-check. IMPORTANT: After receiving the ACK, you MUST call get_response with the returned job_id to get Codex's answer.",
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
          "The specific question or request for Codex (1-3 sentences recommended)"
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
          "Working directory for Codex to review files in. Defaults to MCP server's cwd."
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
      const progress = makeProgressLogger(server, "Codex");
      const projectDir = params.cwd || process.cwd();

      // Verify cwd exists
      if (!existsSync(projectDir)) {
        return {
          content: [
            {
              type: "text" as const,
              text: `Working directory does not exist: ${projectDir}`,
            },
          ],
          isError: true,
        };
      }

      // Detect context
      const ctx = detectContext("codex", projectDir);

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

      // Spawn Codex CLI asynchronously — returns immediately
      const { process: proc, result: resultPromise } = spawnCliAsync({
        command: CODEX_CLI,
        args: ["exec", "--dangerously-bypass-approvals-and-sandbox", "--cd", projectDir, "--skip-git-repo-check", "-"],
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
              text: `SPAWN FAILED: Could not start Codex CLI at ${CODEX_CLI}. Check that the binary exists and is executable.`,
            },
          ],
          isError: true,
        };
      }

      // Register the job
      let jobId: string;
      try {
        jobId = createJob("codex", proc);
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
          target: "codex",
          request_type: params.request_type,
          question: params.question,
          message,
          result: result.success ? "sent" : result.timed_out ? "timeout" : "failed",
          response: result.success ? result.stdout : undefined,
          error: !result.success ? formatError(result) : undefined,
          duration_ms: result.duration_ms,
        });
      });

      // Return ACK immediately — Codex is running
      return {
        content: [
          {
            type: "text" as const,
            text: `ACK: Codex CLI spawned (pid ${proc.pid}), processing your ${params.request_type} request.\n\nJob ID: ${jobId}\n\nCall get_response("${jobId}") to retrieve Codex's answer.`,
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
    "Spawn a persistent Codex session for multi-turn interaction. Runs `codex exec --json` to create a thread, then subsequent ask_daemon() calls use `codex exec resume <thread_id> --json` to continue the conversation. Codex persists the thread on disk — the daemon_id is just a handle to that thread. Each ask is a fresh process (~7s) with full conversation continuity.",
    {
      cwd: z
        .string()
        .optional()
        .describe("Working directory for the session. Defaults to MCP server cwd."),
      timeout_ms: z
        .number()
        .min(5000)
        .max(120_000)
        .optional()
        .describe("Timeout for the initial session creation (default: 60s)."),
    },
    async (params) => {
      const projectDir = params.cwd || process.cwd();
      const swept = sweepScratchpad(projectDir);
      ensureScratchpad(projectDir);

      const result = await executeCli({
        command: CODEX_CLI,
        args: ["exec", "--dangerously-bypass-approvals-and-sandbox", "--skip-git-repo-check", "--json",
          "You are a research and coding assistant. I will send follow-up questions. Acknowledge with: Ready."],
        cwd: projectDir,
        timeout_ms: params.timeout_ms || 60_000,
        onProgress: makeProgressLogger(server, "Codex"),
      });

      if (!result.success) {
        return {
          content: [{ type: "text" as const, text: `Failed to start Codex session:\n${formatError(result)}` }],
          isError: true,
        };
      }

      const threadId = _parseThreadId(result.stdout);
      if (!threadId) {
        return {
          content: [{ type: "text" as const, text: `Could not extract thread ID from Codex output:\n${result.stdout.slice(0, 500)}` }],
          isError: true,
        };
      }

      const sessionId = _genSessionId();
      _sessions.set(sessionId, {
        id: sessionId,
        thread_id: threadId,
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
              `Codex daemon ready.\n\n` +
              `Daemon ID: ${sessionId}\n` +
              `Thread: ${threadId}\n` +
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
    "Send a question to a running Codex daemon and wait for its response. Runs `codex exec resume <thread_id> --json` — full conversation history is automatically included by Codex. Each call takes ~7s (fresh process per ask). The daemon must be idle.",
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
        const result = await executeCli({
          command: CODEX_CLI,
          args: ["exec", "resume", "--dangerously-bypass-approvals-and-sandbox", "--skip-git-repo-check", "--json", session.thread_id, params.question],
          cwd: session.cwd,
          timeout_ms: params.timeout_ms || 120_000,
          onProgress: makeProgressLogger(server, "Codex"),
        });

        session.status = "idle";

        if (!result.success) {
          session.status = "dead";
          return { content: [{ type: "text" as const, text: `Codex failed:\n${formatError(result)}` }], isError: true };
        }

        const response = _parseAgentMessage(result.stdout);
        if (!response) {
          return {
            content: [{ type: "text" as const, text: `No agent_message in Codex output.\n\nRaw:\n${result.stdout.slice(0, 1000)}` }],
            isError: true,
          };
        }

        return {
          content: [{ type: "text" as const, text: `Codex daemon response:\n\n${response}` }],
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
    "Dismiss a Codex daemon — removes it from the active session registry. The underlying Codex thread remains in Codex's session history and can be resumed manually if needed.",
    {
      daemon_id: z.string().min(1).describe("The daemon ID to dismiss"),
    },
    async (params) => {
      const session = _sessions.get(params.daemon_id);
      if (!session) {
        return {
          content: [{ type: "text" as const, text: `Daemon not found: ${params.daemon_id}. It may have already been dismissed.` }],
        };
      }
      const reaped = reapDaemonFiles(session.cwd, params.daemon_id);
      _sessions.delete(params.daemon_id);
      const reapedNote = reaped.length > 0
        ? `\nReaper removed ${reaped.length} scratchpad file(s): ${reaped.join(", ")}`
        : "";
      return {
        content: [
          {
            type: "text" as const,
            text: `Codex daemon ${params.daemon_id} dismissed. Thread ${session.thread_id} remains in Codex's history.${reapedNote}`,
          },
        ],
      };
    }
  );

  // ─── list_daemons ───────────────────────────────────────────────
  server.tool(
    "list_daemons",
    "List all active Codex daemon sessions with their status, age, and thread ID. Non-blocking.",
    {},
    async () => {
      if (_sessions.size === 0) {
        return {
          content: [{ type: "text" as const, text: "No active Codex daemons." }],
        };
      }
      const summary = Array.from(_sessions.values()).map((s) => ({
        id: s.id,
        status: s.status,
        thread_id: s.thread_id,
        age: `${Math.round((Date.now() - s.created_at) / 1000)}s`,
        cwd: s.cwd,
      }));
      return {
        content: [
          {
            type: "text" as const,
            text: `Active Codex daemons (${_sessions.size}):\n\n${JSON.stringify(summary, null, 2)}`,
          },
        ],
      };
    }
  );

  // ─── list_scratchpad ────────────────────────────────────────────
  server.tool(
    "list_scratchpad",
    "List all files currently in the inter-agent scratchpad for a project. The scratchpad is at <project-root>/.claude/scratchpad/ and is shared by all three agents.",
    {
      cwd: z
        .string()
        .optional()
        .describe("Project root to find the scratchpad. Defaults to MCP server cwd."),
    },
    async (params) => {
      const projectDir = params.cwd || process.cwd();
      sweepScratchpad(projectDir);
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
    "Write a progress note or artifact to the inter-agent scratchpad. All agents can read files written here.",
    {
      topic: z
        .string()
        .min(1)
        .describe("Short topic label for the filename (e.g., 'progress-update', 'code-review-findings')"),
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
        const filepath = writeScratchpad(projectDir, "codex", params.topic, params.content);
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
    "List all active Codex inter-agent jobs. Returns job ID, status (running/completed/failed/expired), elapsed time, and PID. Non-blocking — call anytime to check on outstanding requests.",
    {},
    async () => {
      const jobs = listJobs();
      if (jobs.length === 0) {
        return {
          content: [{ type: "text" as const, text: "No active Codex jobs." }],
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
            text: `Active Codex jobs (${jobs.length}):\n\n${JSON.stringify(summary, null, 2)}`,
          },
        ],
      };
    }
  );

  // ─── code_review ───────────────────────────────────────────────
  server.tool(
    "code_review",
    "Review code changes via `codex review` with scope flags. This is a git-aware review — Codex examines diffs, not a free-form prompt. You must provide exactly one scope: uncommitted changes, a base branch comparison, or a specific commit SHA. The cwd must be a valid git repository root. This tool blocks until complete (code reviews are typically fast).",
    {
      cwd: z
        .string()
        .optional()
        .describe(
          "Git repository root directory for the review. Must be a valid git repo."
        ),
      uncommitted: z
        .boolean()
        .optional()
        .describe("Review uncommitted changes (staged + unstaged)"),
      base_branch: z
        .string()
        .optional()
        .describe(
          "Review changes compared to this base branch (e.g., 'main', 'develop')"
        ),
      commit_sha: z
        .string()
        .optional()
        .describe("Review a specific commit by SHA"),
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
      const progress = makeProgressLogger(server, "Codex");
      const projectDir = params.cwd || process.cwd();

      // Validate git repo
      if (!isGitRepo(projectDir)) {
        return {
          content: [
            {
              type: "text" as const,
              text: `Not a valid git repository: ${projectDir}\n\nThe code_review tool requires a git repo root. Use send_message for general questions.`,
            },
          ],
          isError: true,
        };
      }

      // Build scope flags (mutually exclusive)
      const args: string[] = ["review"];
      let scopeDescription = "";

      const scopeCount =
        (params.uncommitted ? 1 : 0) +
        (params.base_branch ? 1 : 0) +
        (params.commit_sha ? 1 : 0);

      if (scopeCount > 1) {
        return {
          content: [
            {
              type: "text" as const,
              text: `Scope flags are mutually exclusive. Provide only ONE of: uncommitted, base_branch, or commit_sha.`,
            },
          ],
          isError: true,
        };
      }

      if (params.uncommitted) {
        args.push("--uncommitted");
        scopeDescription = "uncommitted changes";
      } else if (params.base_branch) {
        args.push("--base", params.base_branch);
        scopeDescription = `changes vs ${params.base_branch}`;
      } else if (params.commit_sha) {
        args.push("--commit", params.commit_sha);
        scopeDescription = `commit ${params.commit_sha.slice(0, 8)}`;
      } else {
        // Default to uncommitted
        args.push("--uncommitted");
        scopeDescription = "uncommitted changes (default)";
      }

      const result = await executeCli({
        command: CODEX_CLI,
        args,
        cwd: projectDir,
        timeout_ms: params.timeout_ms || DEFAULT_TIMEOUT_MS,
        onProgress: progress,
      });

      if (result.success) {
        return {
          content: [
            {
              type: "text" as const,
              text: `Codex Review (${scopeDescription} in ${projectDir}):\n\n${result.stdout}`,
            },
          ],
        };
      }

      return {
        content: [
          {
            type: "text" as const,
            text: `Codex review failed (${scopeDescription}).\n\n${formatError(result)}`,
          },
        ],
        isError: true,
      };
    }
  );
}
