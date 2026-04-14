import { apiFetch } from './client';
import type { PassPredictions } from './types';

export async function fetchPasses(start: string, end: string): Promise<PassPredictions> {
  const params = new URLSearchParams({ start, end });
  const res = await apiFetch(`/api/predict/passes?${params}`);
  if (!res.ok) throw new Error(`Failed to fetch passes: ${res.status}`);
  return res.json();
}
