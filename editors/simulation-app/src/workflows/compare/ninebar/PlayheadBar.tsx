/**
 * PlayheadBar — the ONE shared playhead of the Phase 6 Compare surface
 * (plan: "one shared playhead driving all panes"; contract F8:
 * "anchored on fork_point_tick for forked sessions").
 *
 * A 40px frame row: transport (step/play/step), the tick scrubber, a
 * marker strip carrying the fork anchor (⑂ at `fork_point_tick`) and
 * the first-divergence diamond (from `diff_timeline`), and the mono
 * tick readout. Auto-advance keeps its interval local (the store's
 * `isPlaying` stays a plain boolean, same discipline as the legacy
 * playhead it replaces).
 */

import { useEffect } from 'react';
import type { CSSProperties } from 'react';
import { useCompareStore } from '../useCompareStore';
import { xFor } from './svgPaths';

export interface PlayheadMarkers {
  /** `fork_point_tick` of a picked forked session (anchor glyph). */
  forkAnchorTick: number | null;
  /** `first_divergence_tick` from the pair timeline diff. */
  firstDivergenceTick: number | null;
}

export interface ForkableRow {
  /** Session id (row key). */
  id: string;
  /** `SessionSummary.forkable_ticks` — the EXACT archived rewind points. */
  ticks: number[];
  /** Stroke color for the session's dots (channel identity). */
  stroke: string;
}

