/**
 * TimeSeriesViewer — Layer 1 EP6 primitive.
 *
 * Wraps the existing uPlot-based `UPlotChart` behind the `ResultViewer`
 * contract so panels (PlotsTab today; sweep result browsers and
 * sensitivity analyses tomorrow) never reference uPlot directly. The
 * viewer owns the timestamp-axis alignment and the series-config shape
 * so callers can hand over a plain `{ name, points }[]` payload.
 *
 * live-perf F2: at accumulated-run scale the naive union-of-timestamps +
 * per-series column build below is O(total stored samples) — paid again
 * on every update, which is the dominant cost behind the "chart freezes
 * around 500 ticks" symptom (compounding over an N-tick run into
 * O(N^2) total work). Each series is decimated to a fixed display
 * budget (`PerfBudget.MAX_RENDER_POINTS`, ADR-008) via the existing
 * LTTB downsampler *before* the union/columns are built, so this cost
 * is now O(budget) regardless of how long the run has been going —
 * `UPlotChart.setData` then also only ever receives a bounded-size
 * payload. This only changes what's handed to the chart for *display*;
 * callers still receive/store/export full-fidelity `TimePoint[]` data
 * (e.g. CSV export in `PlotsTab` reads the untouched `timeSeries` prop
 * directly, never this component's internals).
 */
import { useMemo } from 'react';
import { UPlotChart } from '../charts/UPlotChart';
import type { UPlotSeriesConfig } from '../charts/UPlotChart';
import { lttbDecimate } from '@/features/results/selectors';
import { PerfBudget } from '@/shared/perf';
import type {
  AxesConfig,
  ResultData,
  ResultViewer,
  TimeSeriesData,
} from './types';

// Feeds uPlot's canvas-based stroke opt, which cannot resolve CSS custom
// properties — cat-2 (blue) literal, keep in sync with tokens.css.
const DEFAULT_STROKE = '#2A5C8F'; // cat-2 — keep in sync with tokens.css
const DEFAULT_LINE_WIDTH = 1.5;

interface TimeSeriesChartProps {
  data: TimeSeriesData;
  axes: AxesConfig;
}

/**
 * Render helper extracted as a component so React can memoize the uPlot
 * mount across re-renders. Panels consume the viewer via
 * `timeSeriesViewer.render(data, axes)` rather than touching this
 * component directly.
 */
function TimeSeriesChart({ data, axes }: TimeSeriesChartProps) {
  const height = axes.height ?? 200;
  const width = axes.width;

  // Columnar data: uPlot wants [timestamps, series0, series1, ...] with
  // equal-length arrays. Build a sorted union of timestamps across
  // series and fill gaps with NaN so uPlot draws the natural breaks.
  //
  // F2: decimate each series to the display budget first — this is the
  // only step in this memo whose input size we control, so bounding it
  // here bounds the union-build + column-fill below (and the
  // `setData` payload UPlotChart receives) to O(budget) instead of
  // O(total accumulated points). `lttbDecimate` is a no-op below the
  // budget, so small/medium runs (hybrid-scale and under) are unaffected.
  const uplotData = useMemo(() => {
    const series = data.series;
    if (series.length === 0) return [[], []] as [number[], ...number[][]];

    const decimated = series.map((s) => lttbDecimate(s.points, PerfBudget.MAX_RENDER_POINTS));

    const timeSet = new Set<number>();
    for (const points of decimated) {
      for (const p of points) timeSet.add(p.t);
    }
    const timestamps = [...timeSet].sort((a, b) => a - b);
    const timeIndex = new Map(timestamps.map((t, i) => [t, i]));

    const columns: number[][] = [];
    for (const points of decimated) {
      const col = new Array<number>(timestamps.length).fill(NaN);
      for (const p of points) {
        const idx = timeIndex.get(p.t);
        if (idx !== undefined) col[idx] = p.v;
      }
      columns.push(col);
    }
    return [timestamps, ...columns] as [number[], ...number[][]];
  }, [data.series]);

  const uplotSeries = useMemo<UPlotSeriesConfig[]>(
    () =>
      data.series.map((s) => ({
        label: s.name,
        stroke: s.color ?? DEFAULT_STROKE,
        width: DEFAULT_LINE_WIDTH,
      })),
    [data.series],
  );

  return (
    <UPlotChart
      data={uplotData}
      height={height}
      width={width}
      series={uplotSeries}
      markers={axes.markers}
      onSelectRange={axes.onSelectRange}
    />
  );
}

/**
 * The canonical time-series viewer. Accepts any `ResultData` whose kind
 * is `'time-series'`, which narrows the payload to `TimeSeriesData` so
 * the render function can read `.series` without casts.
 */
export const timeSeriesViewer: ResultViewer<TimeSeriesData> = {
  id: 'time-series-uplot',
  kind: 'time-series',
  accepts: (data): data is TimeSeriesData => data.kind === 'time-series',
  render: (data, axes) => <TimeSeriesChart data={data} axes={axes} />,
};
