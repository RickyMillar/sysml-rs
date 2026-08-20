/**
 * SweepStatsPanel — R7.2 import-only stats overlay for Sweep viewers.
 *
 * Consumes the same `ChildDescriptor[]` shape as the R5.2/R5.3 sweep
 * helpers and surfaces per-(parameter × metric) stats. For a 1-D sweep
 * this collapses to a single row; for a 2-D sweep a matrix is emitted
 * where cells show a StatsOverlay of the metric values for each (x, y)
 * bucket.
 *
 * Design tradeoffs:
 *   - Import-only: does not modify Sweep viewer files; shells mount it
 *     alongside.
 *   - For 2-D: only (parameterX, parameterY) combinations whose
 *     occupancy passes `minCellSamples` (default 2) render a full
 *     overlay; sparser cells show a neutral placeholder so the matrix
 *     stays honest.
 *   - Metric extraction is caller-supplied to match the sweep helpers'
 *     `metricExtractor` contract.
 */

import { useMemo } from 'react';
import type { CSSProperties } from 'react';
import {
  collectParamNames,
  toNumber,
  type ChildDescriptor,
} from '../../shared/viewers/sweepViewerHelpers';
import { StatsOverlay } from './StatsOverlay';

/** Outcome metric for a sweep — similar to the MC outcome shape. */
export interface SweepStatsMetric {
  id: string;
  label?: string;
  unit?: string;
  extract: (child: ChildDescriptor) => number;
}

export interface SweepStatsPanelProps {
  /** Sweep batch iteration records. */
  children: ChildDescriptor[];
  /** One or more outcome metrics to summarise per parameter bucket. */
  metrics: SweepStatsMetric[];
  /**
   * Names of swept parameters. When omitted, every parameter observed
   * in `children[*].params` is analysed. Supply a shorter list to
   * constrain the panel.
   */
  parameters?: string[];
  /**
   * When true AND exactly two parameters are present, render as a 2-D
   * stats matrix. Defaults to auto-detect (`parameters.length === 2`).
   */
  matrix?: boolean;
  /** Minimum samples required to render a full overlay per cell. */
  minCellSamples?: number;
  /** Q-Q plot toggle — forwarded to every overlay. Default false to keep the matrix compact. */
  showQQ?: boolean;
  /** RNG override for bootstrap CIs. */
  rng?: () => number;
  className?: string;
  testId?: string;
}

const ROOT_STYLE: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 12,
};

const SECTION_HEADER_STYLE: CSSProperties = {
  fontSize: 12,
  fontWeight: 600,
  margin: 0,
  opacity: 0.85,
};

/** Bucket children by rounded parameter value. */
function bucketByParam(
  children: ChildDescriptor[],
  paramName: string,
): Map<number, ChildDescriptor[]> {
  const out = new Map<number, ChildDescriptor[]>();
  for (const c of children) {
    const raw = c.params ? c.params[paramName] : undefined;
    const n = toNumber(raw);
    if (!Number.isFinite(n)) continue;
    let bag = out.get(n);
    if (!bag) {
      bag = [];
      out.set(n, bag);
    }
    bag.push(c);
  }
  return out;
}

/** Bucket children by a pair of rounded parameter values. */
function bucketByPair(
  children: ChildDescriptor[],
  xParam: string,
  yParam: string,
): Map<string, { x: number; y: number; kids: ChildDescriptor[] }> {
  const out = new Map<string, { x: number; y: number; kids: ChildDescriptor[] }>();
  for (const c of children) {
    const xv = toNumber(c.params ? c.params[xParam] : undefined);
    const yv = toNumber(c.params ? c.params[yParam] : undefined);
    if (!Number.isFinite(xv) || !Number.isFinite(yv)) continue;
    const key = `${xv}__${yv}`;
    let cell = out.get(key);
    if (!cell) {
      cell = { x: xv, y: yv, kids: [] };
      out.set(key, cell);
    }
    cell.kids.push(c);
  }
  return out;
}

