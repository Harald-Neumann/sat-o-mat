import { useRef, useEffect } from 'react';
import {
  Chart,
  ScatterController,
  LineElement,
  PointElement,
  RadialLinearScale,
  Tooltip,
} from 'chart.js';
import type { ApiPass } from '../../api/types';
import { colorForName } from '../../theme/colors';
import styles from './SatellitePasses.module.css';

Chart.register(ScatterController, LineElement, PointElement, RadialLinearScale, Tooltip);

interface PolarPlotDialogProps {
  satellite: string;
  pass: ApiPass;
  onClose: () => void;
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

/** Convert az/el to cartesian x/y for a polar projection (0°el = edge, 90°el = center). */
function toXY(az: number, el: number): { x: number; y: number } {
  const r = 90 - el; // invert: horizon=90, zenith=0
  const rad = ((az - 90) * Math.PI) / 180; // rotate so 0°az = North (up)
  return { x: r * Math.cos(rad), y: r * Math.sin(rad) };
}

function downsample<T>(arr: T[], max: number): T[] {
  if (arr.length <= max) return arr;
  const step = (arr.length - 1) / (max - 1);
  const out: T[] = [];
  for (let i = 0; i < max; i++) out.push(arr[Math.round(i * step)]);
  return out;
}

export function PolarPlotDialog({ satellite, pass, onClose }: PolarPlotDialogProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const chartRef = useRef<Chart | null>(null);

  const maxEl = Math.max(...pass.elevation);
  const maxElIdx = pass.elevation.indexOf(maxEl);
  const trackColor = colorForName(satellite);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    chartRef.current?.destroy();

    // Build the track data (downsampled for perf)
    const indices = Array.from({ length: pass.azimuth.length }, (_, i) => i);
    const sampled = downsample(indices, 200);
    const trackData = sampled.map((i) => toXY(pass.azimuth[i], pass.elevation[i]));

    // Key points
    const aosPoint = toXY(pass.azimuth[0], pass.elevation[0]);
    const losPoint = toXY(pass.azimuth[pass.azimuth.length - 1], pass.elevation[pass.elevation.length - 1]);
    const maxPoint = toXY(pass.azimuth[maxElIdx], pass.elevation[maxElIdx]);

    chartRef.current = new Chart(canvas, {
      type: 'scatter',
      data: {
        datasets: [
          {
            label: 'Track',
            data: trackData,
            showLine: true,
            borderColor: trackColor,
            backgroundColor: trackColor + '18',
            borderWidth: 2,
            pointRadius: 0,
            pointHitRadius: 6,
            pointHoverRadius: 3,
            pointHoverBackgroundColor: trackColor,
            fill: false,
            tension: 0.2,
          },
          {
            label: 'AOS',
            data: [aosPoint],
            pointRadius: 7,
            pointBackgroundColor: '#3fb950',
            pointBorderColor: '#3fb950',
            pointBorderWidth: 0,
            showLine: false,
          },
          {
            label: `Max El (${maxEl.toFixed(1)}°)`,
            data: [maxPoint],
            pointRadius: 7,
            pointBackgroundColor: '#d29922',
            pointBorderColor: '#d29922',
            pointBorderWidth: 0,
            showLine: false,
          },
          {
            label: 'LOS',
            data: [losPoint],
            pointRadius: 7,
            pointBackgroundColor: '#f85149',
            pointBorderColor: '#f85149',
            pointBorderWidth: 0,
            showLine: false,
          },
        ],
      },
      options: {
        responsive: true,
        maintainAspectRatio: true,
        aspectRatio: 1,
        animation: false,
        interaction: {
          mode: 'nearest',
          intersect: true,
        },
        layout: { padding: 20 },
        scales: {
          x: {
            min: -100,
            max: 100,
            display: false,
          },
          y: {
            min: -100,
            max: 100,
            display: false,
          },
        },
        plugins: {
          legend: {
            display: true,
            position: 'bottom',
            labels: {
              color: '#8b949e',
              font: { size: 11 },
              padding: 12,
              usePointStyle: true,
              pointStyleWidth: 10,
              filter: (item) => item.text !== 'Track',
            },
          },
          tooltip: {
            backgroundColor: '#21262d',
            titleColor: '#c9d1d9',
            bodyColor: '#8b949e',
            borderColor: '#30363d',
            borderWidth: 1,
            cornerRadius: 4,
            padding: 8,
            callbacks: {
              label: (ctx) => {
                const px = ctx.parsed.x ?? 0;
                const py = ctx.parsed.y ?? 0;
                const r = Math.sqrt(px * px + py * py);
                const el = 90 - r;
                let az = (Math.atan2(py, px) * 180) / Math.PI + 90;
                if (az < 0) az += 360;
                return ` Az: ${az.toFixed(1)}°  El: ${el.toFixed(1)}°`;
              },
            },
          },
        },
      },
      plugins: [
        {
          id: 'polarGrid',
          beforeDraw(chart) {
            const { ctx, chartArea } = chart;
            if (!chartArea) return;
            const cx = (chartArea.left + chartArea.right) / 2;
            const cy = (chartArea.top + chartArea.bottom) / 2;
            const plotRadius = Math.min(chartArea.right - chartArea.left, chartArea.bottom - chartArea.top) / 2;
            // Scale factor: data range is -100..100, chart maps to pixel area
            const scale = plotRadius / 100;

            ctx.save();

            // Elevation rings at 0°, 30°, 60° (90° is center point)
            ctx.strokeStyle = '#30363d';
            ctx.lineWidth = 1;
            ctx.fillStyle = '#484f58';
            ctx.font = '11px sans-serif';
            ctx.textAlign = 'left';
            ctx.textBaseline = 'bottom';
            for (const elev of [0, 30, 60]) {
              const r = (90 - elev) * scale;
              ctx.beginPath();
              ctx.arc(cx, cy, r, 0, Math.PI * 2);
              ctx.stroke();
              ctx.fillText(`${elev}°`, cx + 3, cy - r - 2);
            }
            // Zenith label
            ctx.fillText('90°', cx + 3, cy - 2);

            // Cardinal direction lines
            ctx.strokeStyle = '#30363d';
            const cardinals: [string, number][] = [['N', -90], ['E', 0], ['S', 90], ['W', 180]];
            ctx.fillStyle = '#8b949e';
            ctx.font = '13px sans-serif';
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            const outerR = 90 * scale;
            const labelR = 96 * scale;
            for (const [label, angle] of cardinals) {
              const rad = (angle * Math.PI) / 180;
              ctx.beginPath();
              ctx.moveTo(cx, cy);
              ctx.lineTo(cx + outerR * Math.cos(rad), cy + outerR * Math.sin(rad));
              ctx.stroke();
              ctx.fillText(label, cx + labelR * Math.cos(rad), cy + labelR * Math.sin(rad));
            }

            ctx.restore();
          },
        },
      ],
    });

    return () => {
      chartRef.current?.destroy();
      chartRef.current = null;
    };
  }, [pass, maxElIdx, maxEl, trackColor]);

  return (
    <div className={styles.overlay} onClick={onClose}>
      <div className={styles.dialog} onClick={(e) => e.stopPropagation()}>
        <div className={styles.dialogHeader}>
          <span>{satellite} &mdash; {formatTime(pass.start)}</span>
          <button className={styles.closeButton} onClick={onClose}>&times;</button>
        </div>
        <div className={styles.dialogBody}>
          <div className={styles.polarChartContainer}>
            <canvas ref={canvasRef} />
          </div>
        </div>
      </div>
    </div>
  );
}
