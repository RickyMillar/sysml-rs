/**
 * DivergenceGutter — thin vertical heat-band showing per-tick divergence
 * across N sessions for one variable.
 *
 * Pure component: takes `{ samples }` where `samples[s][t]` is the value
 * of session `s` at tick `t`. Computes per-tick normalised divergence
 * and paints bands. Clicking a band calls `onScrubTo(tick)` so the
 * surrounding chart's shared playhead can snap to that tick.
 *
 * The heavy math lives in `selectors.ts` so it's testable on its own —
 * this component is just thin presentation.
 */

import { useMemo } from 'react';
import type { CSSProperties } from 'react';
import {
  computeDivergence,
  divergenceColor,
  peakDivergenceTick,
  type SamplesBySession,
} from './selectors';

export interface DivergenceGutterProps {
  /** `samples[s][t]` = value of session s at tick t. NaN = missing. */
  samples: SamplesBySession;
  /** Called with the clicked tick (0-based) when the user snaps to a band. */
  onScrubTo?: (tick: number) => void;
  /** Optional label shown above the gutter (e.g. the variable name). */
  label?: string;
  /** Current playhead tick — highlights the matching band in the gutter. */
  currentTick?: number;
  /** Height of the gutter in px. Defaults to 120. */
  height?: number;
}

export function DivergenceGutter({
  samples,
  onScrubTo,
  label,
  currentTick,
  height = 120,
}: DivergenceGutterProps) {
  const divergence = useMemo(() => computeDivergence(samples), [samples]);
  const peak = useMemo(() => peakDivergenceTick(divergence), [divergence]);

  const bands = divergence.length;
  const hasBands = bands > 0;

  const containerStyle: CSSProperties = {
    width: 14,
    height,
    display: 'flex',
    flexDirection: 'column',
    background: 'var(--surface-container-lowest)',
    border: '1px solid var(--outline-variant)',
    borderRadius: 3,
    overflow: 'hidden',
    position: 'relative',
  };

  return (
    <div
      data-testid="divergence-gutter"
      className="flex flex-col items-center gap-1"
      title={
        label
          ? `Divergence heatmap — ${label}${
              peak >= 0 ? ` (peak at tick ${peak})` : ''
            }`
          : 'Divergence heatmap'
      }
    >
      {label && (
        <div
          style={{
            fontSize: 9,
            fontWeight: 600,
            textTransform: 'uppercase',
            letterSpacing: '0.05em',
            color: 'var(--outline)',
            maxWidth: 48,
            textAlign: 'center',
            lineHeight: 1.1,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {label}
        </div>
      )}
      <div style={containerStyle} role="group" aria-label="divergence gutter">
        {!hasBands && (
          <div
            style={{
              flex: 1,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              fontSize: 8,
              color: 'var(--outline)',
              writingMode: 'vertical-lr',
              textOrientation: 'mixed',
            }}
          >
            no data
          </div>
        )}
        {hasBands &&
          divergence.map((score, tick) => {
            const isPeak = tick === peak && score > 0;
            const isCurrent = currentTick === tick;
            return (
              <button
                key={tick}
                type="button"
                data-testid={`divergence-band-${tick}`}
                data-peak={isPeak ? 'true' : 'false'}
                aria-label={`Tick ${tick} divergence ${(score * 100).toFixed(0)}%`}
                title={`tick ${tick} · ${(score * 100).toFixed(1)}%${
                  isPeak ? ' (peak)' : ''
                }`}
                onClick={() => onScrubTo?.(tick)}
                style={{
                  flex: 1,
                  minHeight: 0,
                  border: 'none',
                  padding: 0,
                  cursor: onScrubTo ? 'pointer' : 'default',
                  background: divergenceColor(score),
                  outline: isCurrent ? '1px solid var(--primary)' : 'none',
                  outlineOffset: -1,
                  boxShadow: isPeak
                    ? 'inset 0 0 0 1px var(--chart-heat-max)'
                    : 'none',
                }}
              />
            );
          })}
      </div>
    </div>
  );
}

// Re-export the pure helper so tests can pull it from either module.
export { computeDivergence } from './selectors';
