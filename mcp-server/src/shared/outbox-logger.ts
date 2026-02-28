/**
 * Save sent messages to the outbox for audit trail.
 *
 * Every message (sent or failed) is saved to:
 *   ~/.claude/inter-agent-messages/outbox/YYYYMMDD_HHMMSS_{target}_{request_type}.txt
 *
 * This replaces the ad-hoc file saving in the current bash skills and provides
 * a reliable audit trail for debugging inter-agent communication.
 */

import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";
import type { OutboxEntry } from "./types.js";

const OUTBOX_DIR = join(homedir(), ".claude", "inter-agent-messages", "outbox");

function ensureOutboxDir(): void {
  if (!existsSync(OUTBOX_DIR)) {
    mkdirSync(OUTBOX_DIR, { recursive: true });
  }
}

function generateFilename(entry: OutboxEntry): string {
  // Format timestamp for filename: YYYYMMDD_HHMMSS
  const now = new Date();
  const formatter = new Intl.DateTimeFormat("en-CA", {
    timeZone: "America/New_York",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
  const parts = formatter.formatToParts(now);
  const get = (type: string) =>
    parts.find((p) => p.type === type)?.value ?? "00";
  const dateStr = `${get("year")}${get("month")}${get("day")}_${get("hour")}${get("minute")}${get("second")}`;

  return `${dateStr}_${entry.target}_${entry.request_type}.txt`;
}

/**
 * Log a sent or failed message to the outbox directory.
 */
export function logToOutbox(entry: OutboxEntry): string {
  ensureOutboxDir();

  const filename = generateFilename(entry);
  const filepath = join(OUTBOX_DIR, filename);

  const content = `# Inter-Agent Message Log
# Result: ${entry.result}
# Timestamp: ${entry.timestamp}
# Target: ${entry.target}
# Request Type: ${entry.request_type}
# Duration: ${Math.round(entry.duration_ms / 1000)}s
${entry.error ? `# Error: ${entry.error}\n` : ""}
## Question
${entry.question}

## Formatted Message
${entry.message}
${entry.response ? `\n## Response\n${entry.response}\n` : ""}`;

  writeFileSync(filepath, content, "utf-8");

  return filepath;
}
