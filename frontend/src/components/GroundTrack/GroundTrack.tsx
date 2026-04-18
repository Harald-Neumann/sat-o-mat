import { useCallback, useEffect, useRef, useState } from 'react';
import { feature as topoFeature } from 'topojson-client';
import type { Topology, GeometryCollection } from 'topojson-specification';
import type { Feature, MultiPolygon, Polygon } from 'geojson';
import landTopo from 'world-atlas/land-110m.json';
import { fetchGroundTracks } from '../../api/predict';
import type { GroundTrackPredictions } from '../../api/types';
import { getColor } from '../SatellitePasses/ElevationChart';
import styles from './GroundTrack.module.css';

type Ring = [number, number][];

function extractLandRings(): Ring[] {
  const topology = landTopo as unknown as Topology<{ land: GeometryCollection }>;
  const fc = topoFeature(topology, topology.objects.land) as
    | Feature<Polygon | MultiPolygon>
    | { type: 'FeatureCollection'; features: Feature<Polygon | MultiPolygon>[] };

  const features: Feature<Polygon | MultiPolygon>[] =
    fc.type === 'FeatureCollection' ? fc.features : [fc];

  const rings: Ring[] = [];
  for (const f of features) {
    const coords = f.geometry.coordinates;
    const polys: Ring[][] =
      f.geometry.type === 'Polygon'
        ? [coords as Ring[]]
        : (coords as Ring[][]);
    for (const poly of polys) {
      for (const ring of poly) rings.push(ring);
    }
  }
  return rings;
}

const LAND_RINGS: Ring[] = extractLandRings();

interface GroundTrackProps {
  /** Minutes of past track to show. Default 30. */
  pastMinutes?: number;
  /** Minutes of future track to show. Default 90. */
  futureMinutes?: number;
  /** How often to refetch track data (seconds). Default 300. */
  refetchSeconds?: number;
  /** How often to advance the current-position marker (seconds). Default 5. */
  tickSeconds?: number;
}

interface TrackEntry {
  id: string;
  color: string;
  startMs: number;
  stepMs: number;
  latitude: number[];
  longitude: number[];
}

const MIN_SEGMENT_PX = 0.5;
const MARKER_RADIUS = 5;

function buildEntries(
  data: GroundTrackPredictions,
  requestEndMs: number,
): TrackEntry[] {
  const entries = Object.entries(data.predictions);
  entries.sort(([a], [b]) => a.localeCompare(b));
  return entries.map(([id, track], idx) => {
    const startMs = new Date(track.start).getTime();
    const n = track.longitude.length;
    const stepMs = n > 1 ? (requestEndMs - startMs) / (n - 1) : 0;
    return {
      id,
      color: getColor(idx),
      startMs,
      stepMs,
      latitude: track.latitude,
      longitude: track.longitude,
    };
  });
}

/** Interpolate lat/lon at a given time. Returns null if outside the track window. */
function positionAt(entry: TrackEntry, tMs: number): { lat: number; lon: number } | null {
  const n = entry.longitude.length;
  if (n === 0 || entry.stepMs <= 0) return null;
  const f = (tMs - entry.startMs) / entry.stepMs;
  if (f < 0 || f > n - 1) return null;
  const i0 = Math.floor(f);
  const i1 = Math.min(i0 + 1, n - 1);
  const frac = f - i0;
  const lat = entry.latitude[i0] + (entry.latitude[i1] - entry.latitude[i0]) * frac;

  const lon0 = entry.longitude[i0];
  let lon1 = entry.longitude[i1];
  if (lon1 - lon0 > 180) lon1 -= 360;
  else if (lon0 - lon1 > 180) lon1 += 360;
  let lon = lon0 + (lon1 - lon0) * frac;
  while (lon > 180) lon -= 360;
  while (lon < -180) lon += 360;

  return { lat, lon };
}

function project(lon: number, lat: number, width: number, height: number) {
  return {
    x: ((lon + 180) / 360) * width,
    y: ((90 - lat) / 180) * height,
  };
}

