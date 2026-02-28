/**
 * In-memory job store for async inter-agent requests.
 *
 * When send_message is called, the CLI process is spawned and a job is created.
 * The tool returns immediately with a job_id (SYN/ACK). The caller then uses
 * get_response(job_id) to retrieve the result when ready.
 *
 * Jobs auto-expire after MAX_JOB_TTL_MS to prevent memory leaks from
 * abandoned requests (session crash, compaction loses job_id, etc.).
 */

import type { ChildProcess } from "node:child_process";
import type { ExecutionResult } from "./types.js";

/** How long to keep completed/failed jobs before garbage collection */
const COMPLETED_JOB_TTL_MS = 5 * 60_000; // 5 minutes after completion

/** How long before an in-flight job is considered abandoned */
const MAX_JOB_TTL_MS = 10 * 60_000; // 10 minutes total lifetime

/** Max concurrent jobs per server to prevent runaway */
const MAX_CONCURRENT_JOBS = 10;

export type JobStatus = "running" | "completed" | "failed" | "expired";

export interface Job {
  id: string;
  target: string; // "gemini" or "codex"
  pid: number | null;
  status: JobStatus;
  created_at: number;
  completed_at: number | null;
  result: ExecutionResult | null;
  /** Reference to the running process (for potential cancellation) */
  process: ChildProcess | null;
}

/** Minimal job info returned to the caller (no process handle) */
export interface JobInfo {
  id: string;
  target: string;
  pid: number | null;
  status: JobStatus;
  elapsed_ms: number;
  result: ExecutionResult | null;
}

let jobCounter = 0;

const jobs = new Map<string, Job>();

/** Generate a short, unique job ID */
function generateJobId(target: string): string {
  const prefix = target === "gemini" ? "gm" : "cx";
  const ts = Date.now().toString(36);
  jobCounter++;
  return `${prefix}_${ts}_${jobCounter}`;
}

/** Run garbage collection on expired/old jobs */
function gc(): void {
  const now = Date.now();
  for (const [id, job] of jobs) {
    // Expire completed jobs after TTL
    if (
      job.status === "completed" || job.status === "failed" || job.status === "expired"
    ) {
      if (job.completed_at && now - job.completed_at > COMPLETED_JOB_TTL_MS) {
        jobs.delete(id);
      }
    }
    // Expire abandoned running jobs
    if (job.status === "running" && now - job.created_at > MAX_JOB_TTL_MS) {
      job.status = "expired";
      job.completed_at = now;
      // Kill the process if still running
      if (job.process && !job.process.killed) {
        job.process.kill("SIGTERM");
      }
      job.process = null;
    }
  }
}

/** Create a new job and return its ID. Throws if at capacity. */
export function createJob(target: string, process: ChildProcess): string {
  gc(); // clean up before checking capacity

  const runningCount = Array.from(jobs.values()).filter(
    (j) => j.status === "running"
  ).length;
  if (runningCount >= MAX_CONCURRENT_JOBS) {
    throw new Error(
      `Too many concurrent jobs (${runningCount}/${MAX_CONCURRENT_JOBS}). Wait for existing jobs to complete.`
    );
  }

  const id = generateJobId(target);
  jobs.set(id, {
    id,
    target,
    pid: process.pid ?? null,
    status: "running",
    created_at: Date.now(),
    completed_at: null,
    result: null,
    process,
  });

  return id;
}

/** Mark a job as completed with its result */
export function completeJob(id: string, result: ExecutionResult): void {
  const job = jobs.get(id);
  if (!job) return;
  job.status = result.success ? "completed" : "failed";
  job.completed_at = Date.now();
  job.result = result;
  job.process = null; // release process handle
}

/** Get job info (safe to return to caller — no process handle) */
export function getJob(id: string): JobInfo | null {
  gc();
  const job = jobs.get(id);
  if (!job) return null;
  return {
    id: job.id,
    target: job.target,
    pid: job.pid,
    status: job.status,
    elapsed_ms: Date.now() - job.created_at,
    result: job.result,
  };
}

/**
 * Wait for a job to complete. Resolves when the job finishes or expires.
 * Polls internally — the caller blocks on the returned promise.
 */
export function waitForJob(id: string, timeout_ms?: number): Promise<JobInfo | null> {
  const effectiveTimeout = timeout_ms || MAX_JOB_TTL_MS;
  const startTime = Date.now();

  return new Promise((resolve) => {
    const check = () => {
      const job = getJob(id);
      if (!job) {
        resolve(null);
        return;
      }
      if (job.status !== "running") {
        resolve(job);
        return;
      }
      if (Date.now() - startTime > effectiveTimeout) {
        resolve(job); // return as-is (still running but caller timed out)
        return;
      }
      setTimeout(check, 500); // poll every 500ms internally
    };
    check();
  });
}

/** List all active jobs (for debugging) */
export function listJobs(): JobInfo[] {
  gc();
  return Array.from(jobs.values()).map((job) => ({
    id: job.id,
    target: job.target,
    pid: job.pid,
    status: job.status,
    elapsed_ms: Date.now() - job.created_at,
    result: job.result,
  }));
}
