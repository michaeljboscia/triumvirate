/**
 * Gemini model fallback chain with quota-exhaustion tracking.
 *
 * Chain (pro-first across generations, then flash):
 *   gemini-3-pro-preview  →  gemini-2.5-pro  →  gemini-3-flash-preview  →  gemini-2.5-flash
 *
 * State persists in ~/.gemini/quota-state.json with a 1-hour TTL per exhausted model.
 * After TTL expires the model is retried automatically (quota windows typically reset hourly).
 */

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";

// ─── Chain & State ────────────────────────────────────────────────────────────

export const MODEL_CHAIN = [
  "gemini-3-pro-preview",
  "gemini-2.5-pro",
  "gemini-3-flash-preview",
  "gemini-2.5-flash",
] as const;

export type GeminiModel = (typeof MODEL_CHAIN)[number];

const STATE_FILE = join(homedir(), ".gemini", "quota-state.json");
const QUOTA_TTL_MS = 60 * 60 * 1000; // 1 hour

interface ExhaustedEntry {
  model: string;
  exhausted_at: number;
}

interface QuotaState {
  exhausted: ExhaustedEntry[];
}

// ─── Quota error detection ────────────────────────────────────────────────────

/** Patterns emitted by Gemini CLI when quota/rate-limit is hit */
const QUOTA_PATTERNS = [
  /RESOURCE_EXHAUSTED/i,
  /quota.*exceeded/i,
  /exceeded.*quota/i,
  /rate.?limit/i,
  /too many requests/i,
  /\b429\b/,
  /you.ve exceeded your/i,
  /per.*minute.*limit/i,
  /daily.*limit/i,
];

export function isQuotaError(stderr: string, stdout: string): boolean {
  const combined = stderr + stdout;
  return QUOTA_PATTERNS.some((p) => p.test(combined));
}

// ─── State I/O ────────────────────────────────────────────────────────────────

function readState(): QuotaState {
  try {
    if (existsSync(STATE_FILE)) {
      return JSON.parse(readFileSync(STATE_FILE, "utf-8")) as QuotaState;
    }
  } catch {
    /* corrupt or missing — start fresh */
  }
  return { exhausted: [] };
}

function writeState(state: QuotaState): void {
  try {
    writeFileSync(STATE_FILE, JSON.stringify(state, null, 2), "utf-8");
  } catch {
    /* non-fatal: next request will just retry the model and fail quickly */
  }
}

/** Remove entries older than QUOTA_TTL_MS */
function pruneExpired(state: QuotaState): QuotaState {
  const cutoff = Date.now() - QUOTA_TTL_MS;
  return { exhausted: state.exhausted.filter((e) => e.exhausted_at > cutoff) };
}

function getExhaustedSet(): Set<string> {
  const state = pruneExpired(readState());
  return new Set(state.exhausted.map((e) => e.model));
}

// ─── Public API ───────────────────────────────────────────────────────────────

/**
 * Mark a model as quota-exhausted.
 * Persists to disk and updates in-memory cache.
 */
export function reportExhausted(model: string): void {
  let state = pruneExpired(readState());
  state.exhausted = state.exhausted.filter((e) => e.model !== model);
  state.exhausted.push({ model, exhausted_at: Date.now() });
  writeState(state);
}

/**
 * Returns all non-exhausted models in preferred order.
 * Always includes at least the last model in the chain as an emergency fallback.
 */
export function getAvailableModels(): string[] {
  const exhausted = getExhaustedSet();
  const available = MODEL_CHAIN.filter((m) => !exhausted.has(m));
  // Always guarantee at least one option
  if (available.length === 0) {
    return [MODEL_CHAIN[MODEL_CHAIN.length - 1]];
  }
  return available;
}

/**
 * Returns the current best model (first non-exhausted in chain).
 */
export function getCurrentModel(): string {
  return getAvailableModels()[0];
}

/**
 * Returns the `--model <name>` CLI args for the current best model.
 */
export function getModelArgs(): string[] {
  return ["--model", getCurrentModel()];
}

/**
 * Returns a summary of current quota state (for logging/debugging).
 */
export function getQuotaStatus(): string {
  const exhausted = getExhaustedSet();
  const current = getCurrentModel();
  const exhaustedList = [...exhausted];
  if (exhaustedList.length === 0) {
    return `Model: ${current} (all models available)`;
  }
  return `Model: ${current} | Exhausted: ${exhaustedList.join(", ")}`;
}