function drawLand(ctx: CanvasRenderingContext2D, w: number, h: number) {
  ctx.fillStyle = '#1a2536';
  ctx.strokeStyle = '#2a394d';
  ctx.lineWidth = 0.6;
  ctx.beginPath();
  for (const ring of LAND_RINGS) {
    if (ring.length < 2) continue;
    let prevLon = ring[0][0];
    let p = project(prevLon, ring[0][1], w, h);
    ctx.moveTo(p.x, p.y);
    for (let i = 1; i < ring.length; i++) {
      const lon = ring[i][0];
      const lat = ring[i][1];
      p = project(lon, lat, w, h);
      if (Math.abs(lon - prevLon) > 180) {
        // Date-line crossing: start a new subpath
        ctx.moveTo(p.x, p.y);
      } else {
        ctx.lineTo(p.x, p.y);
      }
      prevLon = lon;
    }
  }
  ctx.fill('evenodd');
  ctx.stroke();
}

function drawMap(ctx: CanvasRenderingContext2D, w: number, h: number) {
  // Ocean background
  ctx.fillStyle = '#0b1420';
  ctx.fillRect(0, 0, w, h);

  drawLand(ctx, w, h);

  // Minor grid every 15°
  ctx.strokeStyle = '#1b2430';
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let lon = -180; lon <= 180; lon += 15) {
    const x = ((lon + 180) / 360) * w;
    ctx.moveTo(x, 0);
    ctx.lineTo(x, h);
  }
  for (let lat = -90; lat <= 90; lat += 15) {
    const y = ((90 - lat) / 180) * h;
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
  }
  ctx.stroke();

  // Major grid every 30°
  ctx.strokeStyle = '#263141';
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let lon = -180; lon <= 180; lon += 30) {
    const x = ((lon + 180) / 360) * w;
    ctx.moveTo(x, 0);
    ctx.lineTo(x, h);
  }
  for (let lat = -90; lat <= 90; lat += 30) {
    const y = ((90 - lat) / 180) * h;
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
  }
  ctx.stroke();

  // Equator and prime meridian
  ctx.strokeStyle = '#3a485c';
  ctx.lineWidth = 1;
  ctx.beginPath();
  const eqY = h / 2;
  ctx.moveTo(0, eqY);
  ctx.lineTo(w, eqY);
  const pmX = w / 2;
  ctx.moveTo(pmX, 0);
  ctx.lineTo(pmX, h);
  ctx.stroke();

  // Border
  ctx.strokeStyle = '#30363d';
  ctx.lineWidth = 1;
  ctx.strokeRect(0.5, 0.5, w - 1, h - 1);

  // Axis labels
  ctx.fillStyle = '#484f58';
  ctx.font = '10px sans-serif';
  ctx.textBaseline = 'top';
  ctx.textAlign = 'left';
  for (let lon = -180; lon <= 150; lon += 60) {
    const x = ((lon + 180) / 360) * w;
    ctx.fillText(`${lon}°`, x + 2, 2);
  }
  ctx.textAlign = 'right';
  for (let lat = 60; lat >= -60; lat -= 30) {
    const y = ((90 - lat) / 180) * h;
    ctx.fillText(`${lat}°`, w - 4, y + 2);
  }
}

function drawTrack(
  ctx: CanvasRenderingContext2D,
  entry: TrackEntry,
  w: number,
  h: number,
  nowMs: number,
) {
  const n = entry.longitude.length;
  if (n < 2) return;

  // Past (dashed), future (solid)
  const currentF = entry.stepMs > 0 ? (nowMs - entry.startMs) / entry.stepMs : -1;

  ctx.lineWidth = 1.5;

  const drawSegment = (from: number, to: number, past: boolean) => {
    if (to - from < 1) return;
    ctx.strokeStyle = entry.color;
    ctx.globalAlpha = past ? 0.35 : 0.9;
    ctx.setLineDash(past ? [3, 3] : []);

    ctx.beginPath();
    let last = project(entry.longitude[from], entry.latitude[from], w, h);
    ctx.moveTo(last.x, last.y);
    for (let i = from + 1; i <= to; i++) {
      const lon = entry.longitude[i];
      const lat = entry.latitude[i];
      const prevLon = entry.longitude[i - 1];
      if (Math.abs(lon - prevLon) > 180) {
        // Date-line crossing: break the path
        ctx.stroke();
        ctx.beginPath();
        last = project(lon, lat, w, h);
        ctx.moveTo(last.x, last.y);
        continue;
      }
      const p = project(lon, lat, w, h);
      if (Math.hypot(p.x - last.x, p.y - last.y) >= MIN_SEGMENT_PX) {
        ctx.lineTo(p.x, p.y);
        last = p;
      }
    }
    ctx.stroke();
  };

  if (currentF <= 0) {
    drawSegment(0, n - 1, false);
  } else if (currentF >= n - 1) {
    drawSegment(0, n - 1, true);
  } else {
    const mid = Math.floor(currentF);
    drawSegment(0, mid, true);
    drawSegment(mid, n - 1, false);
  }

  ctx.globalAlpha = 1;
  ctx.setLineDash([]);
}

