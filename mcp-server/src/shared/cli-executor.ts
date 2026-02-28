/**
 * CLI execution with timeout, SIGTERM/SIGKILL chain, and automatic retry.
 *
 * This is the core reliability fix. Instead of bash `gemini -p "$message"`
 * (which breaks on special characters), we use Node.js child_process.spawn()
 * with proper signal handling and structured error responses.
 *
 * Flow:
 *   First attempt ──timeout──> SIGTERM ──5s grace──> SIGKILL ──> Retry once ──> Return error
 */

import { spawn, type ChildProcess } from "node:child_process";
import {
  ANSI_REGEX,
  DEFAULT_TIMEOUT_MS,
  MAX_TIMEOUT_MS,
  SIGTERM_GRACE_MS,
  type ExecutionResult,
} from "./types.js";

/** Strip ANSI escape codes from CLI output */
export function stripAnsi(text: string): string {
  return text.replace(ANSI_REGEX, "");
}

/** Patterns that indicate a retryable Codex failure */
const RETRYABLE_PATTERNS = [
  /stream disconnected/i,
  /interrupted.*please rerun/i,
  /ECONNRESET/i,
  /socket hang up/i,
];

function isRetryable(stderr: string, stdout: string): boolean {
  const combined = stderr + stdout;
  return RETRYABLE_PATTERNS.some((p) => p.test(combined));
}

/** Progress event types for real-time feedback */
export type ProgressEvent =
  | { type: "spawned"; command: string; pid: number }
  | { type: "heartbeat"; elapsed_ms: number }
  | { type: "stdout_data"; bytes: number }
  | { type: "timeout"; elapsed_ms: number; action: "SIGTERM" | "SIGKILL" }
  | { type: "retry"; attempt: number }
  | { type: "done"; elapsed_ms: number; success: boolean };

/** Callback for progress events — tools wire this to MCP logging */
export type OnProgress = (event: ProgressEvent) => void;

interface ExecOptions {
  /** CLI binary path */
  command: string;
  /** CLI arguments */
  args: string[];
  /** Data to write to stdin (for Codex) */
  stdin?: string;
  /** Working directory */
  cwd?: string;
  /** Timeout in milliseconds */
  timeout_ms?: number;
  /** Whether to retry on failure/timeout */
  retry?: boolean;
  /** Environment variable overrides */
  env?: Record<string, string>;
  /** Progress callback for real-time feedback (ACK, WORKING, etc.) */
  onProgress?: OnProgress;
}

/**
 * Execute a CLI command with timeout and signal handling.
 * Returns a structured result regardless of success or failure.
 */
/**
 * Heartbeat schedule — progressive backoff to avoid log spam.
 * First heartbeat at 10s, then 30s apart, then 60s apart.
 */
const HEARTBEAT_SCHEDULE_MS = [10_000, 30_000, 60_000];

