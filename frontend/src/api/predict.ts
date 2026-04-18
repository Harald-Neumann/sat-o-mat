import { apiFetch } from './client';
import type { GroundTrackPredictions, PassPredictions } from './types';

export async function fetchPasses(start: string, end: string): Promise<PassPredictions> {
  const params = new URLSearchParams({ start, end });
  const res = await apiFetch(`/api/predict/passes?${params}`);
  if (!res.ok) throw new Error(`Failed to fetch passes: ${res.status}`);
  return res.json();
}

export async function fetchGroundTracks(start: string, end: string): Promise<GroundTrackPredictions> {
  const params = new URLSearchParams({ start, end });
  const res = await apiFetch(`/api/predict/ground_track?${params}`);
  if (!res.ok) throw new Error(`Failed to fetch ground tracks: ${res.status}`);
  return res.json();
}
