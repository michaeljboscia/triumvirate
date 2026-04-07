import { writable, type Readable } from 'svelte/store';

export interface LessonRow {
  id: number;
  title: string;
  confidence: number;
  tags?: string;
  updated_at?: string;
}

const lessonsState = writable<LessonRow[]>([]);

function authHeaders(): HeadersInit {
  if (typeof window === 'undefined') {
    return { 'Content-Type': 'application/json' };
  }
  const token = window.localStorage.getItem('triumvirate_daemon_token');
  if (!token) {
    return { 'Content-Type': 'application/json' };
  }
  return {
    'Content-Type': 'application/json',
    Authorization: `Bearer ${token}`
  };
}

export async function loadLessons(): Promise<void> {
  try {
    const res = await fetch('/lesson/list', {
      method: 'POST',
      headers: authHeaders(),
      body: JSON.stringify({ stale_days: 14 })
    });
    if (!res.ok) {
      lessonsState.set([]);
      return;
    }
    const payload = (await res.json()) as { lessons?: LessonRow[] };
    lessonsState.set(payload.lessons ?? []);
  } catch {
    lessonsState.set([]);
  }
}

export async function validateLesson(lessonId: number): Promise<void> {
  try {
    await fetch('/lesson/validate', {
      method: 'POST',
      headers: authHeaders(),
      body: JSON.stringify({ lesson_id: lessonId })
    });
  } finally {
    await loadLessons();
  }
}

export const lessons: Readable<LessonRow[]> = lessonsState;
