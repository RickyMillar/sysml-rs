/**
 * StreamCard — shows active streaming actions with emission rate counters.
 *
 * Each stream row displays: action name, emission count, rate (emissions/sec),
 * and last emitted value. Data is derived from the session snapshot's
 * `streaming_actions` field (populated by the ActionRunner when stream sources
 * are present).
 *
 * When no streaming action data is available, renders an empty-state message.
 *
 * NOTE (2026-04-17): the backend `ExecutionSnapshot` does not yet include a
 * `streaming_actions` field. Until it does, ResultsWorkbench keeps this
 * panel inactive with a "coming soon" hint. The empty-state branch remains
 * honest about the pending backend support.
 */

import { useMemo, useRef } from 'react';
import { CardShell } from './CardShell';

// ── Types ────────────────────────────────────────────────────────────────

export interface StreamActionEntry {
  /** Stream source node ID (action name). */
  name: string;
  /** Total emissions since start / last reset. */
  emissions: number;
  /** Last emitted value (stringified). */
  lastValue: string | null;
}

interface StreamCardProps {
  /** Active streaming action entries from the session snapshot. */
  streams: StreamActionEntry[];
  /** Current simulation clock time in ms (for rate computation). */
  clockTimeMs: number;
  /** Whether the simulation is currently running. */
  running: boolean;
  expanded?: boolean;
  onHeaderClick?: () => void;
}

// ── Component ────────────────────────────────────────────────────────────

export function StreamCard({
  streams,
  clockTimeMs,
  running,
  expanded,
  onHeaderClick,
}: StreamCardProps) {
  // Track previous emission counts for delta-based rate computation.
  const prevRef = useRef<{ time: number; counts: Record<string, number> }>({
    time: 0,
    counts: {},
  });

  const rows = useMemo(() => {
    const now = clockTimeMs;
    const prev = prevRef.current;
    const dt = (now - prev.time) / 1000; // seconds

    const result = streams.map((s) => {
      const prevCount = prev.counts[s.name] ?? 0;
      const delta = s.emissions - prevCount;
      const rate = dt > 0 ? delta / dt : 0;
      return { ...s, rate };
    });

    // Update ref for next render cycle
    const nextCounts: Record<string, number> = {};
    for (const s of streams) {
      nextCounts[s.name] = s.emissions;
    }
    prevRef.current = { time: now, counts: nextCounts };

    return result;
  }, [streams, clockTimeMs]);

  // Empty state
  if (rows.length === 0) {
    return (
      <CardShell
        title="Streams"
        icon="stream"
        accentColor="var(--text-secondary)"
        expanded={expanded}
        onHeaderClick={onHeaderClick}
      >
        <div style={{ fontSize: 'var(--text-xs)', color: 'var(--outline)' }}>
          Stream monitoring coming soon. Backend support is queued
          (see ExecutionSnapshot.streaming_actions).
        </div>
      </CardShell>
    );
  }

  return (
    <CardShell
      title="Streams"
      icon="stream"
      accentColor="var(--text-secondary)"
      expanded={expanded}
      onHeaderClick={onHeaderClick}
    >
      <div className="flex flex-col gap-1.5">
        {rows.map((row) => (
          <div
            key={row.name}
            className="flex flex-col gap-0.5 p-2 rounded"
            style={{ background: 'var(--surface-container-low)' }}
          >
            {/* Row header: name + rate badge */}
            <div className="flex items-center justify-between">
              <span
                className="mono-text"
                style={{
                  fontSize: 'var(--text-xs)',
                  fontWeight: 600,
                  color: 'var(--on-surface)',
                }}
              >
                {row.name}
              </span>
              {running && (
                <span
                  className="mono-text"
                  style={{
                    fontSize: '9px',
                    color: 'var(--chart-series-8)',
                    background: 'rgba(34,211,238,0.1)',
                    padding: '1px 4px',
                    borderRadius: '4px',
                  }}
                >
                  {row.rate.toFixed(1)}/s
                </span>
              )}
            </div>

            {/* Metrics row */}
            <div className="flex items-center gap-3" style={{ fontSize: '10px', color: 'var(--outline)' }}>
              <span>
                <span style={{ fontWeight: 600, color: 'var(--on-surface-variant)' }}>
                  {row.emissions}
                </span>{' '}
                emissions
              </span>
              {row.lastValue != null && (
                <span>
                  last:{' '}
                  <span className="mono-text" style={{ color: 'var(--chart-series-8)' }}>
                    {row.lastValue}
                  </span>
                </span>
              )}
            </div>
          </div>
        ))}
      </div>
    </CardShell>
  );
}
