/**
 * Find the latest session log in a project directory.
 *
 * Session logs follow the naming convention:
 *   session-logs/{owner}--{client}_{domain}_{repo}_{feature}_{YYYYMMDD}_v{N}_{agent}.md
 *
 * We look for *_claude.md files (since Claude is always the sender in Phase 1).
 */

import { existsSync, readdirSync, mkdirSync } from "node:fs";
import { join } from "node:path";

/**
 * Find the most recent Claude session log in a project directory.
 * Returns a relative path from the project root, or generates a new filename.
 */
export function findLatestSessionLog(
  cwd: string,
  taxonomy: string
): string {
  const sessionLogDir = join(cwd, "session-logs");

  // Look for existing Claude session logs
  if (existsSync(sessionLogDir)) {
    try {
      const files = readdirSync(sessionLogDir)
        .filter((f) => f.endsWith("_claude.md"))
        .sort()
        .reverse();

      if (files.length > 0) {
        return `session-logs/${files[0]}`;
      }
    } catch {
      // Fall through to generate new filename
    }
  }

  // No existing log — generate new filename
  const parts = taxonomy.split("/");
  const owner = parts[0] || "unknown";
  const client = parts[1] || "project";
  const domain = parts[2] || "general";
  const repo = parts[3] || "repo";

  const now = new Date();
  const formatter = new Intl.DateTimeFormat("en-CA", {
    timeZone: "America/New_York",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  });
  const dateStr = formatter.format(now).replace(/-/g, "");

  return `session-logs/${owner}--${client}_${domain}_${repo}_inter-agent-request_${dateStr}_v1_claude.md`;
}
