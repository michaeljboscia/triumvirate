/**
 * Compute the next session log path for a daemon agent (gemini or codex).
 *
 * Session log filename format (SESSION_LOG_SPEC v1.0):
 *   <owner>--<client>_<domain>_<repo>_<feature>_<YYYYMMDD>_v<N+1>_<agent>.md
 *
 * Version counter is per-agent (gemini and codex count independently).
 *
 * Set AI_MEMORY_DIR to a private repo path — logs go to $AI_MEMORY_DIR/<repo>/.
 * Falls back to <cwd>/session-logs/ if AI_MEMORY_DIR is not configured.
 * Session logs must NOT live inside project repos — they are AI working memory.
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
 *
 * Session logs go to a dedicated private AI memory repo, NOT inside the project.
 * Set AI_MEMORY_DIR to the path of your private memory repo. Each project gets
 * a subfolder named by its repo slug: $AI_MEMORY_DIR/<repo>/<logfile>.md
 *
 * Falls back to <cwd>/session-logs/ if AI_MEMORY_DIR is not set.
 */
export function computeAgentLogPath(
  cwd: string,
  agent: "gemini" | "codex"
): string {
  const t = parseTaxonomyFull(cwd);

  // Resolve memory repo: AI_MEMORY_DIR env var → <cwd>/session-logs fallback
  let sessionLogDir: string;
  if (process.env.AI_MEMORY_DIR) {
    const memoryRepoDir = join(process.env.AI_MEMORY_DIR, t.repo);
    try {
      mkdirSync(memoryRepoDir, { recursive: true });
      sessionLogDir = memoryRepoDir;
    } catch {
      sessionLogDir = join(cwd, "session-logs");
      mkdirSync(sessionLogDir, { recursive: true });
    }
  } else {
    // No memory repo configured — fall back to project session-logs/
    sessionLogDir = join(cwd, "session-logs");
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
