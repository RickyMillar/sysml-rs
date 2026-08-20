/**
 * SharedPlayhead — the single scrubber/time-cursor that drives every
 * waveform, diagram, and value display in the CompareWorkflow.
 *
 * Controls:
 *   - Step back (⟵)
 *   - Play / Pause (auto-advance at 100 ms/tick, stops at maxTick)
 *   - Step forward (⟶)
 *   - Range slider (jump to any tick)
 *
 * Keeps its own interval timer in an effect so the store's auto-advance
 * state is purely boolean — no setInterval IDs leak into Zustand.
 *
 * The playhead is deterministic: it defaults to paused so deep-links
 * (`?tick=42`) don't start racing forward the moment the page mounts.
 */

import { useEffect } from 'react';
import type { CSSProperties } from 'react';
import { useCompareStore } from './useCompareStore';

export interface SharedPlayheadProps {
  /** The maximum tick the playhead should reach (inclusive). */
  maxTick: number;
  /** Optional labels per session, shown as small pills with their frozen tick. */
  sessionTicks?: Array<{ id: string; label: string; ticks: number }>;
  /** Interval between auto-advances while playing, in ms. Default 100. */
  advanceMs?: number;
}

export function SharedPlayhead({
  maxTick,
  sessionTicks,
  advanceMs = 100,
}: SharedPlayheadProps) {
  const sharedTick = useCompareStore((s) => s.sharedTick);
  const setSharedTick = useCompareStore((s) => s.setSharedTick);
  const isPlaying = useCompareStore((s) => s.isPlaying);
  const setIsPlaying = useCompareStore((s) => s.setIsPlaying);

  // Clamp the tick whenever maxTick shrinks (e.g. user deselected the
  // longest session).
  useEffect(() => {
    if (sharedTick > maxTick) {
      setSharedTick(maxTick);
    }
  }, [maxTick, sharedTick, setSharedTick]);

  // Stop playing once we hit the end.
  useEffect(() => {
    if (isPlaying && sharedTick >= maxTick) {
      setIsPlaying(false);
    }
  }, [isPlaying, sharedTick, maxTick, setIsPlaying]);

  // Local auto-advance timer.
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

  const onStepBack = () => setSharedTick(Math.max(0, sharedTick - 1));
  const onStepForward = () => setSharedTick(Math.min(maxTick, sharedTick + 1));
  const onPlayToggle = () => {
    if (sharedTick >= maxTick) setSharedTick(0);
    setIsPlaying(!isPlaying);
  };

  const isDisabled = maxTick <= 0;

  return (
    <div
      data-testid="shared-playhead"
      className="flex items-center gap-3 px-4 shrink-0"
      style={{
        height: 44,
        background: 'var(--surface-container-low)',
        borderBottom: '1px solid var(--outline-variant)',
      }}
    >
      <PlayButton
        action="step-back"
        icon="skip_previous"
        title="Step back"
        disabled={isDisabled || sharedTick <= 0}
        onClick={onStepBack}
      />
      <PlayButton
        action="play-pause"
        icon={isPlaying ? 'pause' : 'play_arrow'}
        title={isPlaying ? 'Pause' : 'Play'}
        disabled={isDisabled}
        onClick={onPlayToggle}
      />
      <PlayButton
        action="step-forward"
        icon="skip_next"
        title="Step forward"
        disabled={isDisabled || sharedTick >= maxTick}
        onClick={onStepForward}
      />

      <div className="flex-1 flex items-center gap-3">
        <input
          data-testid="shared-playhead-scrubber"
          type="range"
          min={0}
          max={Math.max(0, maxTick)}
          step={1}
          value={Math.min(sharedTick, maxTick)}
          disabled={isDisabled}
          onChange={(e) => setSharedTick(Number(e.target.value))}
          style={{ flex: 1, accentColor: 'var(--primary)' }}
          aria-label="Shared playhead tick"
        />
        <span
          data-testid="shared-playhead-tick"
          className="mono-text shrink-0"
          style={{
            fontSize: 11,
            color: 'var(--on-surface-variant)',
            minWidth: 90,
            textAlign: 'right',
          }}
        >
          tick {sharedTick} / {maxTick}
        </span>
      </div>

      {sessionTicks && sessionTicks.length > 0 && (
        <div className="flex items-center gap-1" data-testid="shared-playhead-session-pills">
          {sessionTicks.map((s) => {
            const frozen = sharedTick >= s.ticks;
            return (
              <span
                key={s.id}
                title={
                  frozen
                    ? `${s.label} — frozen at tick ${s.ticks - 1} (session ended)`
                    : `${s.label} — at tick ${Math.min(sharedTick, s.ticks - 1)}`
                }
                style={{
                  fontSize: 10,
                  padding: '1px 6px',
                  borderRadius: 3,
                  background: 'var(--surface-container-high)',
                  color: frozen ? 'var(--outline)' : 'var(--on-surface)',
                  border: `1px solid ${
                    frozen ? 'var(--outline-variant)' : 'var(--primary)'
                  }`,
                  fontFamily: 'var(--font-mono, monospace)',
                }}
              >
                {s.label}
                {frozen ? ' •' : ''}
              </span>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ── Internal button ─────────────────────────────────────────────────

function PlayButton({
  action,
  icon,
  title,
  disabled,
  onClick,
}: {
  action: string;
  icon: string;
  title: string;
  disabled: boolean;
  onClick: () => void;
}) {
  const style: CSSProperties = {
    width: 28,
    height: 28,
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    border: '1px solid var(--outline-variant)',
    borderRadius: 4,
    background: disabled
      ? 'var(--surface-container)'
      : 'var(--surface-container-high)',
    color: disabled ? 'var(--outline)' : 'var(--on-surface)',
    cursor: disabled ? 'not-allowed' : 'pointer',
  };
  return (
    <button
      type="button"
      data-testid={`shared-playhead-${action}`}
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      style={style}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 18 }}>
        {icon}
      </span>
    </button>
  );
}

// Re-export for convenience in tests.
export { useCompareStore } from './useCompareStore';
