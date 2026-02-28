/**
 * Build ===INTER_AGENT_REQUEST=== formatted messages from structured data.
 *
 * This is the core of the reliability fix: Claude passes structured JSON
 * parameters via MCP, and this module constructs the protocol message
 * as a plain TypeScript string. No heredoc, no bash, no escaping issues.
 *
 * The receiving agents (Gemini/Codex) continue to see the exact same
 * protocol format they're already configured to parse.
 */

import type { ProtocolContext, RequestType, REQUEST_TYPES } from "./types.js";

export interface MessageParams {
  context: ProtocolContext;
  request_type: RequestType;
  question: string;
  additional_context?: string;
}

/**
 * Validate that all required protocol fields are present and non-empty.
 * Returns an array of error messages (empty = valid).
 */
export function validateContext(context: ProtocolContext): string[] {
  const errors: string[] = [];
  const required: (keyof ProtocolContext)[] = [
    "from",
    "to",
    "timestamp",
    "repo",
    "branch",
    "taxonomy",
    "cwd",
  ];

  for (const field of required) {
    if (!context[field]) {
      errors.push(`Missing or empty field: ${field}`);
    }
  }

  return errors;
}

/**
 * Format a complete inter-agent protocol message.
 * This produces the exact same text format that the bash scripts produce,
 * but without any shell escaping concerns.
 */
export function formatMessage(params: MessageParams): string {
  const { context, request_type, question, additional_context } = params;

  const contextText = additional_context || "Full context in session log.";

  return `===INTER_AGENT_REQUEST===
FROM: ${context.from}
TO: ${context.to}
TIMESTAMP: ${context.timestamp}
REPO: ${context.repo}
BRANCH: ${context.branch}
SESSION_LOG: ${context.session_log}
TAXONOMY: ${context.taxonomy}
CWD: ${context.cwd}
REQUEST_TYPE: ${request_type}
===END_HEADERS===

QUESTION:
${question}

CONTEXT:
${contextText}

===END_REQUEST===`;
}