export function PlayheadBar({
  maxTick,
  markers,
  forkableRows = [],
  advanceMs = 100,
}: {
  maxTick: number;
  markers: PlayheadMarkers;
  /** Per picked session: archived (forkable) ticks, drawn as dot rows
   *  under the scrubber so valid rewind points are visible instead of
   *  guessed (F8). Clicking a dot scrubs to that exact tick. */
  forkableRows?: ForkableRow[];
  advanceMs?: number;
}) {
  const sharedTick = useCompareStore((s) => s.sharedTick);
  const setSharedTick = useCompareStore((s) => s.setSharedTick);
  const isPlaying = useCompareStore((s) => s.isPlaying);
  const setIsPlaying = useCompareStore((s) => s.setIsPlaying);

  // Clamp when the domain shrinks (e.g. the longest session unpicked).
  useEffect(() => {
    if (sharedTick > maxTick) setSharedTick(maxTick);
  }, [maxTick, sharedTick, setSharedTick]);

  // Local auto-advance timer; stops at the end.
  useEffect(() => {
    if (!isPlaying) return;
    if (maxTick <= 0) {
      setIsPlaying(false);
      return;
    }
    const handle = setInterval(() => {
      const store = useCompareStore.getState();
      const next = store.sharedTick + 1;
      if (next > maxTick) {
        store.setIsPlaying(false);
        store.setSharedTick(maxTick);
        return;
      }
      store.setSharedTick(next);
    }, advanceMs);
    return () => clearInterval(handle);
  }, [isPlaying, maxTick, advanceMs, setIsPlaying]);

  const disabled = maxTick <= 0;
  const stripH = 6 + (forkableRows.length > 0 ? 2 + forkableRows.length * 2 : 0);
  const scale = { width: 100, height: stripH, maxTick, yMin: 0, yMax: 1 };

  return (
    <div
      data-testid="compare-playhead"
      className="flex items-center gap-2 px-3 shrink-0"
      style={{ height: 40, borderBottom: '1px solid var(--border-hairline)' }}
    >
      <TransportButton
        action="step-back"
        label="‹"
        title="Step back one tick"
        disabled={disabled || sharedTick <= 0}
        onClick={() => setSharedTick(Math.max(0, sharedTick - 1))}
      />
      <TransportButton
        action="play-pause"
        label={isPlaying ? '❚❚' : '▶'}
        title={isPlaying ? 'Pause' : 'Play'}
        disabled={disabled}
        onClick={() => {
          if (sharedTick >= maxTick) setSharedTick(0);
          setIsPlaying(!isPlaying);
        }}
      />
      <TransportButton
        action="step-forward"
        label="›"
        title="Step forward one tick"
        disabled={disabled || sharedTick >= maxTick}
        onClick={() => setSharedTick(Math.min(maxTick, sharedTick + 1))}
      />

      <div className="flex-1 flex flex-col justify-center" style={{ gap: 2 }}>
        <input
          data-testid="compare-playhead-scrubber"
          type="range"
          min={0}
          max={Math.max(0, maxTick)}
          step={1}
          value={Math.min(sharedTick, maxTick)}
          disabled={disabled}
          onChange={(e) => setSharedTick(Number(e.target.value))}
          style={{ width: '100%', accentColor: 'var(--accent)', height: 14 }}
          aria-label="Shared playhead tick"
        />
        {/* Marker strip — fork anchor + first divergence, x-aligned
            with the scrubber track. */}
        <svg
          data-testid="compare-playhead-markers"
          viewBox={`0 0 100 ${stripH}`}
          preserveAspectRatio="none"
          style={{ width: '100%', height: stripH, display: 'block' }}
        >
          {markers.forkAnchorTick !== null && maxTick > 0 && (
            <rect
              data-testid="compare-marker-fork-anchor"
              x={xFor(markers.forkAnchorTick, scale) - 0.3}
              y={0}
              width={0.6}
              height={stripH}
              fill="var(--text-secondary)"
            >
              <title>fork point · tick {markers.forkAnchorTick}</title>
            </rect>
          )}
          {markers.firstDivergenceTick !== null && maxTick > 0 && (
            <rect
              data-testid="compare-marker-first-divergence"
              x={xFor(markers.firstDivergenceTick, scale) - 0.3}
              y={0}
              width={0.6}
              height={stripH}
              fill="var(--diff-modified)"
            >
              <title>first divergence · tick {markers.firstDivergenceTick}</title>
            </rect>
          )}
          {/* F8: each picked session's ARCHIVED ticks — the exact set
              fork-at-tick accepts — as clickable dots, one row per
              session. */}
          {maxTick > 0 &&
            forkableRows.map((row, ri) => (
              <g key={row.id} data-testid={`compare-forkable-row-${row.id}`}>
                {row.ticks.map((t) => (
                  <rect
                    key={t}
                    x={xFor(t, scale) - 0.25}
                    y={7 + ri * 2}
                    width={0.5}
                    height={1.5}
                    fill={row.stroke}
                    style={{ cursor: 'pointer' }}
                    onClick={() => setSharedTick(t)}
                  >
                    <title>forkable · tick {t}</title>
                  </rect>
                ))}
              </g>
            ))}
        </svg>
      </div>

      {markers.firstDivergenceTick !== null && (
        <button
          type="button"
          data-testid="compare-jump-divergence"
          onClick={() => setSharedTick(markers.firstDivergenceTick ?? 0)}
          title={`Jump to first divergence (tick ${markers.firstDivergenceTick})`}
          style={{
            fontSize: 'var(--text-xs)',
            padding: '2px 8px',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            background: 'transparent',
            color: 'var(--diff-modified)',
            cursor: 'pointer',
            whiteSpace: 'nowrap',
          }}
        >
          ◆ divergence
        </button>
      )}

      <span
        data-testid="compare-playhead-tick"
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-xs)',
          color: 'var(--text-secondary)',
          minWidth: 96,
          textAlign: 'right',
        }}
      >
        tick {sharedTick} / {maxTick}
      </span>
    </div>
  );
}

function TransportButton({
  action,
  label,
  title,
  disabled,
  onClick,
}: {
  action: string;
  label: string;
  title: string;
  disabled: boolean;
  onClick: () => void;
}) {
  const style: CSSProperties = {
    width: 26,
    height: 24,
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    border: '1px solid var(--border-default)',
    borderRadius: 'var(--radius-sm)',
    background: 'transparent',
    color: disabled ? 'var(--text-disabled)' : 'var(--text-primary)',
    cursor: disabled ? 'not-allowed' : 'pointer',
    fontSize: 10,
    lineHeight: 1,
  };
  return (
    <button
      type="button"
      data-testid={`compare-playhead-${action}`}
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      style={style}
    >
      {label}
    </button>
  );
}