function drawMarker(
  ctx: CanvasRenderingContext2D,
  entry: TrackEntry,
  w: number,
  h: number,
  nowMs: number,
) {
  const pos = positionAt(entry, nowMs);
  if (!pos) return;
  const { x, y } = project(pos.lon, pos.lat, w, h);

  ctx.save();
  // Outer glow
  ctx.fillStyle = entry.color;
  ctx.globalAlpha = 0.25;
  ctx.beginPath();
  ctx.arc(x, y, MARKER_RADIUS + 4, 0, Math.PI * 2);
  ctx.fill();

  // Solid core
  ctx.globalAlpha = 1;
  ctx.beginPath();
  ctx.arc(x, y, MARKER_RADIUS, 0, Math.PI * 2);
  ctx.fill();
  ctx.strokeStyle = '#0d1117';
  ctx.lineWidth = 1.5;
  ctx.stroke();

  // Label
  ctx.font = '11px sans-serif';
  ctx.textBaseline = 'middle';
  const textX = x + MARKER_RADIUS + 6;
  const textFits = textX + ctx.measureText(entry.id).width + 4 < w;
  ctx.textAlign = textFits ? 'left' : 'right';
  const labelX = textFits ? textX : x - MARKER_RADIUS - 6;
  const metrics = ctx.measureText(entry.id);
  const padX = 4;
  const boxW = metrics.width + padX * 2;
  const boxH = 14;
  const boxX = textFits ? labelX - padX : labelX - metrics.width - padX;
  ctx.fillStyle = 'rgba(13, 17, 23, 0.75)';
  ctx.fillRect(boxX, y - boxH / 2, boxW, boxH);
  ctx.fillStyle = entry.color;
  ctx.fillText(entry.id, labelX, y);
  ctx.restore();
}

interface Hover {
  id: string;
  color: string;
  timeMs: number;
  lat: number;
  lon: number;
  px: number;
  py: number;
  containerW: number;
  containerH: number;
  sampleIndex: number;
}

const HOVER_THRESHOLD_PX = 10;

function formatUtc(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => n.toString().padStart(2, '0');
  return (
    `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())} ` +
    `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())}Z`
  );
}

function HoverTooltip({ hover }: { hover: Hover }) {
  const latLabel = `${Math.abs(hover.lat).toFixed(2)}°${hover.lat >= 0 ? 'N' : 'S'}`;
  const lonLabel = `${Math.abs(hover.lon).toFixed(2)}°${hover.lon >= 0 ? 'E' : 'W'}`;
  const offset = 14;
  const flipX = hover.px > hover.containerW - 180;
  const flipY = hover.py > hover.containerH - 80;
  const style: React.CSSProperties = {
    left: flipX ? undefined : hover.px + offset,
    right: flipX ? hover.containerW - hover.px + offset : undefined,
    top: flipY ? undefined : hover.py + offset,
    bottom: flipY ? hover.containerH - hover.py + offset : undefined,
  };
  return (
    <div className={styles.tooltip} style={style}>
      <div className={styles.tooltipTitle}>
        <span className={styles.tooltipDot} style={{ background: hover.color }} />
        {hover.id}
      </div>
      <div className={styles.tooltipLine}>{formatUtc(hover.timeMs)}</div>
      <div className={styles.tooltipLine}>
        {latLabel}, {lonLabel}
      </div>
    </div>
  );
}

