/**
 * Shared types for inter-agent MCP servers.
 *
 * These types define the inter-agent protocol structure that all three
 * agents (Claude, Gemini, Codex) understand. The protocol format
 * (===INTER_AGENT_REQUEST===) is preserved — only the transport changes.
 */

// Valid request types for inter-agent protocol
export const REQUEST_TYPES = [
  "question",
  "review",
  "debug",
  "architecture",
  "other",
] as const;
export type RequestType = (typeof REQUEST_TYPES)[number];

// Agent identifiers
export const AGENT_TARGETS = ["claude", "gemini", "codex"] as const;
export type AgentTarget = (typeof AGENT_TARGETS)[number];

// Auto-detected context fields (populated by context-detector)
export interface ProtocolContext {
  from: AgentTarget;
  to: AgentTarget;
  timestamp: string; // ISO-8601 with EST/EDT timezone
  repo: string; // git remote URL or local path
  branch: string; // current git branch
  session_log: string; // relative path from repo root
  taxonomy: string; // owner/client/domain/repo
  cwd: string; // absolute working directory
}

// Result from CLI execution
export interface ExecutionResult {
  success: boolean;
  stdout: string;
  stderr: string;
  exit_code: number | null;
  duration_ms: number;
  timed_out: boolean;
  retried: boolean;
  command: string;
}

// Outbox log entry
export interface OutboxEntry {
  timestamp: string;
  target: AgentTarget;
  request_type: RequestType;
  question: string;
  message: string; // full formatted protocol message
  result: "sent" | "failed" | "timeout";
  response?: string;
  error?: string;
  duration_ms: number;
}

// Timeout defaults (milliseconds)
export const DEFAULT_TIMEOUT_MS = 300_000; // 5 minutes
export const MAX_TIMEOUT_MS = 600_000; // 10 minutes
export const SIGTERM_GRACE_MS = 5_000; // 5 seconds between SIGTERM and SIGKILL

// ANSI escape code pattern for stripping CLI output coloring
export const ANSI_REGEX = /\x1B\[[0-9;]*[a-zA-Z]/g;
