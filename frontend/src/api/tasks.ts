import { apiFetch } from './client';
import type { TaskListEntry } from './types';

export async function listTasks(): Promise<TaskListEntry[]> {
  const res = await apiFetch('/api/tasks');
  if (!res.ok) throw new Error(`Failed to list tasks: ${res.status}`);
  return res.json();
}

export async function getTask(id: string): Promise<string> {
  const res = await apiFetch(`/api/tasks/${encodeURIComponent(id)}`);
  if (!res.ok) throw new Error(`Failed to get task: ${res.status}`);
  return res.text();
}

export async function putTask(id: string, yaml: string): Promise<number> {
  const res = await apiFetch(`/api/tasks/${encodeURIComponent(id)}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'text/plain' },
    body: yaml,
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `Failed to save task: ${res.status}`);
  }
  return res.status;
}

export async function deleteTask(id: string): Promise<void> {
  const res = await apiFetch(`/api/tasks/${encodeURIComponent(id)}`, {
    method: 'DELETE',
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(text || `Failed to delete task: ${res.status}`);
  }
}
