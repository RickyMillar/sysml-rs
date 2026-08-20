/**
 * AttributeDetail — full time-series chart + summary stats for a
 * focused attribute.
 *
 * Round 2 Task #143. Reuses the existing `timeSeriesViewer`
 * primitive (`shared/viewers/TimeSeriesViewer`) so the canvas
 * matches Plots + Trade Study viewers exactly. `Sparkline`
 * covers the low-sample fallback (<3 points, or zero-sample new
 * sessions).
 */
import { useMemo } from 'react';
import { Sparkline } from '@/features/variables/Sparkline';
import { formatVariableValue } from '@/features/variables/VariableTree';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { timeSeriesViewer } from '@/shared/viewers/TimeSeriesViewer';
import type { MarkerLine } from '@/shared/viewers/types';
import type { AttributeTreeNode } from '../types';
import { DetailMeta, DetailShell } from './common';
import { useSessionStore } from '../../store';
import { computeAttributeStats } from './stats';

/**
 * Bound-marker shape read off the AttributeTreeNode (R3.3 — backend-
 * extracted in `crates/tooling/sysml-service/src/bounds.rs`). Mirrors
 * `AttributeTreeNode['bounds']` element shape with `Required` typing
 * so chart helpers can rely on every field being present once the
 * walker has emitted a marker.
 */
type BoundMarker = NonNullable<AttributeTreeNode['bounds']>[number];

export function AttributeDetail({
  node,
  testIdPrefix,
}: {
  node: AttributeTreeNode;
  testIdPrefix: string;
}) {
  // Resolve the attribute's backend variable name the same way
  // mergeLiveState does — fully-qualified ownerPath.name first, then
  // the bare name.
  const tsRevision = useTimeSeriesStore((s) => s.revision);
  const draftOverride = useSessionStore(
    (s) => s.draftOverrides[varName(node)] ?? s.draftOverrides[node.name],
  );

  const points = useMemo(() => {
    const map = useTimeSeriesStore.getState().getTimeSeries();
    const full = varName(node);
    return map[full] ?? map[node.name] ?? [];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tsRevision, node.id, node.name, node.ownerPath]);

  const stats = useMemo(() => computeAttributeStats(points), [points]);

  // R3.3: bound markers are projected onto the AttributeTreeNode by
  // the backend (`bounds.rs::extract_bounds_by_attribute`). The
  // backend resolves attribute references to ElementId, so two
  // circuits sharing a `temperature` short name get separate
  // per-instance bound lists — no FE AST walking, no name-collision
  // risk. Empty array when no constraints reference this attribute.
  const bounds = useMemo<readonly BoundMarker[]>(
    () => node.bounds ?? [],
    [node.bounds],
  );
  const markers = useMemo<MarkerLine[]>(
    () => bounds.map(boundToMarker),
    [bounds],
  );

  return (
    <DetailShell testIdPrefix={testIdPrefix} suffix="attribute">
      <DetailMeta
        node={node}
        extra={
          typeof node.lastChangedTick === 'number'
            ? `last change @ tick ${node.lastChangedTick}`
            : undefined
        }
      />
      <div className="flex items-center gap-3">
        <div
          data-testid={`${testIdPrefix}-attribute-value`}
          className="mono-text"
          style={{
            fontSize: 22,
            color: 'var(--text-primary)',
            fontWeight: 600,
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {node.value === undefined
            ? '—'
            : formatVariableValue(node.value ?? null, node.unit)}
        </div>
        {draftOverride !== undefined && (
          <span
            data-testid={`${testIdPrefix}-attribute-draft`}
            className="mono-text"
            title="Override queued for the next tick"
            style={{
              fontSize: 10,
              color: 'var(--severity-warning)',
              border: '1px solid var(--severity-warning)',
              padding: '2px 6px',
              borderRadius: 10,
              fontWeight: 600,
            }}
          >
            DRAFT → {draftOverride}
          </span>
        )}
      </div>

      {/* Chart or low-sample fallback. */}
      {points.length >= 3 ? (
        <div
          data-testid={`${testIdPrefix}-attribute-chart`}
          style={{
            border: '1px solid var(--border-default)',
            borderRadius: 4,
            overflow: 'hidden',
          }}
        >
          {timeSeriesViewer.render(
            {
              kind: 'time-series',
              series: [
                {
                  name: node.name,
                  points: points as Array<{ t: number; v: number }>,
                },
              ],
            },
            {
              height: 140,
              xLabel: 't',
              yLabel: node.unit ?? '',
              markers,
            },
          )}
          {bounds.length > 0 && (
            <BoundsLegend
              bounds={bounds}
              unit={node.unit}
              testIdPrefix={testIdPrefix}
            />
          )}
        </div>
      ) : points.length > 0 ? (
        <div
          data-testid={`${testIdPrefix}-attribute-chart-fallback`}
          style={{
            fontSize: 11,
            color: 'var(--text-muted)',
            border: '1px dashed var(--border-default)',
            padding: 8,
            borderRadius: 4,
          }}
        >
          Only {points.length} sample
          {points.length === 1 ? '' : 's'} so far — chart lights up at 3+.
          <div style={{ marginTop: 6 }}>
            <Sparkline
              samples={points.map((p) => p.v)}
              width={140}
              height={24}
              color="var(--chart-series-1)"
              ariaLabel={`${node.name} sparkline`}
            />
          </div>
        </div>
      ) : (
        <div
          data-testid={`${testIdPrefix}-attribute-chart-empty`}
          style={{
            fontSize: 11,
            color: 'var(--text-muted)',
            border: '1px dashed var(--border-default)',
            padding: 8,
            borderRadius: 4,
          }}
        >
          No samples yet. Run the session to start recording.
        </div>
      )}

      {/* Stats strip. */}
      {stats && (
        <div
          data-testid={`${testIdPrefix}-attribute-stats`}
          className="mono-text"
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(5, 1fr)',
            gap: 6,
            fontSize: 10,
            color: 'var(--text-muted)',
            borderTop: '1px solid var(--border-default)',
            paddingTop: 6,
          }}
        >
          <StatCell label="min" value={formatStat(stats.min, node.unit)} />
          <StatCell label="max" value={formatStat(stats.max, node.unit)} />
          <StatCell label="mean" value={formatStat(stats.mean, node.unit)} />
          <StatCell label="σ" value={formatStat(stats.stddev, node.unit)} />
          <StatCell label="n" value={String(stats.count)} />
        </div>
      )}
    </DetailShell>
  );
}