async function execOnce(options: ExecOptions): Promise<ExecutionResult> {
  const {
    command,
    args,
    stdin,
    cwd,
    timeout_ms = DEFAULT_TIMEOUT_MS,
    env,
    onProgress,
  } = options;

  const effectiveTimeout = Math.min(timeout_ms, MAX_TIMEOUT_MS);
  const startTime = Date.now();

  return new Promise<ExecutionResult>((resolve) => {
    let stdoutChunks: Buffer[] = [];
    let stderrChunks: Buffer[] = [];
    let totalStdoutBytes = 0;
    let timedOut = false;
    let killed = false;
    let timeoutHandle: NodeJS.Timeout | undefined;
    let graceHandle: NodeJS.Timeout | undefined;
    let heartbeatHandle: NodeJS.Timeout | undefined;

    const proc: ChildProcess = spawn(command, args, {
      cwd,
      env: { ...process.env, ...env },
      stdio: ["pipe", "pipe", "pipe"],
    });

    // ACK: Process spawned successfully — immediate feedback
    if (proc.pid && onProgress) {
      onProgress({ type: "spawned", command, pid: proc.pid });
    }

    // HEARTBEAT: Progressive backoff — 10s, then 30s, then 60s repeating
    let heartbeatIndex = 0;
    function scheduleHeartbeat() {
      const delay = HEARTBEAT_SCHEDULE_MS[Math.min(heartbeatIndex, HEARTBEAT_SCHEDULE_MS.length - 1)];
      heartbeatHandle = setTimeout(() => {
        onProgress?.({
          type: "heartbeat",
          elapsed_ms: Date.now() - startTime,
        });
        heartbeatIndex++;
        scheduleHeartbeat();
      }, delay);
    }
    if (onProgress) {
      scheduleHeartbeat();
    }

    // Collect stdout — also fire progress on first data (proves agent is responding)
    proc.stdout?.on("data", (chunk: Buffer) => {
      stdoutChunks.push(chunk);
      totalStdoutBytes += chunk.length;
      // Notify on first stdout data — the sibling agent is actively responding
      if (totalStdoutBytes === chunk.length && onProgress) {
        onProgress({ type: "stdout_data", bytes: chunk.length });
      }
    });

    // Collect stderr
    proc.stderr?.on("data", (chunk: Buffer) => {
      stderrChunks.push(chunk);
    });

    // Handle stdin errors (EPIPE when process exits early)
    proc.stdin?.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code !== "EPIPE") {
        stderrChunks.push(Buffer.from(`stdin error: ${err.message}\n`));
      }
      // EPIPE is expected when process closes stdin early — tolerate it
    });

    // Write stdin data if provided (Codex send_message, Gemini send_message)
    // Use end(data) instead of write()+end() to avoid backpressure issues on large payloads
    if (stdin && proc.stdin) {
      proc.stdin.end(stdin);
    }

    // Set timeout
    timeoutHandle = setTimeout(() => {
      timedOut = true;
      onProgress?.({ type: "timeout", elapsed_ms: Date.now() - startTime, action: "SIGTERM" });
      // Send SIGTERM first (graceful shutdown)
      proc.kill("SIGTERM");

      // If still alive after grace period, SIGKILL
      graceHandle = setTimeout(() => {
        if (!killed) {
          killed = true;
          onProgress?.({ type: "timeout", elapsed_ms: Date.now() - startTime, action: "SIGKILL" });
          proc.kill("SIGKILL");
        }
      }, SIGTERM_GRACE_MS);
    }, effectiveTimeout);

    // Handle process exit
    proc.on("close", (code, signal) => {
      clearTimeout(timeoutHandle);
      clearTimeout(graceHandle);
      clearTimeout(heartbeatHandle);

      const duration_ms = Date.now() - startTime;
      const stdout = stripAnsi(Buffer.concat(stdoutChunks).toString("utf-8"));
      const stderr = stripAnsi(Buffer.concat(stderrChunks).toString("utf-8"));
      const success = code === 0 && !timedOut;

      onProgress?.({ type: "done", elapsed_ms: duration_ms, success });

      resolve({
        success,
        stdout,
        stderr,
        exit_code: code,
        duration_ms,
        timed_out: timedOut,
        retried: false,
        command: `${command} ${args.join(" ")}`,
      });
    });

    // Handle spawn errors (binary not found, permission denied, etc.)
    proc.on("error", (err) => {
      clearTimeout(timeoutHandle);
      clearTimeout(graceHandle);
      clearTimeout(heartbeatHandle);

      const duration_ms = Date.now() - startTime;

      onProgress?.({ type: "done", elapsed_ms: duration_ms, success: false });

      resolve({
        success: false,
        stdout: "",
        stderr: `spawn error: ${err.message}`,
        exit_code: null,
        duration_ms,
        timed_out: false,
        retried: false,
        command: `${command} ${args.join(" ")}`,
      });
    });
  });
}

/**
 * Execute a CLI command with automatic retry on timeout or retryable errors.
 * This is the main entry point for all CLI invocations.
 */
export async function executeCli(options: ExecOptions): Promise<ExecutionResult> {
  const shouldRetry = options.retry !== false; // retry by default

  const firstResult = await execOnce(options);

  // If successful, return immediately
  if (firstResult.success) {
    return firstResult;
  }

  // If retryable failure, try once more
  if (
    shouldRetry &&
    (firstResult.timed_out ||
      isRetryable(firstResult.stderr, firstResult.stdout))
  ) {
    options.onProgress?.({ type: "retry", attempt: 2 });
    const retryResult = await execOnce(options);
    retryResult.retried = true;
    return retryResult;
  }

  return firstResult;
}

/**
 * Spawn a CLI process WITHOUT waiting for it to finish.
 * Returns the ChildProcess immediately (for job store registration)
 * and a Promise that resolves when the process completes.
 *
 * This is the async counterpart to executeCli — used by the SYN/ACK pattern
 * where send_message returns immediately and get_response waits.
 */
