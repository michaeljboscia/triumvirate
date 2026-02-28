/**
 * Scratchpad Reaper — manages the inter-agent collaboration scratchpad.
 *
 * The scratchpad is a project-local ephemeral directory at:
 *   <project-root>/.claude/scratchpad/
 *
 * All three agents (Claude, Gemini, Codex) know to write artifacts here
 * during collaborative sessions. Claude reads them asynchronously.
 *
 * File naming convention:
 *   YYYYMMDD_HHMMSS_<agentId>_<topic>.md
 *   Example: 20260217_091500_gemini_pain-signal-analysis.md
 *
 * The Reaper runs in three situations:
 *   1. On spawn_daemon() — sweeps stale files before a new session starts
 *   2. On dismiss_daemon() — removes that daemon's files
 *   3. On any MCP tool call (via sweepScratchpad) — opportunistic TTL cleanup
 *
 * Files older than SCRATCHPAD_TTL_MS are deleted regardless of daemon status.
 * This prevents disk pollution from crashed sessions.
 */

import {
  existsSync,
  mkdirSync,
  readdirSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { join, basename } from "node:path";

/** How long scratchpad files live before the Reaper claims them */
const SCRATCHPAD_TTL_MS = 2 * 60 * 60_000; // 2 hours

/** Subdirectory under the project's .claude/ folder */
const SCRATCHPAD_DIR_NAME = "scratchpad";

/**
 * Resolve the scratchpad directory for a given project root.
 * Returns the absolute path to <project-root>/.claude/scratchpad/
 */
export function getScratchpadDir(projectRoot: string): string {
  return join(projectRoot, ".claude", SCRATCHPAD_DIR_NAME);
}

/**
 * Ensure the scratchpad directory exists. Creates it if absent.
 * Also writes a .gitignore so scratchpad files don't get committed.
 */
export function ensureScratchpad(projectRoot: string): string {
  const dir = getScratchpadDir(projectRoot);

  if (!existsSync(dir)) {
    mkdirSync(dir, { recursive: true });

    // Keep scratchpad files out of git
    const gitignorePath = join(dir, ".gitignore");
    writeFileSync(gitignorePath, "*\n!.gitignore\n", "utf-8");
  }

  return dir;
}

/**
 * Write a file to the scratchpad.
 * Returns the full path of the written file.
 */
export function writeScratchpad(
  projectRoot: string,
  agentId: string,
  topic: string,
  content: string
): string {
  const dir = ensureScratchpad(projectRoot);

  const now = new Date();
  const pad = (n: number, w = 2) => String(n).padStart(w, "0");
  const timestamp =
    `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}` +
    `_${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;

  // Sanitize topic for use in filename
  const safeTopic = topic.replace(/[^a-z0-9-]/gi, "-").toLowerCase().slice(0, 40);
  const filename = `${timestamp}_${agentId}_${safeTopic}.md`;
  const filepath = join(dir, filename);

  writeFileSync(filepath, content, "utf-8");
  return filepath;
}

/**
 * List all files currently in the scratchpad.
 */
export function listScratchpad(projectRoot: string): Array<{
  filename: string;
  path: string;
  age_ms: number;
  size_bytes: number;
}> {
  const dir = getScratchpadDir(projectRoot);
  if (!existsSync(dir)) return [];

  const now = Date.now();
  return readdirSync(dir)
    .filter((f) => f.endsWith(".md"))
    .map((f) => {
      const path = join(dir, f);
      const stat = statSync(path);
      return {
        filename: f,
        path,
        age_ms: now - stat.mtimeMs,
        size_bytes: stat.size,
      };
    })
    .sort((a, b) => a.age_ms - b.age_ms); // newest first
}

/**
 * Reaper: remove all scratchpad files older than SCRATCHPAD_TTL_MS.
 * Returns the list of files deleted.
 */
export function sweepScratchpad(projectRoot: string): string[] {
  const dir = getScratchpadDir(projectRoot);
  if (!existsSync(dir)) return [];

  const now = Date.now();
  const deleted: string[] = [];

  for (const entry of readdirSync(dir)) {
    if (!entry.endsWith(".md")) continue;
    const filepath = join(dir, entry);
    try {
      const stat = statSync(filepath);
      if (now - stat.mtimeMs > SCRATCHPAD_TTL_MS) {
        unlinkSync(filepath);
        deleted.push(entry);
      }
    } catch {
      // File already gone — fine
    }
  }

  return deleted;
}

/**
 * Reaper: remove all scratchpad files written by a specific agent.
 * Used by dismiss_daemon() to clean up that daemon's artifacts.
 * Returns the list of files deleted.
 */
export function reapDaemonFiles(projectRoot: string, agentId: string): string[] {
  const dir = getScratchpadDir(projectRoot);
  if (!existsSync(dir)) return [];

  const deleted: string[] = [];
  const prefix = `_${agentId}_`;

  for (const entry of readdirSync(dir)) {
    if (!entry.endsWith(".md")) continue;
    if (!entry.includes(prefix)) continue;
    const filepath = join(dir, entry);
    try {
      unlinkSync(filepath);
      deleted.push(entry);
    } catch {
      // Already gone — fine
    }
  }

  return deleted;
}
