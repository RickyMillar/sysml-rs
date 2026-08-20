/**
 * UPlotChart — React wrapper around uPlot for high-performance
 * canvas-based time-series rendering per ADR-008.
 *
 * Handles mount/unmount, resize, and data updates. Accepts uPlot's
 * native columnar format: [timestamps, ...seriesValues] where each
 * entry is a number[].
 */

import { useRef, useEffect, useCallback } from 'react';
import uPlot from 'uplot';
import 'uplot/dist/uPlot.min.css';

export interface UPlotSeriesConfig {
  label: string;
  stroke?: string;
  width?: number;
  /** Render as staircase (discrete) instead of smooth line. */
  paths?: uPlot.Series.PathBuilder;
}

/** A horizontal reference line drawn behind the series (e.g. constraint bounds). */
export interface UPlotMarker {
  y: number;
  color?: string;
  dash?: [number, number];
  label?: string;
}

export interface UPlotChartProps {
  /** Columnar data: [timestamps, series0, series1, ...]. */
  data: uPlot.AlignedData;
  /** Width in CSS pixels. If omitted, fills container. */
  width?: number;
  /** Height in CSS pixels. */
  height?: number;
  /** Series configuration (index 0 is the x-axis, series start at index 1). */
  series?: UPlotSeriesConfig[];
  /** Horizontal reference lines (drawn beneath the series). */
  markers?: readonly UPlotMarker[];
  /** Additional uPlot options to merge. */
  opts?: Partial<uPlot.Options>;
  /**
   * Fires with the drag-selected x-range in axis units (W3-E zoom
   * re-query). uPlot's own drag-zoom scale change still happens; the
   * caller may replace the data with a higher-fidelity window.
   */
  onSelectRange?: (fromX: number, toX: number) => void;
}

