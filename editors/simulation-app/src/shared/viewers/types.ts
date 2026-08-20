/**
 * ResultViewer — Layer 1 primitive (extensibility plan EP6).
 *
 * Each viewer is a self-describing renderer for a specific shape of
 * result data (time series, histogram, timeline, table, heatmap,
 * parallel-coords, ...). Today only `time-series` ships; new shapes slot
 * in by adding a `ResultViewer<T>` entry to the kit.
 *
 * Panels consume viewers via `.render(data, axes)` — they never touch
 * uPlot (or any chart library) directly. This keeps the chart swap
 * discussion localized to the kit, not scattered across cards.
 */
import type { ReactNode } from 'react';
import type { TimePoint } from '../../features/sessions/types';
import type { Verdict } from '../../engine/types';

/** Supported viewer kinds. Extend as new primitives land. */
export type ResultViewerKind =
  | 'time-series'
  | 'histogram'
  | 'timeline'
  | 'table'
  | 'heatmap'
  | 'parallel-coords'
  | 'pass-fail-grid'
  // R5.3 — sweep viewer family. Each consumes a `ChildDescriptor[]` from the
  // batch poller and renders a sweep-specific lens over the result grid.
  | 'sweep-table'
  | 'sweep-tornado'
  | 'sweep-parallel-coords'
  | 'sweep-heatmap'
  // R5.7 — Monte Carlo histogram viewer (per-output distribution + stats).
  | 'mc-histogram'
  // R5.11 — trade study table with Pareto-front overlay and
  // "promote to compare" multi-select.
  | 'trade-table'
  // R6.2 — traceability matrix viewer. Rows = requirements / source
  // elements, columns = linked targets (parts, constraints, cases),
  // cells = 4-valued verdict badges. Consumes `TraceMatrix` (see
  // engine/types.ts) derived from `sysml.trace_matrix`.
  | 'trace-matrix';

/**
 * Loose tag describing what `data` a viewer can consume. Today we only
 * need the kind discriminator; later entries (series count, axis shape,
 * dtype, etc.) can live on specific subtypes.
 */
export interface ResultData {
  kind: ResultViewerKind;
}

/** Horizontal reference line drawn across the y-axis. */
export interface MarkerLine {
  /** Y value where the line is drawn. */
  y: number;
  /** Stroke colour (defaults to an amber reference tone when omitted). */
  color?: string;
  /** Dash pattern in `[on, off]` pixels. Omit for solid. */
  dash?: [number, number];
  /** Human-readable label (tooltip / ARIA hint, not rendered on the line). */
  label?: string;
}

/** Generic axis hint surface consumed by `render`. */
export interface AxesConfig {
  /** Desired rendered height in CSS pixels. */
  height?: number;
  /** Desired rendered width in CSS pixels. Omit to fill container. */
  width?: number;
  /** X-axis label (optional). */
  xLabel?: string;
  /** Y-axis label (optional). */
  yLabel?: string;
  /** Horizontal reference lines drawn across the y-axis (e.g. constraint
   *  bounds). Empty / omitted leaves the chart unchanged. */
  markers?: readonly MarkerLine[];
  /**
   * Drag-zoom callback (ninebar Phase 3 W3-E): fires with the selected
   * x-range (in the axis' native units — ms for time series) when the
   * user drag-selects. Callers re-query the backend's decimated window
   * (`sessions.timeseries_decimated`) rather than zooming into
   * already-decimated display data.
   */
  onSelectRange?: (fromX: number, toX: number) => void;
}

/**
 * One series in a time-series viewer payload.
 *
 * `points` is the per-trace sample list; `color` is an optional override
 * for the auto-palette (most callers rely on the palette, some — like
 * PlotsTab's domain-aware colouring — supply explicit colors).
 */
export interface TimeSeriesSeries {
  name: string;
  points: TimePoint[];
  color?: string;
}

/**
 * Payload accepted by `timeSeriesViewer`. Keeping the columnar
 * transformation inside the viewer (not the caller) means panels never
 * need to know uPlot's AlignedData quirks.
 */
export interface TimeSeriesData extends ResultData {
  kind: 'time-series';
  series: TimeSeriesSeries[];
}

/**
 * Payload accepted by `passFailGridViewer`.
 *
 * `verdicts` is a flat list — the viewer derives the row (verification
 * case) and column (individual requirement check) taxonomy from each
 * verdict's `metadata.case_name` / `metadata.requirement_id`. Keeping the
 * payload flat means producers (the Verify runner and later aggregators)
 * don't need to pre-group; the matrix is purely a rendering choice.
 *
 * `onVerdictSelect` is the drill callback Agent Q wires on top of —
 * clicking any cell fires it with the underlying `Verdict` so the parent
 * can open RunWorkflow at `verdict.evidence.tick`.
 */
export interface PassFailGridData extends ResultData {
  kind: 'pass-fail-grid';
  verdicts: Verdict[];
  /** Click handler. Optional — when absent cells are non-interactive. */
  onVerdictSelect?: (verdict: Verdict) => void;
}

/**
 * Declarative viewer description. The kit exposes one instance per
 * supported kind; registries and panels look up a viewer by `kind` and
 * invoke `render`.
 */
export interface ResultViewer<T extends ResultData = ResultData> {
  id: string;
  kind: ResultViewerKind;
  /** Quick sanity check — does this viewer accept this shape of data? */
  accepts: (data: ResultData) => data is T;
  /**
   * Produce the ReactNode for the viewer. Must be a pure render — no
   * side-effects or app-store reads — so panels can memoize inputs.
   */
  render: (data: T, axes: AxesConfig) => ReactNode;
}