export function GroundTrack({
  pastMinutes = 30,
  futureMinutes = 90,
  refetchSeconds = 300,
  tickSeconds = 5,
}: GroundTrackProps) {
  const [entries, setEntries] = useState<TrackEntry[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [nowMs, setNowMs] = useState<number>(() => Date.now());
  const [hover, setHover] = useState<Hover | null>(null);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let cancelled = false;
    const run = () => {
      const start = new Date(Date.now() - pastMinutes * 60_000);
      const end = new Date(Date.now() + futureMinutes * 60_000);
      fetchGroundTracks(start.toISOString(), end.toISOString())
        .then((data) => {
          if (cancelled) return;
          setEntries(buildEntries(data, end.getTime()));
          setError(null);
          setLoaded(true);
        })
        .catch((err: Error) => {
          if (cancelled) return;
          setError(err.message);
          setLoaded(true);
        });
    };
    run();
    const id = window.setInterval(run, refetchSeconds * 1000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [pastMinutes, futureMinutes, refetchSeconds]);

  useEffect(() => {
    const id = window.setInterval(() => setNowMs(Date.now()), tickSeconds * 1000);
    return () => window.clearInterval(id);
  }, [tickSeconds]);

  // Draw on canvas
  useEffect(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container) return;

    const render = () => {
      const rect = container.getBoundingClientRect();
      const w = Math.max(1, Math.floor(rect.width));
      const h = Math.max(1, Math.floor(rect.height));
      const dpr = window.devicePixelRatio || 1;
      if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
        canvas.width = w * dpr;
        canvas.height = h * dpr;
      }
      const ctx = canvas.getContext('2d');
      if (!ctx) return;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      drawMap(ctx, w, h);
      const visible = entries.filter((e) => !hidden.has(e.id));
      for (const entry of visible) drawTrack(ctx, entry, w, h, nowMs);
      for (const entry of visible) drawMarker(ctx, entry, w, h, nowMs);
    };

    render();
    const ro = new ResizeObserver(render);
    ro.observe(container);
    return () => ro.disconnect();
  }, [entries, hidden, nowMs]);

  const toggle = useCallback((id: string) => {
    setHidden((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const showAll = useCallback(() => setHidden(new Set()), []);
  const hideAll = useCallback(() => {
    setHidden(new Set(entries.map((e) => e.id)));
  }, [entries]);

  const handleMouseMove = useCallback((e: React.MouseEvent<HTMLDivElement>) => {
    const container = containerRef.current;
    if (!container) return;
    const rect = container.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;

    let best: Hover | null = null;
    let bestDist = HOVER_THRESHOLD_PX * HOVER_THRESHOLD_PX;

    for (const entry of entries) {
      if (hidden.has(entry.id)) continue;
      const lats = entry.latitude;
      const lons = entry.longitude;
      for (let i = 0; i < lons.length; i++) {
        const px = ((lons[i] + 180) / 360) * w;
        const py = ((90 - lats[i]) / 180) * h;
        const dx = px - mx;
        const dy = py - my;
        const d2 = dx * dx + dy * dy;
        if (d2 < bestDist) {
          bestDist = d2;
          best = {
            id: entry.id,
            color: entry.color,
            timeMs: entry.startMs + i * entry.stepMs,
            lat: lats[i],
            lon: lons[i],
            px,
            py,
            containerW: w,
            containerH: h,
            sampleIndex: i,
          };
        }
      }
    }

    setHover((prev) => {
      if (!best) return prev === null ? prev : null;
      if (prev && prev.id === best.id && prev.sampleIndex === best.sampleIndex) return prev;
      return best;
    });
  }, [entries, hidden]);

  const handleMouseLeave = useCallback(() => setHover(null), []);

  return (
    <div className={styles.wrapper}>
      <div
        className={styles.mapSection}
        ref={containerRef}
        onMouseMove={handleMouseMove}
        onMouseLeave={handleMouseLeave}
      >
        <canvas ref={canvasRef} className={styles.canvas} />
        {hover && <HoverTooltip hover={hover} />}
        {!loaded && entries.length === 0 && !error && (
          <div className={styles.statusOverlay}>Loading...</div>
        )}
        {error && <div className={styles.statusOverlay}>{error}</div>}
      </div>
      <div className={styles.sidebar}>
        <div className={styles.sidebarHeader}>
          <span>Satellites</span>
          <div className={styles.sidebarActions}>
            <button className={styles.sidebarAction} onClick={showAll}>All</button>
            <button className={styles.sidebarAction} onClick={hideAll}>None</button>
          </div>
        </div>
        <div className={styles.satList}>
          {entries.length === 0 && loaded ? (
            <div className={styles.empty}>No satellites</div>
          ) : (
            entries.map((e) => {
              const isHidden = hidden.has(e.id);
              return (
                <label key={e.id} className={styles.satRow}>
                  <input
                    type="checkbox"
                    checked={!isHidden}
                    onChange={() => toggle(e.id)}
                  />
                  <span className={styles.satDot} style={{ background: e.color }} />
                  <span
                    className={`${styles.satName} ${isHidden ? styles.satNameMuted : ''}`}
                    title={e.id}
                  >
                    {e.id}
                  </span>
                </label>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}
