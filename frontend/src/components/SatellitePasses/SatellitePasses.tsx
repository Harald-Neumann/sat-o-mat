import { useState, useEffect, useCallback, useRef } from 'react';
import { fetchPasses } from '../../api/predict';
import type { ApiPass, PassPredictions } from '../../api/types';
import { ElevationChart } from './ElevationChart';
import { colorForName } from '../../theme/colors';
import styles from './SatellitePasses.module.css';

interface SatellitePassesProps {
  timeRange: [number, number];
  onPassSelect?: (satellite: string, pass: ApiPass) => void;
}

interface FlatPass {
  satellite: string;
  pass: ApiPass;
  color: string;
}

function flattenPasses(data: PassPredictions): FlatPass[] {
  const entries = Object.entries(data.predictions);
  const result: FlatPass[] = [];
  entries.forEach(([satellite, passes]) => {
    const color = colorForName(satellite);
    for (const pass of passes) {
      result.push({ satellite, pass, color });
    }
  });
  result.sort((a, b) => new Date(a.pass.start).getTime() - new Date(b.pass.start).getTime());
  return result;
}

function formatTime(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
}

function formatDuration(start: string, end: string): string {
  const ms = new Date(end).getTime() - new Date(start).getTime();
  const totalSec = Math.floor(ms / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  return m > 0 ? `${m}m ${s}s` : `${s}s`;
}

function azToCompass(deg: number): string {
  const dirs = ['N', 'NNE', 'NE', 'ENE', 'E', 'ESE', 'SE', 'SSE', 'S', 'SSW', 'SW', 'WSW', 'W', 'WNW', 'NW', 'NNW'];
  return dirs[Math.round(deg / 22.5) % 16];
}

export function SatellitePasses({ timeRange, onPassSelect }: SatellitePassesProps) {
  const [passes, setPasses] = useState<FlatPass[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [visibleRange, setVisibleRange] = useState<[number, number]>(timeRange);
  const fetchRef = useRef(0);

  const load = useCallback(() => {
    const id = ++fetchRef.current;
    setLoading(true);
    setError(null);
    const start = new Date(timeRange[0]).toISOString();
    const end = new Date(timeRange[1]).toISOString();
    fetchPasses(start, end)
      .then((data) => {
        if (id !== fetchRef.current) return;
        setPasses(flattenPasses(data));
      })
      .catch((err) => {
        if (id !== fetchRef.current) return;
        setError(err.message);
      })
      .finally(() => {
        if (id === fetchRef.current) setLoading(false);
      });
  }, [timeRange]);

  useEffect(load, [load]);

  const handlePassClick = useCallback((satellite: string, pass: ApiPass) => {
    onPassSelect?.(satellite, pass);
  }, [onPassSelect]);

  const handleVisibleRangeChange = useCallback((start: number, end: number) => {
    setVisibleRange([start, end]);
  }, []);

  const visiblePasses = passes.filter((fp) => {
    const s = new Date(fp.pass.start).getTime();
    const e = new Date(fp.pass.end).getTime();
    return s < visibleRange[1] && e > visibleRange[0];
  });

  return (
    <div className={styles.wrapper}>
      <div className={styles.chartSection}>
        {loading && passes.length === 0 && (
          <div className={styles.statusOverlay}>Loading...</div>
        )}
        {error && (
          <div className={styles.statusOverlay}>{error}</div>
        )}
        <ElevationChart
          passes={passes}
          rangeStart={timeRange[0]}
          rangeEnd={timeRange[1]}
          onPassClick={handlePassClick}
          onVisibleRangeChange={handleVisibleRangeChange}
        />
      </div>
      <div className={styles.tableSection}>
        {visiblePasses.length === 0 && !loading ? (
          <div className={styles.empty}>No passes in range</div>
        ) : (
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Satellite</th>
                <th>AOS</th>
                <th>Max El</th>
                <th>Az @ AOS</th>
                <th>Az @ LOS</th>
                <th>Duration</th>
              </tr>
            </thead>
            <tbody>
              {visiblePasses.map((fp, i) => {
                const maxEl = Math.max(...fp.pass.elevation);
                const aosAz = fp.pass.azimuth[0];
                const losAz = fp.pass.azimuth[fp.pass.azimuth.length - 1];
                return (
                  <tr
                    key={`${fp.satellite}-${i}`}
                    className={styles.clickable}
                    onClick={() => handlePassClick(fp.satellite, fp.pass)}
                  >
                    <td>
                      <span className={styles.satDot} style={{ background: fp.color }} />
                      {fp.satellite}
                    </td>
                    <td className={styles.mono}>{formatTime(fp.pass.start)}</td>
                    <td className={styles.mono}>{maxEl.toFixed(1)}°</td>
                    <td className={styles.mono}>{aosAz.toFixed(1)}° {azToCompass(aosAz)}</td>
                    <td className={styles.mono}>{losAz.toFixed(1)}° {azToCompass(losAz)}</td>
                    <td className={styles.mono}>{formatDuration(fp.pass.start, fp.pass.end)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