function extractValues(
  kids: ChildDescriptor[],
  metric: SweepStatsMetric,
): number[] {
  const out: number[] = [];
  for (const c of kids) {
    const v = metric.extract(c);
    if (Number.isFinite(v)) out.push(v);
  }
  return out;
}

export function SweepStatsPanel(props: SweepStatsPanelProps) {
  const {
    children,
    metrics,
    parameters,
    matrix,
    minCellSamples = 2,
    showQQ = false,
    rng,
    className,
    testId,
  } = props;

  const paramNames = useMemo(() => {
    if (parameters && parameters.length > 0) return parameters;
    return collectParamNames(children);
  }, [children, parameters]);

  const isMatrix = matrix ?? paramNames.length === 2;

  if (metrics.length === 0 || paramNames.length === 0) {
    return (
      <div
        className={className}
        data-testid={testId ? `${testId}-empty` : 'sweep-stats-panel-empty'}
        role="status"
        style={{
          padding: 12,
          borderRadius: 6,
          border: '1px dashed color-mix(in srgb, var(--outline-variant) 30%, transparent)',
          fontStyle: 'italic',
          opacity: 0.7,
          fontSize: 12,
        }}
      >
        No swept parameters or metrics configured.
      </div>
    );
  }

  return (
    <div
      className={className}
      style={ROOT_STYLE}
      data-testid={testId ?? 'sweep-stats-panel'}
      aria-label="Sweep statistical summary"
    >
      {isMatrix && paramNames.length >= 2 ? (
        <MatrixLayout
          children={children}
          metrics={metrics}
          xParam={paramNames[0]}
          yParam={paramNames[1]}
          minCellSamples={minCellSamples}
          showQQ={showQQ}
          rng={rng}
        />
      ) : (
        <RowLayout
          children={children}
          metrics={metrics}
          params={paramNames}
          showQQ={showQQ}
          rng={rng}
        />
      )}
    </div>
  );
}

interface RowLayoutProps {
  children: ChildDescriptor[];
  metrics: SweepStatsMetric[];
  params: string[];
  showQQ: boolean;
  rng?: () => number;
}

function RowLayout({ children, metrics, params, showQQ, rng }: RowLayoutProps) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }} data-testid="sweep-stats-rows">
      {params.map((param) => {
        const buckets = bucketByParam(children, param);
        return (
          <section key={param} data-testid={`sweep-stats-param-${param}`}>
            <h6 style={SECTION_HEADER_STYLE}>parameter: {param}</h6>
            <div
              style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
                gap: 10,
                marginTop: 6,
              }}
            >
              {metrics.map((metric) => {
                // For the row layout we aggregate ALL children under the
                // parameter (the parameter identifies the sweep axis; the
                // stats describe the metric's distribution across it).
                const values: number[] = [];
                for (const bag of buckets.values()) {
                  values.push(...extractValues(bag, metric));
                }
                return (
                  <StatsOverlay
                    key={metric.id}
                    values={values}
                    label={`${metric.label ?? metric.id}`}
                    unit={metric.unit}
                    showQQ={showQQ}
                    rng={rng}
                    testId={`sweep-stats-overlay-${param}-${metric.id}`}
                  />
                );
              })}
            </div>
          </section>
        );
      })}
    </div>
  );
}

interface MatrixLayoutProps {
  children: ChildDescriptor[];
  metrics: SweepStatsMetric[];
  xParam: string;
  yParam: string;
  minCellSamples: number;
  showQQ: boolean;
  rng?: () => number;
}

