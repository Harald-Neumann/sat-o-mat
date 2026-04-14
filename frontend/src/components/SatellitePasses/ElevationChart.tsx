import { useRef, useEffect, useCallback } from 'react';
import {
  Chart,
  LineController,
  LineElement,
  PointElement,
  LinearScale,
  TimeScale,
  Filler,
  Tooltip,
  Legend,
  type ChartEvent,
  type ActiveElement,
} from 'chart.js';
import zoomPlugin from 'chartjs-plugin-zoom';
import 'chartjs-adapter-moment';
import type { ApiPass } from '../../api/types';
import styles from './SatellitePasses.module.css';

Chart.register(
  LineController,
  LineElement,
  PointElement,
  LinearScale,
  TimeScale,
  Filler,
  Tooltip,
  Legend,
  zoomPlugin,
);

interface FlatPass {
  satellite: string;
  pass: ApiPass;
  color: string;
}

interface ElevationChartProps {
  passes: FlatPass[];
  rangeStart: number;
  rangeEnd: number;
  onPassClick?: (satellite: string, pass: ApiPass) => void;
  onVisibleRangeChange?: (start: number, end: number) => void;
}

const COLORS = [
  '#58a6ff', '#3fb950', '#d29922', '#f85149', '#bc8cff',
  '#79c0ff', '#56d364', '#e3b341', '#ff7b72', '#d2a8ff',
];

export function getColor(index: number): string {
  return COLORS[index % COLORS.length];
}

/** Downsample a pass to at most `max` points for chart performance. */
function downsample(pass: ApiPass, max: number) {
  const n = pass.elevation.length;
  if (n <= max) return pass;
  const step = (n - 1) / (max - 1);
  const elevation: number[] = [];
  const azimuth: number[] = [];
  for (let i = 0; i < max; i++) {
    const idx = Math.round(i * step);
    elevation.push(pass.elevation[idx]);
    azimuth.push(pass.azimuth[idx]);
  }
  return { ...pass, elevation, azimuth };
}

function buildDatasets(passes: FlatPass[]) {
  const bySat = new Map<string, { color: string; passes: ApiPass[] }>();
  for (const fp of passes) {
    let entry = bySat.get(fp.satellite);
    if (!entry) {
      entry = { color: fp.color, passes: [] };
      bySat.set(fp.satellite, entry);
    }
    entry.passes.push(fp.pass);
  }

  return Array.from(bySat.entries()).map(([satellite, { color, passes: satPasses }]) => {
    const data: { x: number; y: number }[] = [];

    for (const raw of satPasses) {
      if (data.length > 0) {
        data.push({ x: NaN, y: NaN });
      }
      const pass = downsample(raw, 120);
      const startMs = new Date(pass.start).getTime();
      const endMs = new Date(pass.end).getTime();
      const step = pass.elevation.length > 1
        ? (endMs - startMs) / (pass.elevation.length - 1)
        : 0;
      for (let i = 0; i < pass.elevation.length; i++) {
        data.push({ x: startMs + i * step, y: pass.elevation[i] });
      }
    }

    return {
      label: satellite,
      data,
      borderColor: color,
      backgroundColor: color + '18',
      borderWidth: 2,
      pointRadius: 0,
      pointHitRadius: 8,
      pointHoverRadius: 4,
      pointHoverBackgroundColor: color,
      fill: true,
      tension: 0.3,
      spanGaps: false,
    };
  });
}

