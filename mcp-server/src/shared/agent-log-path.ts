/**
 * Compute the next session log path for a daemon agent (gemini or codex).
 *
 * Session log filename format (SESSION_LOG_SPEC v1.0):
 *   <owner>--<client>_<domain>_<repo>_<feature>_<YYYYMMDD>_v<N+1>_<agent>.md
 *
 * Version counter is per-agent (gemini and codex count independently).
 * Written to <cwd>/session-logs/, falling back to ~/.gemini/session-logs/
 * or ~/.codex/session-logs/ if the project path is unwritable.
 */

import { existsSync, readdirSync, readFileSync, mkdirSync } from "node:fs";
import { join, basename } from "node:path";
import { execSync } from "node:child_process";
import { homedir } from "node:os";

export interface TaxonomyFull {
  owner: string;
  client: string;
  domain: string;
  repo: string;
  feature: string;
}

/** Parse full taxonomy including feature field from taxonomy.json or git remote */
export function parseTaxonomyFull(cwd: string): TaxonomyFull {
  const taxonomyPath = join(cwd, ".claude", "taxonomy.json");
  if (existsSync(taxonomyPath)) {
    try {
      const data = JSON.parse(readFileSync(taxonomyPath, "utf-8"));
      if (data.owner && data.client && data.domain && data.repo) {
        return {
          owner: data.owner,
          client: data.client,
          domain: data.domain,
          repo: data.repo,
          feature: data.feature || "inter-agent",
        };
      }
    } catch { /* fall through */ }
  }

  // Git remote fallback
  try {
    const remote = execSync("git remote get-url origin", {
      cwd,
      timeout: 3000,
      encoding: "utf-8",
      stdio: ["pipe", "pipe", "pipe"],
    }).trim();
    const match = remote.match(/github\.com[:/]([^/]+)\/([^/.]+)/);
    if (match) {
      return {
        owner: match[1],
        client: "project",
        domain: "general",
        repo: match[2],
        feature: "inter-agent",
      };
    }
  } catch { /* fall through */ }

  return {
    owner: "unknown",
    client: "project",
    domain: "general",
    repo: basename(cwd),
    feature: "inter-agent",
  };
}

/** Format current date as YYYYMMDD in America/New_York timezone */
function dateStr(): string {
  return new Intl.DateTimeFormat("en-CA", {
    timeZone: "America/New_York",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  })
    .format(new Date())
    .replace(/-/g, "");
}

/**
 * Compute the next session log file path for a given agent.
 * Creates the session-logs directory if it doesn't exist.
 * Falls back to ~/.gemini/session-logs/ or ~/.codex/session-logs/ if project dir unwritable.
 */
export function computeAgentLogPath(
  cwd: string,
  agent: "gemini" | "codex"
): string {
  const t = parseTaxonomyFull(cwd);

  // Prefer project session-logs/, fall back to agent home dir
  let sessionLogDir = join(cwd, "session-logs");
  try {
    mkdirSync(sessionLogDir, { recursive: true });
  } catch {
    sessionLogDir = join(homedir(), `.${agent}`, "session-logs");
    mkdirSync(sessionLogDir, { recursive: true });
  }

  // Find highest vN for this specific agent (agents version independently)
  let maxV = 0;
  try {
    const agentFiles = readdirSync(sessionLogDir).filter((f) =>
      f.endsWith(`_${agent}.md`)
    );
    maxV = agentFiles.reduce((max, f) => {
      const m = f.match(/_v(\d+)_/);
      return m ? Math.max(max, parseInt(m[1], 10)) : max;
    }, 0);
  } catch { /* no existing files — start at v1 */ }

  const filename = `${t.owner}--${t.client}_${t.domain}_${t.repo}_${t.feature}_${dateStr()}_v${maxV + 1}_${agent}.md`;
  return join(sessionLogDir, filename);
}
