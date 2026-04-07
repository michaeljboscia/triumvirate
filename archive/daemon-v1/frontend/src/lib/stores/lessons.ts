import { writable } from 'svelte/store';

export interface LessonRecord {
  id: number;
  decision: string;
  rationale: string;
  outcome: 'success' | 'failure' | 'partial';
  confidence_score: number;
  effective_confidence: number;
  pattern: string;
  agent_source: string;
  created_at: string;
}

export interface LessonsFilter {
  outcome?: string;
  agent_source?: string;
  pattern?: string;
  min_confidence?: number;
}

export const lessons = writable<LessonRecord[]>([]);

export async function refreshLessons(filter: LessonsFilter = {}, baseUrl = ''): Promise<void> {
  const params = new URLSearchParams();
  if (filter.outcome) params.set('outcome', filter.outcome);
  if (filter.agent_source) params.set('agent_source', filter.agent_source);
  if (filter.pattern) params.set('pattern', filter.pattern);
  if (typeof filter.min_confidence === 'number') {
    params.set('min_confidence', String(filter.min_confidence));
  }
  const suffix = params.toString() ? `?${params.toString()}` : '';
  const res = await fetch(`${baseUrl}/api/lessons${suffix}`);
  if (!res.ok) return;
  const data = (await res.json()) as { lessons: LessonRecord[] };
  lessons.set(data.lessons ?? []);
}