export function ElevationChart({ passes, rangeStart, rangeEnd, onPassClick, onVisibleRangeChange }: ElevationChartProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const chartRef = useRef<Chart | null>(null);
  const passesRef = useRef(passes);
  passesRef.current = passes;
  const onPassClickRef = useRef(onPassClick);
  onPassClickRef.current = onPassClick;
  const onVisibleRangeChangeRef = useRef(onVisibleRangeChange);
  onVisibleRangeChangeRef.current = onVisibleRangeChange;
  const rangeRef = useRef({ start: rangeStart, end: rangeEnd });
  rangeRef.current = { start: rangeStart, end: rangeEnd };

  const emitVisibleRange = useCallback((chart: Chart) => {
    const scale = chart.scales.x;
    if (scale) onVisibleRangeChangeRef.current?.(scale.min, scale.max);
  }, []);

  // Create chart once (or when data changes). Range is set from ref.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    chartRef.current?.destroy();

    const datasets = buildDatasets(passes);

    const handleClick = (_event: ChartEvent, elements: ActiveElement[]) => {
      const cb = onPassClickRef.current;
      if (!cb || elements.length === 0) return;
      const el = elements[0];
      const datasetLabel = datasets[el.datasetIndex]?.label;
      if (!datasetLabel) return;
      const point = datasets[el.datasetIndex].data[el.index];
      if (!point || isNaN(point.x)) return;
      const clickTime = point.x;

      let best: FlatPass | null = null;
      let bestDist = Infinity;
      for (const fp of passesRef.current) {
        if (fp.satellite !== datasetLabel) continue;
        const s = new Date(fp.pass.start).getTime();
        const e = new Date(fp.pass.end).getTime();
        if (clickTime >= s && clickTime <= e) { best = fp; break; }
        const dist = Math.min(Math.abs(clickTime - s), Math.abs(clickTime - e));
        if (dist < bestDist) { bestDist = dist; best = fp; }
      }
      if (best) cb(best.satellite, best.pass);
    };

    chartRef.current = new Chart(canvas, {
      type: 'line',
      data: { datasets },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        animation: false,
        layout: { padding: { top: 4, right: 8, bottom: 0, left: 0 } },
        interaction: {
          mode: 'nearest',
          axis: 'x',
          intersect: false,
        },
        scales: {
          x: {
            type: 'time',
            min: rangeRef.current.start,
            max: rangeRef.current.end,
            time: {
              displayFormats: {
                minute: 'HH:mm',
                hour: 'HH:mm',
                day: 'MMM D',
              },
              tooltipFormat: 'MMM D, HH:mm:ss',
            },
            ticks: {
              color: '#8b949e',
              font: { size: 11, family: 'sans-serif' },
              maxRotation: 0,
            },
            grid: { color: '#21262d', lineWidth: 1 },
            border: { color: '#30363d' },
          },
          y: {
            min: 0,
            max: 90,
            ticks: {
              stepSize: 15,
              color: '#8b949e',
              font: { size: 11, family: 'sans-serif' },
              callback: (v) => `${v}°`,
            },
            grid: { color: '#21262d', lineWidth: 1 },
            border: { color: '#30363d' },
            title: {
              display: true,
              text: 'Elevation',
              color: '#484f58',
              font: { size: 11 },
            },
          },
        },
        plugins: {
          legend: {
            display: true,
            position: 'top',
            align: 'end',
            labels: {
              color: '#8b949e',
              font: { size: 11 },
              boxWidth: 12,
              boxHeight: 2,
              padding: 12,
              usePointStyle: false,
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
            displayColors: true,
            callbacks: {
              label: (ctx) => ` ${ctx.dataset.label}: ${(ctx.parsed.y ?? 0).toFixed(1)}°`,
            },
          },
          zoom: {
            pan: {
              enabled: true,
              mode: 'x',
              modifierKey: undefined,
              onPanComplete: ({ chart }) => emitVisibleRange(chart),
            },
            zoom: {
              wheel: { enabled: true },
              pinch: { enabled: true },
              mode: 'x',
              onZoomComplete: ({ chart }) => emitVisibleRange(chart),
            },
            limits: {
              x: { minRange: 60_000 },
            },
          },
        },
        onClick: handleClick,
      },
    });

    return () => {
      chartRef.current?.destroy();
      chartRef.current = null;
    };
  }, [passes]); // only rebuild when pass data changes

  // Update x-axis range in-place — no destroy/rebuild, no animation
  useEffect(() => {
    const chart = chartRef.current;
    if (!chart) return;
    const xScale = chart.options.scales?.x;
    if (!xScale) return;
    xScale.min = rangeStart;
    xScale.max = rangeEnd;
    chart.update('none');
    emitVisibleRange(chart);
  }, [rangeStart, rangeEnd, emitVisibleRange]);

  const handleDoubleClick = useCallback(() => {
    const chart = chartRef.current;
    if (!chart) return;
    chart.resetZoom();
    emitVisibleRange(chart);
  }, [emitVisibleRange]);

  return (
    <div className={styles.chartContainer} onDoubleClick={handleDoubleClick}>
      <canvas ref={canvasRef} />
    </div>
  );
}
