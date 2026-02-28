/**
 * Auto-detect inter-agent protocol context fields.
 *
 * Ported from detect-context.sh — same logic, no shell escaping.
 * Detects: repo, branch, cwd, taxonomy, session_log, timestamp.
 * All git operations use execSync with short timeouts since they're local.
 */

import { execSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import type { AgentTarget, ProtocolContext } from "./types.js";

const GIT_TIMEOUT = 5_000; // 5s for local git ops

function execGit(command: string, cwd?: string): string {
  try {
    return execSync(command, {
      cwd,
      timeout: GIT_TIMEOUT,
      encoding: "utf-8",
      stdio: ["pipe", "pipe", "pipe"],
    }).trim();
  } catch {
    return "";
  }
}

/** Get git remote URL or fall back to cwd */
export function detectRepo(cwd: string): string {
  return execGit("git remote get-url origin", cwd) || cwd;
}

/** Get current git branch or fall back to "main" */
export function detectBranch(cwd: string): string {
  return execGit("git branch --show-current", cwd) || "main";
}

/** Parse taxonomy from .claude/taxonomy.json, git remote, or directory name */
export function detectTaxonomy(cwd: string): string {
  // Try .claude/taxonomy.json first
  const taxonomyPath = join(cwd, ".claude", "taxonomy.json");
  if (existsSync(taxonomyPath)) {
    try {
      const data = JSON.parse(readFileSync(taxonomyPath, "utf-8"));
      if (data.owner && data.client && data.domain && data.repo) {
        return `${data.owner}/${data.client}/${data.domain}/${data.repo}`;
      }
    } catch {
      // Fall through to git remote parsing
    }
  }

  // Try git remote parsing (github.com/owner/repo format)
  const remote = execGit("git remote get-url origin", cwd);
  if (remote) {
    const match = remote.match(/github\.com[:/]([^/]+)\/([^/.]+)/);
    if (match) {
      return `${match[1]}/project/general/${match[2]}`;
    }
  }

  // Fallback: use directory name
  return `unknown/project/general/${basename(cwd)}`;
}

/** Generate ISO-8601 timestamp with America/New_York timezone */
export function generateTimestamp(): string {
  const now = new Date();
  // Format in America/New_York timezone
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

  const date = `${get("year")}-${get("month")}-${get("day")}`;
  const time = `${get("hour")}:${get("minute")}:${get("second")}`;

  // Calculate EST/EDT offset
  const jan = new Date(now.getFullYear(), 0, 1);
  const jul = new Date(now.getFullYear(), 6, 1);
  const stdOffset = Math.max(
    jan.getTimezoneOffset(),
    jul.getTimezoneOffset()
  );
  // Check if currently in DST by comparing NY time offset
  const nyOffset = new Date(
    now.toLocaleString("en-US", { timeZone: "America/New_York" })
  );
  const utc = new Date(now.toLocaleString("en-US", { timeZone: "UTC" }));
  const diffHours = Math.round(
    (utc.getTime() - nyOffset.getTime()) / (1000 * 60 * 60)
  );
  const offsetStr =
    diffHours >= 0
      ? `-${String(diffHours).padStart(2, "0")}:00`
      : `+${String(Math.abs(diffHours)).padStart(2, "0")}:00`;

  return `${date}T${time}${offsetStr}`;
}

/**
 * Detect all protocol context fields for a given target agent.
 * This is the main entry point — equivalent to detect-context.sh main().
 */
export function detectContext(
  target: AgentTarget,
  cwdOverride?: string
): ProtocolContext {
  const cwd = cwdOverride || process.cwd();

  return {
    from: "claude",
    to: target,
    timestamp: generateTimestamp(),
    repo: detectRepo(cwd),
    branch: detectBranch(cwd),
    session_log: "", // Populated by session-log-finder if needed
    taxonomy: detectTaxonomy(cwd),
    cwd,
  };
}