// ─── Bounds overlay helpers ──────────────────────────────────────────

/**
 * Distinct hues for bound markers. Upper / lower / target each get
 * their own hue so a constraint like `0 <= x <= 100` renders with
 * distinct top and bottom reference lines that the eye can separate.
 *
 * Token sweep judgment call: amber is the reserved accent wedge
 * (selection/active/primacy per tokens.css), so 'target' — despite
 * being amber (#f59e0b) historically — is remapped to chart-series-6
 * (slate) rather than --accent/--accent-fg. upper/lower map to the
 * closest-hue chart series (magenta, blue) instead of collapsing to
 * one token, preserving the three-way visual separation the original
 * comment calls for.
 */
const BOUND_COLORS: Record<BoundMarker['kind'], string> = {
  upper: 'var(--chart-series-4)',
  lower: 'var(--chart-series-2)',
  target: 'var(--chart-series-6)',
};

function boundToMarker(b: BoundMarker): MarkerLine {
  return {
    y: b.y,
    color: BOUND_COLORS[b.kind],
    // Dashed for upper/lower (soft reference), solid for target
    // (hard equality). The eye reads solid as "should be *at* this",
    // dashed as "should stay *away from* this".
    dash: b.kind === 'target' ? undefined : [4, 3],
    label: `${b.kind} ${b.operator} ${b.y}`,
  };
}

function BoundsLegend({
  bounds,
  unit,
  testIdPrefix,
}: {
  bounds: readonly BoundMarker[];
  unit: string | undefined;
  testIdPrefix: string;
}) {
  return (
    <div
      data-testid={`${testIdPrefix}-attribute-bounds`}
      className="mono-text"
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        gap: 8,
        padding: '4px 8px',
        borderTop: '1px solid var(--border-default)',
        background: 'var(--surface-panel)',
        fontSize: 9,
      }}
    >
      {bounds.map((b, i) => (
        <span
          key={`${b.y}-${b.kind}-${b.constraintName}-${i}`}
          data-testid={`${testIdPrefix}-attribute-bound-${b.kind}-${b.y}`}
          style={{
            color: 'var(--text-secondary)',
            display: 'inline-flex',
            alignItems: 'center',
            gap: 4,
          }}
          title={`${b.constraintName}: ${b.operator} ${b.y}`}
        >
          <span
            aria-hidden
            style={{
              display: 'inline-block',
              width: 10,
              height: 2,
              background: BOUND_COLORS[b.kind],
              borderRadius: 1,
            }}
          />
          {b.operator} {formatStat(b.y, unit)}
        </span>
      ))}
    </div>
  );
}

function StatCell({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5" style={{ minWidth: 0 }}>
      <span
        style={{
          fontSize: 9,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
        }}
      >
        {label}
      </span>
      <span
        className="truncate"
        style={{
          color: 'var(--text-primary)',
          fontSize: 11,
          fontVariantNumeric: 'tabular-nums',
        }}
      >
        {value}
      </span>
    </div>
  );
}

function varName(node: AttributeTreeNode): string {
  return node.ownerPath ? `${node.ownerPath}.${node.name}` : node.name;
}

function formatStat(value: number, unit: string | undefined): string {
  if (!Number.isFinite(value)) return '—';
  const abs = Math.abs(value);
  const pretty =
    abs === 0
      ? '0'
      : abs >= 1e6 || abs < 1e-3
        ? value.toExponential(3)
        : (() => {
            const p = value.toPrecision(5);
            return p.includes('.') ? p.replace(/\.?0+$/, '') : p;
          })();
  return unit ? `${pretty} ${unit}` : pretty;
}
