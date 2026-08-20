/**
 * PassRateDashboard — Monte Carlo streaming dashboard (R5.8).
 *
 * Renders one row per tracked constraint with pass-count / total shown
 * as a percentage and a visual bar, plus an overall pass-rate card
 * pinned at the top. Updates incrementally as more children arrive —
 * the purity of `computePassRate` / `computeOverallPassRate` lets this
 * be a pure render over the current descriptor list with no local state.
 *
 * Styling follows the same engineering-atelier palette as
 * PassFailGridViewer (R3.3) and the verify cards so the Monte Carlo UI
 * blends in with the rest of the app.
 */

import { useMemo } from 'react';
import { EvaluationModeBadge } from '@/components/EvaluationModeBadge';
import type { CSSProperties } from 'react';
import {
  computeOverallPassRate,
  computePassRate,
  type ChildDescriptor,
  type OverallPassRate,
  type PassRateBreakdown,
} from './passRateHelpers';

export interface PassRateDashboardProps {
  /** Iteration records — streaming-friendly; incomplete rows are OK. */
  children: ChildDescriptor[];
  /** Stable constraint ids to track. Each becomes one dashboard row. */
  constraints: string[];
  /** Optional human-readable label map (id → label). */
  labels?: Record<string, string>;
  /** Test id passthrough. */
  testId?: string;
  /** Optional className hook. */
  className?: string;
  /** Optional outer style override. */
  style?: CSSProperties;
}

const ROOT_STYLE: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 12,
  color: 'var(--on-surface)',
  fontSize: 12,
};

const CARD_STYLE: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 6,
  padding: '12px 14px',
  borderRadius: 8,
  border: '1px solid color-mix(in srgb, var(--outline-variant) 30%, transparent)',
  background: 'color-mix(in srgb, var(--surface-container) 60%, transparent)',
};

const BAR_TRACK_STYLE: CSSProperties = {
  position: 'relative',
  width: '100%',
  height: 8,
  borderRadius: 4,
  background: 'color-mix(in srgb, var(--outline-variant) 20%, transparent)',
  overflow: 'hidden',
};

function pct(rate: number): string {
  if (!Number.isFinite(rate)) return '—';
  return `${(rate * 100).toFixed(1)}%`;
}

function barColor(rate: number): string {
  // Continuous gradient: fail → warning → pass so the eye picks up
  // degradation quickly during a streaming batch. The two middle tiers
  // stay off the reserved accent wedge (hue 40-95) by using
  // --severity-warning (brass/olive, hue ~105) instead of amber.
  if (!Number.isFinite(rate)) return 'var(--text-muted)';
  if (rate >= 0.95) return 'var(--verdict-pass)';
  if (rate >= 0.75) return 'var(--severity-warning)';
  if (rate >= 0.5) return 'color-mix(in srgb, var(--severity-warning) 50%, var(--verdict-fail) 50%)';
  return 'var(--verdict-fail)';
}

interface OverallCardProps {
  overall: OverallPassRate;
  constraintCount: number;
}

function OverallCard({ overall, constraintCount }: OverallCardProps) {
  const color = barColor(overall.rate);
  const fillWidth = `${Math.max(0, Math.min(1, overall.rate)) * 100}%`;
  return (
    <div
      style={{
        ...CARD_STYLE,
        borderLeft: `3px solid ${color}`,
        gap: 8,
      }}
      role="group"
      aria-label="Overall pass-rate summary"
      data-testid="pass-rate-overall-card"
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
        <span style={{ display: 'inline-flex', alignItems: 'baseline', gap: 8 }}>
          <span style={{ fontSize: 11, opacity: 0.8, letterSpacing: 0.4, textTransform: 'uppercase' }}>
            Overall
          </span>
          {/* B10 §2.1a(d): the mode is a binding label wherever verdicts
              read — these pass-rates roll up per-child session runs, so
              the mode is trajectory by construction. */}
          <EvaluationModeBadge mode="trajectory" size="compact" testId="pass-rate-overall-mode" />
        </span>
        <span style={{ fontSize: 22, fontWeight: 600, color }} data-testid="pass-rate-overall-value">
          {pct(overall.rate)}
        </span>
      </div>
      <div style={BAR_TRACK_STYLE} aria-hidden="true">
        <div
          style={{
            width: fillWidth,
            height: '100%',
            background: color,
            transition: 'width 150ms linear',
          }}
        />
      </div>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11, opacity: 0.75 }}>
        <span data-testid="pass-rate-overall-allpass">
          {overall.allPass} / {overall.total} all-pass
        </span>
        <span>
          {overall.anyFail} with ≥1 fail · tracking {constraintCount} constraint
          {constraintCount === 1 ? '' : 's'}
        </span>
      </div>
    </div>
  );
}

interface RowProps {
  id: string;
  label: string;
  breakdown: PassRateBreakdown;
}

function ConstraintRow({ id, label, breakdown }: RowProps) {
  const color = barColor(breakdown.passRate);
  const fillWidth = `${Math.max(0, Math.min(1, breakdown.passRate)) * 100}%`;
  return (
    <div
      style={CARD_STYLE}
      role="group"
      aria-label={`Pass rate for ${label}`}
      data-testid={`pass-rate-row-${id}`}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 12 }}>
        <span style={{ fontFamily: 'ui-monospace, "JetBrains Mono", monospace', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {label}
        </span>
        <span style={{ fontSize: 14, fontWeight: 600, color, whiteSpace: 'nowrap' }} data-testid={`pass-rate-row-${id}-value`}>
          {pct(breakdown.passRate)}
        </span>
      </div>
      <div style={BAR_TRACK_STYLE} aria-hidden="true">
        <div style={{ width: fillWidth, height: '100%', background: color, transition: 'width 150ms linear' }} />
      </div>
      <div style={{ display: 'flex', gap: 10, fontSize: 11, opacity: 0.75, flexWrap: 'wrap' }}>
        <span data-testid={`pass-rate-row-${id}-pass`}>pass: {breakdown.pass}</span>
        <span>fail: {breakdown.fail}</span>
        <span>inconclusive: {breakdown.inconclusive}</span>
        <span>error: {breakdown.error}</span>
        <span>total: {breakdown.total}</span>
      </div>
    </div>
  );
}

export function PassRateDashboard(props: PassRateDashboardProps) {
  const { children, constraints, labels, testId, className, style } = props;

  const overall = useMemo(
    () => computeOverallPassRate(children, constraints),
    [children, constraints],
  );
  const rows = useMemo(
    () =>
      constraints.map((id) => ({
        id,
        label: labels?.[id] ?? id,
        breakdown: computePassRate(children, id),
      })),
    [children, constraints, labels],
  );

  if (constraints.length === 0) {
    return (
      <div
        data-testid={testId ?? 'pass-rate-dashboard-empty'}
        role="status"
        style={{
          ...ROOT_STYLE,
          padding: 24,
          border: '1px dashed color-mix(in srgb, var(--outline-variant) 35%, transparent)',
          borderRadius: 8,
          textAlign: 'center',
          fontStyle: 'italic',
          opacity: 0.7,
        }}
      >
        No constraints configured for this Monte Carlo run.
      </div>
    );
  }

  return (
    <div
      className={className}
      style={{ ...ROOT_STYLE, ...style }}
      data-testid={testId ?? 'pass-rate-dashboard'}
    >
      <OverallCard overall={overall} constraintCount={constraints.length} />
      {rows.map((row) => (
        <ConstraintRow key={row.id} id={row.id} label={row.label} breakdown={row.breakdown} />
      ))}
    </div>
  );
}