function MatrixLayout(props: MatrixLayoutProps) {
  const { children, metrics, xParam, yParam, minCellSamples, showQQ, rng } = props;
  const pairs = useMemo(() => bucketByPair(children, xParam, yParam), [children, xParam, yParam]);
  const xs = useMemo(() => {
    const s = new Set<number>();
    for (const { x } of pairs.values()) s.add(x);
    return [...s].sort((a, b) => a - b);
  }, [pairs]);
  const ys = useMemo(() => {
    const s = new Set<number>();
    for (const { y } of pairs.values()) s.add(y);
    return [...s].sort((a, b) => a - b);
  }, [pairs]);

  if (xs.length === 0 || ys.length === 0) {
    return (
      <div
        data-testid="sweep-stats-matrix-empty"
        role="status"
        style={{
          padding: 12,
          borderRadius: 6,
          border: '1px dashed color-mix(in srgb, var(--outline-variant) 30%, transparent)',
          fontStyle: 'italic',
          opacity: 0.7,
          fontSize: 12,
        }}
      >
        No 2-D sweep data yet.
      </div>
    );
  }

  return (
    <div
      data-testid="sweep-stats-matrix"
      style={{ display: 'flex', flexDirection: 'column', gap: 14 }}
    >
      <h6 style={SECTION_HEADER_STYLE}>
        parameters: {xParam} × {yParam}
      </h6>
      {metrics.map((metric) => (
        <section key={metric.id} data-testid={`sweep-stats-matrix-${metric.id}`}>
          <p style={{ fontSize: 11, margin: '0 0 4px 0', opacity: 0.7 }}>
            metric: {metric.label ?? metric.id}
            {metric.unit ? ` (${metric.unit})` : ''}
          </p>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: `80px repeat(${xs.length}, minmax(180px, 1fr))`,
              gap: 4,
              alignItems: 'start',
              fontSize: 11,
            }}
          >
            <div />
            {xs.map((xv) => (
              <div
                key={`hx-${xv}`}
                style={{ fontVariantNumeric: 'tabular-nums', opacity: 0.75, padding: '4px 6px' }}
                data-testid={`sweep-stats-matrix-col-${xv}`}
              >
                {xParam}={xv}
              </div>
            ))}
            {ys.map((yv) => (
              <RowFor
                key={`row-${yv}`}
                yv={yv}
                yParam={yParam}
                xs={xs}
                pairs={pairs}
                metric={metric}
                minCellSamples={minCellSamples}
                showQQ={showQQ}
                rng={rng}
              />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

interface RowForProps {
  yv: number;
  yParam: string;
  xs: number[];
  pairs: Map<string, { x: number; y: number; kids: ChildDescriptor[] }>;
  metric: SweepStatsMetric;
  minCellSamples: number;
  showQQ: boolean;
  rng?: () => number;
}

function RowFor(props: RowForProps) {
  const { yv, yParam, xs, pairs, metric, minCellSamples, showQQ, rng } = props;
  return (
    <>
      <div
        style={{ fontVariantNumeric: 'tabular-nums', opacity: 0.75, padding: '4px 6px' }}
        data-testid={`sweep-stats-matrix-row-${yv}`}
      >
        {yParam}={yv}
      </div>
      {xs.map((xv) => {
        const cell = pairs.get(`${xv}__${yv}`);
        const kids = cell?.kids ?? [];
        const values = extractValues(kids, metric);
        if (values.length < minCellSamples) {
          return (
            <div
              key={`cell-${xv}-${yv}`}
              data-testid={`sweep-stats-matrix-cell-${metric.id}-${xv}-${yv}-sparse`}
              style={{
                padding: 10,
                borderRadius: 6,
                border: '1px dashed color-mix(in srgb, var(--outline-variant) 25%, transparent)',
                fontStyle: 'italic',
                fontSize: 10,
                opacity: 0.6,
              }}
            >
              N={values.length} (sparse)
            </div>
          );
        }
        return (
          <StatsOverlay
            key={`cell-${xv}-${yv}`}
            values={values}
            label={`${metric.label ?? metric.id} @ (${xv}, ${yv})`}
            unit={metric.unit}
            showQQ={showQQ}
            rng={rng}
            testId={`sweep-stats-matrix-cell-${metric.id}-${xv}-${yv}`}
          />
        );
      })}
    </>
  );
}