export function UPlotChart({
  data,
  width,
  height = 200,
  series,
  markers,
  opts,
  onSelectRange,
}: UPlotChartProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<uPlot | null>(null);
  // Ref-carried so the setSelect hook (captured once at chart create)
  // always calls the latest handler without recreating the chart.
  const onSelectRangeRef = useRef(onSelectRange);
  onSelectRangeRef.current = onSelectRange;

  // Build uPlot options from props
  const buildOptions = useCallback(
    (w: number): uPlot.Options => {
      const seriesOpts: uPlot.Series[] = [
        // x-axis (timestamps)
        {},
      ];

      if (series) {
        for (const s of series) {
          seriesOpts.push({
            label: s.label,
            // uPlot draws to a 2D canvas context and cannot resolve CSS
            // custom properties — cat-2 (blue), keep in sync with tokens.css
            stroke: s.stroke ?? '#2A5C8F', // cat-2 — keep in sync with tokens.css
            width: s.width ?? 1.5,
            paths: s.paths,
          });
        }
      } else if (data.length > 1) {
        // Auto-generate series config for each data column.
        // uPlot draws to a 2D canvas context and cannot resolve CSS custom
        // properties, so these are the cat ramp literals — keep in sync
        // with the --nb-cat-* primitives in tokens.css.
        const COLORS = [
          '#1D6E62', '#2A5C8F', '#5B4A9E', '#8E3A6B',
          '#2F6B3C', '#4A5F72', '#74438A', '#2C6480',
        ]; // cat-1..8 — keep in sync with tokens.css
        for (let i = 1; i < data.length; i++) {
          seriesOpts.push({
            label: `Series ${i}`,
            stroke: COLORS[(i - 1) % COLORS.length],
            width: 1.5,
          });
        }
      }

      // Horizontal reference lines drawn beneath the series via a
      // `draw` hook. uPlot exposes `valToPos(val, 'y', true)` which
      // gives canvas-space pixel y for the given data value; we draw
      // a thin dashed (or solid) stroke across the plot area for each
      // marker. Drawing in `draw` (not `drawSeries`) ensures the lines
      // land above the grid but under the data traces, which matches
      // the "reference" visual weight we want.
      const markerHook =
        markers && markers.length > 0
          ? (u: uPlot) => {
              const ctx = u.ctx;
              const { left, top, width: plotW, height: plotH } = u.bbox;
              ctx.save();
              ctx.beginPath();
              ctx.rect(left, top, plotW, plotH);
              ctx.clip();
              for (const m of markers) {
                const yPx = u.valToPos(m.y, 'y', true);
                if (!Number.isFinite(yPx)) continue;
                if (yPx < top - 0.5 || yPx > top + plotH + 0.5) continue;
                ctx.beginPath();
                // accent-fg (dark) — mirrors --chart-annotation in tokens.css;
                // canvas can't resolve var(), keep in sync
                ctx.strokeStyle = m.color ?? '#E5B269';
                ctx.lineWidth = 1;
                if (m.dash) ctx.setLineDash([m.dash[0], m.dash[1]]);
                else ctx.setLineDash([]);
                ctx.moveTo(left, yPx);
                ctx.lineTo(left + plotW, yPx);
                ctx.stroke();
              }
              ctx.restore();
            }
          : null;

      const baseOpts: uPlot.Options = {
        width: w,
        height,
        series: seriesOpts,
        cursor: {
          drag: { x: true, y: false },
        },
        scales: {
          x: { time: false },
        },
        axes: [
          {
            // X is simulated time in milliseconds (snapshot `time_ms`). Label it
            // so the axis isn't mistaken for a step/tick index — with dt=1ms the
            // values coincide with the step count, which reads as "1,2,3,…".
            label: 'time (ms)',
            stroke: '#666',
            grid: { stroke: '#333', width: 0.5 },
            font: '10px monospace',
            labelFont: '10px monospace',
          },
          {
            stroke: '#666',
            grid: { stroke: '#333', width: 0.5 },
            font: '10px monospace',
          },
        ],
        ...opts,
      };

      if (markerHook) {
        baseOpts.hooks = {
          ...(baseOpts.hooks ?? {}),
          draw: [...(baseOpts.hooks?.draw ?? []), markerHook],
        };
      }

      // W3-E: publish drag-selected x-ranges (axis units) so callers can
      // re-query the backend's decimated window. Ref-carried handler —
      // never triggers a chart rebuild.
      baseOpts.hooks = {
        ...(baseOpts.hooks ?? {}),
        setSelect: [
          ...(baseOpts.hooks?.setSelect ?? []),
          (u: uPlot) => {
            const handler = onSelectRangeRef.current;
            if (!handler || u.select.width < 5) return;
            const fromX = u.posToVal(u.select.left, 'x');
            const toX = u.posToVal(u.select.left + u.select.width, 'x');
            if (Number.isFinite(fromX) && Number.isFinite(toX)) handler(fromX, toX);
          },
        ],
      };

      return baseOpts;
    },
    [data.length, height, series, markers, opts],
  );

  // Mount / rebuild chart
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const w = width ?? (el.clientWidth || 400);
    const options = buildOptions(w);

    // Destroy previous instance
    if (chartRef.current) {
      chartRef.current.destroy();
      chartRef.current = null;
    }

    chartRef.current = new uPlot(options, data, el);

    return () => {
      if (chartRef.current) {
        chartRef.current.destroy();
        chartRef.current = null;
      }
    };
    // Rebuild when series config or height change
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [buildOptions]);

  // Update data without rebuilding the chart
  useEffect(() => {
    if (chartRef.current && data.length > 0 && data[0].length > 0) {
      chartRef.current.setData(data);
    }
  }, [data]);

  // Handle resize
  useEffect(() => {
    const el = containerRef.current;
    if (!el || !chartRef.current) return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const newWidth = entry.contentRect.width;
        if (newWidth > 0 && chartRef.current) {
          chartRef.current.setSize({ width: newWidth, height });
        }
      }
    });

    observer.observe(el);
    return () => observer.disconnect();
  }, [height]);

  return <div ref={containerRef} style={{ width: width ?? '100%' }} />;
}
