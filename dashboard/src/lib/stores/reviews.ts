import { derived, writable, type Readable } from 'svelte/store';

export interface ReviewEntry {
  review_id: string;
  reviewer_agent: string;
  verdict: 'approve' | 'request_changes' | 'pending';
  requested_at_ms: number;
  completed_at_ms?: number;
}

const reviewsState = writable<ReviewEntry[]>([]);
let socket: WebSocket | null = null;

function wsUrl(): string {
  if (typeof window === 'undefined') {
    return 'ws://127.0.0.1:8080/ws';
  }
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}/ws`;
}

function ingest(raw: string): void {
  let parsed: { type?: string; ts_ms?: number; payload?: Record<string, unknown> };
  try {
    parsed = JSON.parse(raw);
  } catch {
    return;
  }
  const ts = parsed.ts_ms ?? Date.now();

  if (parsed.type === 'review_completed') {
    const payload = parsed.payload ?? {};
    const review_id = String(payload.review_id ?? `review-${ts}`);
    const reviewer_agent = String(payload.reviewer_agent ?? 'unknown');
    const verdictRaw = String(payload.verdict ?? 'approve').toLowerCase();
    const verdict = verdictRaw === 'request_changes' ? 'request_changes' : 'approve';

    reviewsState.update((current) => {
      const existingIdx = current.findIndex((entry) => entry.review_id === review_id);
      const nextEntry: ReviewEntry = {
        review_id,
        reviewer_agent,
        verdict,
        requested_at_ms: ts,
        completed_at_ms: ts
      };
      if (existingIdx >= 0) {
        const clone = [...current];
        clone[existingIdx] = { ...clone[existingIdx], ...nextEntry };
        return clone;
      }
      return [nextEntry, ...current].slice(0, 300);
    });
  }
}

export function connectReviewStream(): void {
  if (typeof window === 'undefined' || socket) {
    return;
  }
  socket = new WebSocket(wsUrl());
  socket.onmessage = (event) => ingest(String(event.data));
  socket.onclose = () => {
    socket = null;
  };
}

export function disconnectReviewStream(): void {
  if (socket) {
    socket.close();
    socket = null;
  }
}

export const reviews: Readable<ReviewEntry[]> = reviewsState;

export const pendingReviews: Readable<ReviewEntry[]> = derived(reviewsState, ($reviews) =>
  $reviews.filter((review) => !review.completed_at_ms)
);

export const reviewHistory: Readable<ReviewEntry[]> = derived(reviewsState, ($reviews) =>
  $reviews.filter((review) => Boolean(review.completed_at_ms))
);

export const approvalRateByAgent: Readable<Record<string, number>> = derived(reviewsState, ($reviews) => {
  const totals = new Map<string, { approve: number; total: number }>();
  for (const review of $reviews) {
    if (!review.completed_at_ms) {
      continue;
    }
    const bucket = totals.get(review.reviewer_agent) ?? { approve: 0, total: 0 };
    bucket.total += 1;
    if (review.verdict === 'approve') {
      bucket.approve += 1;
    }
    totals.set(review.reviewer_agent, bucket);
  }

  const out: Record<string, number> = {};
  for (const [agent, bucket] of totals.entries()) {
    out[agent] = bucket.total === 0 ? 0 : bucket.approve / bucket.total;
  }
  return out;
});
