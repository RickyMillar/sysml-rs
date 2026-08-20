/**
 * PhasePill — frame chip for the active session's 6-phase state machine
 * (ninebar Phase 1, plan §0 frame row).
 *
 * Reads `phase` off `useSessionStore` (ADR-004 section 4: idle ->
 * configuring -> running -> paused/completed/error). A small dot marks
 * every phase except `running`, which swaps in the `<Ninebar compact/>`
 * indeterminate glyph — a running session IS a live/pending measure, the
 * one case the meter's governance sanctions outside a literal
 * progress/coverage value (see `src/components/Ninebar.tsx`).
 *
 * Colour discipline: amber (`--accent-*`) marks selection/active only and
 * must NEVER stand for status here. Only `error` gets a semantic colour
 * (`--severity-error`); every other phase renders in neutral text tones.
 */
import { Ninebar } from '@/components/Ninebar';
import { useSessionStore } from '@/features/sessions/store';
import type { SessionPhase } from '@/features/sessions/types';

const PHASE_LABEL: Record<SessionPhase, string> = {
  idle: 'Idle',
  configuring: 'Configuring',
  running: 'Running',
  paused: 'Paused',
  completed: 'Done',
  error: 'Error',
};

/** Dot colour per phase. `running` never renders its dot (Ninebar glyph
 *  replaces it) but keeps an entry here so the lookup stays total. */
const PHASE_DOT_COLOR: Record<SessionPhase, string> = {
  idle: 'var(--text-disabled)',
  configuring: 'var(--text-secondary)',
  running: 'var(--text-secondary)',
  paused: 'var(--text-secondary)',
  completed: 'var(--text-secondary)',
  error: 'var(--severity-error)',
};

export function PhasePill() {
  const phase = useSessionStore((s) => s.phase);

  return (
    <div
      data-testid="phase-pill"
      data-phase={phase}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        height: 'var(--row-compact)',
        padding: '0 8px',
        fontSize: 'var(--text-sm)',
        color: phase === 'error' ? 'var(--severity-error)' : 'var(--text-secondary)',
        border: '1px solid var(--border-default)',
        borderRadius: 'var(--radius-sm)',
      }}
    >
      {phase === 'running' ? (
        <Ninebar compact size={10} label="session running" />
      ) : (
        <span
          aria-hidden
          data-testid="phase-pill-dot"
          style={{
            width: 6,
            height: 6,
            borderRadius: '50%',
            background: PHASE_DOT_COLOR[phase],
            flexShrink: 0,
          }}
        />
      )}
      {PHASE_LABEL[phase]}
    </div>
  );
}