export function spawnCliAsync(options: ExecOptions): {
  process: ChildProcess;
  result: Promise<ExecutionResult>;
} {
  const {
    command,
    args,
    stdin,
    cwd,
    timeout_ms = DEFAULT_TIMEOUT_MS,
    env,
    onProgress,
  } = options;

  const effectiveTimeout = Math.min(timeout_ms, MAX_TIMEOUT_MS);
  const startTime = Date.now();

  const proc: ChildProcess = spawn(command, args, {
    cwd,
    env: { ...process.env, ...env },
    stdio: ["pipe", "pipe", "pipe"],
  });

  // ACK: Process spawned — immediate feedback
  if (proc.pid && onProgress) {
    onProgress({ type: "spawned", command, pid: proc.pid });
  }

  // Write stdin if provided
  if (stdin && proc.stdin) {
    proc.stdin.on("error", (err: NodeJS.ErrnoException) => {
      if (err.code !== "EPIPE") {
        // Non-EPIPE errors will be captured in stderr
      }
    });
    proc.stdin.end(stdin);
  }

  const result = new Promise<ExecutionResult>((resolve) => {
    let stdoutChunks: Buffer[] = [];
    let stderrChunks: Buffer[] = [];
    let timedOut = false;
    let killed = false;
    let timeoutHandle: NodeJS.Timeout | undefined;
    let graceHandle: NodeJS.Timeout | undefined;
    let heartbeatHandle: NodeJS.Timeout | undefined;

    // Heartbeat
    let heartbeatIndex = 0;
    function scheduleHeartbeat() {
      const delay =
        HEARTBEAT_SCHEDULE_MS[
          Math.min(heartbeatIndex, HEARTBEAT_SCHEDULE_MS.length - 1)
        ];
      heartbeatHandle = setTimeout(() => {
        onProgress?.({
          type: "heartbeat",
          elapsed_ms: Date.now() - startTime,
        });
        heartbeatIndex++;
        scheduleHeartbeat();
      }, delay);
    }
    if (onProgress) {
      scheduleHeartbeat();
    }

    proc.stdout?.on("data", (chunk: Buffer) => {
      stdoutChunks.push(chunk);
      if (stdoutChunks.length === 1 && onProgress) {
        onProgress({ type: "stdout_data", bytes: chunk.length });
      }
    });

    proc.stderr?.on("data", (chunk: Buffer) => {
      stderrChunks.push(chunk);
    });

    timeoutHandle = setTimeout(() => {
      timedOut = true;
      onProgress?.({
        type: "timeout",
        elapsed_ms: Date.now() - startTime,
        action: "SIGTERM",
      });
      proc.kill("SIGTERM");
      graceHandle = setTimeout(() => {
        if (!killed) {
          killed = true;
          onProgress?.({
            type: "timeout",
            elapsed_ms: Date.now() - startTime,
            action: "SIGKILL",
          });
          proc.kill("SIGKILL");
        }
      }, SIGTERM_GRACE_MS);
    }, effectiveTimeout);

    proc.on("close", (code) => {
      clearTimeout(timeoutHandle);
      clearTimeout(graceHandle);
      clearTimeout(heartbeatHandle);

      const duration_ms = Date.now() - startTime;
      const stdout = stripAnsi(
        Buffer.concat(stdoutChunks).toString("utf-8")
      );
      const stderr = stripAnsi(
        Buffer.concat(stderrChunks).toString("utf-8")
      );
      const success = code === 0 && !timedOut;

      onProgress?.({ type: "done", elapsed_ms: duration_ms, success });

      resolve({
        success,
        stdout,
        stderr,
        exit_code: code,
        duration_ms,
        timed_out: timedOut,
        retried: false,
        command: `${command} ${args.join(" ")}`,
      });
    });

    proc.on("error", (err) => {
      clearTimeout(timeoutHandle);
      clearTimeout(graceHandle);
      clearTimeout(heartbeatHandle);

      const duration_ms = Date.now() - startTime;
      onProgress?.({ type: "done", elapsed_ms: duration_ms, success: false });

      resolve({
        success: false,
        stdout: "",
        stderr: `spawn error: ${err.message}`,
        exit_code: null,
        duration_ms,
        timed_out: false,
        retried: false,
        command: `${command} ${args.join(" ")}`,
      });
    });
  });

  return { process: proc, result };
}

/**
 * Format an ExecutionResult into a human-readable error message
 * that Claude can reason about and present to the user.
 */
export function formatError(result: ExecutionResult): string {
  const lines: string[] = [];

  if (result.timed_out) {
    lines.push(
      `TIMEOUT: Command timed out after ${Math.round(result.duration_ms / 1000)}s`
    );
    lines.push(
      `The target agent may be processing a complex request. Try increasing timeout_ms.`
    );
  } else if (result.exit_code !== null && result.exit_code !== 0) {
    lines.push(`FAILED: Command exited with code ${result.exit_code}`);
  } else if (result.exit_code === null) {
    lines.push(`SPAWN ERROR: Could not start the CLI process`);
    lines.push(
      `Check that the CLI binary exists and is executable.`
    );
  }

  if (result.stderr) {
    lines.push(`\nStderr:\n${result.stderr.slice(0, 2000)}`);
  }

  if (result.retried) {
    lines.push(`\nNote: This was the RETRY attempt (first attempt also failed).`);
  }

  lines.push(`\nCommand: ${result.command}`);
  lines.push(`Duration: ${Math.round(result.duration_ms / 1000)}s`);

  // Troubleshooting hints
  if (result.stderr.includes("not found") || result.exit_code === null) {
    lines.push(`\nTroubleshooting: Verify the CLI binary path in mcp_config.json env vars.`);
  }
  if (
    result.stderr.includes("auth") ||
    result.stderr.includes("login") ||
    result.stderr.includes("token")
  ) {
    lines.push(
      `\nTroubleshooting: The target agent may need authentication. Run the CLI manually to check.`
    );
  }

  return lines.join("\n");
}
