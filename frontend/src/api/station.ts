import { apiFetch } from './client';

export interface StationInfo {
  name: string;
}

export async function getStation(): Promise<StationInfo> {
  const res = await apiFetch('/api/station');
  if (!res.ok) throw new Error(`Failed to get station info: ${res.status}`);
  return res.json();
}
