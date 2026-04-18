export const SATELLITE_PALETTE = [
  '#58a6ff', '#3fb950', '#d29922', '#f85149', '#bc8cff',
  '#79c0ff', '#56d364', '#e3b341', '#ff7b72', '#d2a8ff',
  '#7ee787', '#ffa657', '#a5d6ff', '#ff9492', '#e2c0ff',
  '#85e89d', '#ffcc7d', '#94d3ff',
] as const;

/** djb2 hash — stable across runs and browsers. */
function hashName(name: string): number {
  let h = 5381;
  for (let i = 0; i < name.length; i++) {
    h = ((h << 5) + h + name.charCodeAt(i)) | 0;
  }
  return h;
}

/** Deterministic color for a satellite/object name. */
export function colorForName(name: string): string {
  const h = hashName(name);
  const idx = ((h % SATELLITE_PALETTE.length) + SATELLITE_PALETTE.length) % SATELLITE_PALETTE.length;
  return SATELLITE_PALETTE[idx];
}
