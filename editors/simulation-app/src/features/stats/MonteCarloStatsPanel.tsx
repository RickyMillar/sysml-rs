/**
 * MonteCarloStatsPanel — R7.2 import-only stats strip for MC viewers.
 *
 * Mounts a `StatsOverlay` per outcome metric above the existing
 * `MonteCarloHistogramViewer` output. This component does NOT modify
 * the histogram viewer — it composes alongside so shells can layer
 * stats without touching R5.7 wiring.
 *
 * Usage (shell side, non-binding):
 *   <MonteCarloStatsPanel
 *     children={mcChildren}
 *     outcomes={[
 *       { id: 'trip_time', extract: c => num(c.metrics?.trip_time) },
 *     ]}
 *   />
 *   <MonteCarloHistogramViewer children={...} outcomes={...} />
 */

import { useMemo } from 'react';
import type { CSSProperties } from 'react';
import type { ChildDescriptor } from '../../workflows/analyze/montecarlo/passRateHelpers';
import { StatsOverlay } from './StatsOverlay';

/** One outcome to summarise — mirrors MC viewer's outcome shape. */
export interface MonteCarloStatsOutcome {
  id: string;
  label?: string;
  unit?: string;
  extract: (child: ChildDescriptor) => number | null | undefined;
}

export interface MonteCarloStatsPanelProps {
  /** Iteration records from the batch poller. */
  children: ChildDescriptor[];
  /** One or more outcomes to summarise. */
  outcomes: MonteCarloStatsOutcome[];
  /** Show the Q-Q plot per outcome. Defaults to true. */
  showQQ?: boolean;
  /** RNG override for bootstrap CIs — deterministic fixture by default. */
  rng?: () => number;
  className?: string;
  testId?: string;
}

const ROOT_STYLE: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 10,
};

export function MonteCarloStatsPanel(props: MonteCarloStatsPanelProps) {
  const { children, outcomes, showQQ = true, rng, className, testId } = props;

  if (outcomes.length === 0) {
    return (
      <div
        className={className}
        data-testid={testId ? `${testId}-empty` : 'mc-stats-panel-empty'}
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
        No outcome metrics configured.
      </div>
    );
  }

  return (
    <div
      className={className}
      style={ROOT_STYLE}
      data-testid={testId ?? 'mc-stats-panel'}
      aria-label="Monte Carlo statistical summary"
    >
      {outcomes.map((outcome) => (
        <OutcomeStats
          key={outcome.id}
          outcome={outcome}
          children={children}
          showQQ={showQQ}
          rng={rng}
        />
      ))}
    </div>
  );
}

interface OutcomeStatsProps {
  outcome: MonteCarloStatsOutcome;
  children: ChildDescriptor[];
  showQQ: boolean;
  rng?: () => number;
}

function OutcomeStats({ outcome, children, showQQ, rng }: OutcomeStatsProps) {
  const values = useMemo(() => {
    const out: number[] = [];
    for (const c of children) {
      const v = outcome.extract(c);
      if (v == null || !Number.isFinite(v)) continue;
      out.push(v as number);
    }
    return out;
  }, [children, outcome]);

  return (
    <StatsOverlay
      label={outcome.label ?? outcome.id}
      values={values}
      unit={outcome.unit}
      showQQ={showQQ}
      rng={rng}
      testId={`mc-stats-overlay-${outcome.id}`}
    />
  );
}
