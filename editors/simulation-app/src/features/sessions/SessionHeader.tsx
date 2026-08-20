/**
 * SessionHeader — top bar with run target name, status badge, and
 * Run/Pause/Stop/Step controls.
 *
 * Buttons call the session controller (play, pause, stop, stepOnce).
 * Phase-aware: buttons enable/disable based on the current 6-phase
 * state machine.
 *
 * Per simulation-ui-endgame.md §"Session Workspace" header row.
 * Includes split-button Run dropdown per §"Run Configuration".
 */

import { useState, useRef, useEffect, useCallback, useMemo } from 'react';
import { useNavigate } from 'react-router-dom';
import { useSessionStore } from './store';
import { useSessionController } from './useSessionController';
import { useSessionDetail, useSessionTopology } from './queries';
import { useForkSession, useResetSession } from './mutations';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { exportJSON } from '@/shared/export';
import type { SessionPhase } from './types';
import { TickSpeedControl } from './TickSpeedControl';
import { DtMsInput } from './DtMsInput';
import { StepSizeAdvisoryBadge } from './StepSizeAdvisoryBadge';
import {
  useBreakpointStore,
  selectArmedCount,
  breakpointLabel,
} from '@/features/breakpoints/useBreakpointStore';

// ── Phase badge ──────────────────────────────────────────────────────

/**
 * `pausedAtBreakpointLabel` distinguishes a breakpoint-driven halt from
 * a plain user Pause click (BP5, item 3c) — when set, the badge reads
 * "paused · <label>" instead of the bare phase name so the user sees
 * WHICH breakpoint stopped the run without having to open the panel.
 */
function PhaseBadge({
  phase,
  pausedAtBreakpointLabel,
}: {
  phase: SessionPhase;
  pausedAtBreakpointLabel?: string | null;
}) {
  const colorMap: Record<SessionPhase, string> = {
    idle: 'var(--text-muted)',
    configuring: 'var(--accent-fg)',
    running: 'var(--sim-state-active)',
    paused: 'var(--severity-warning)',
    completed: 'var(--sim-state-completed)',
    error: 'var(--severity-error)',
  };

  const text =
    phase === 'paused' && pausedAtBreakpointLabel
      ? `paused · ${pausedAtBreakpointLabel}`
      : phase;

  return (
    <span
      className="mono-text"
      data-testid="phase-badge"
      title={phase === 'paused' && pausedAtBreakpointLabel ? 'Paused at a breakpoint' : undefined}
      style={{
        fontSize: '10px',
        fontWeight: 700,
        textTransform: phase === 'paused' && pausedAtBreakpointLabel ? 'none' : 'uppercase',
        letterSpacing: '0.06em',
        color: colorMap[phase],
        padding: '2px 8px',
        borderRadius: 4,
        background: 'var(--surface-raised)',
        whiteSpace: 'nowrap',
      }}
    >
      {text}
    </span>
  );
}

// ── Control buttons ──────────────────────────────────────────────────

function ControlButton({
  icon,
  label,
  onClick,
  disabled,
  accent,
  danger,
  title,
}: {
  icon: string;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  accent?: boolean;
  danger?: boolean;
  /** Tooltip override — defaults to `label` (e.g. to explain why the
   *  control is currently disabled). */
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={title ?? label}
      data-testid={`control-${label.toLowerCase()}`}
      className="flex items-center gap-1 px-2.5 py-1 rounded transition-all"
      style={{
        background: accent
          ? 'linear-gradient(135deg, var(--accent-fg), var(--accent))'
          : 'var(--surface-raised)',
        color: danger
          ? 'var(--severity-error)'
          : accent
            ? 'var(--on-accent)'
            : 'var(--text-primary)',
        border: 'none',
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.4 : 1,
        fontSize: '12px',
        fontWeight: 600,
      }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: '16px' }}>
        {icon}
      </span>
      {label}
    </button>
  );
}

// ── Run Configuration split-button dropdown ─────────────────────────
// Per simulation-ui-endgame.md §"Run Configuration":
//   Primary = Run, dropdown = Step-by-step | separator | Sweep/MC/What-If

interface RunDropdownProps {
  onRun: () => void;
  onStepByStep: () => void;
  disabled?: boolean;
}

function RunDropdown({ onRun, onStepByStep, disabled }: RunDropdownProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  return (
    <div ref={ref} style={{ position: 'relative', display: 'inline-flex' }}>
      {/* Primary Run button */}
      <button
        onClick={onRun}
        disabled={disabled}
        title={
          disabled
            ? 'No run target — in the tree, click ▶ Run on a state machine, analysis case, or verification case to select one'
            : 'Run'
        }
        data-testid="control-run"
        className="flex items-center gap-1 px-2.5 py-1 transition-all"
        style={{
          background: 'linear-gradient(135deg, var(--accent-fg), var(--accent))',
          color: 'var(--on-accent)',
          border: 'none',
          borderRadius: '4px 0 0 4px',
          cursor: disabled ? 'not-allowed' : 'pointer',
          opacity: disabled ? 0.4 : 1,
          fontSize: '12px',
          fontWeight: 600,
        }}
      >
        <span className="material-symbols-outlined" style={{ fontSize: '16px' }}>
          play_arrow
        </span>
        Run
      </button>

      {/* Chevron toggle */}
      <button
        onClick={() => setOpen((o) => !o)}
        disabled={disabled}
        title="Run options"
        data-testid="run-dropdown-toggle"
        className="flex items-center py-1 px-1 transition-all"
        style={{
          background: 'linear-gradient(135deg, var(--accent-fg), var(--accent))',
          color: 'var(--on-accent)',
          border: 'none',
          borderLeft: '1px solid rgba(255,255,255,0.2)',
          borderRadius: '0 4px 4px 0',
          cursor: disabled ? 'not-allowed' : 'pointer',
          opacity: disabled ? 0.4 : 1,
          fontSize: '12px',
        }}
      >
        <span className="material-symbols-outlined" style={{ fontSize: '14px' }}>
          {open ? 'expand_less' : 'expand_more'}
        </span>
      </button>

      {/* Dropdown menu */}
      {open && (
        <div
          data-testid="run-dropdown-menu"
          style={{
            position: 'absolute',
            top: '100%',
            right: 0,
            marginTop: 4,
            minWidth: 220,
            background: 'var(--surface-raised)',
            border: '1px solid var(--border-default)',
            borderRadius: 6,
            boxShadow: '0 8px 24px rgba(0,0,0,0.3)',
            zIndex: 100,
            overflow: 'hidden',
          }}
        >
          {/* Step-by-step is a Run-internal mode. Parameter sweeps,
              Monte Carlo and what-if comparisons live in their own
              workflows under /analyze and /compare — discovery happens
              from the model tree (Launch analysis case) and from the
              top nav, not from this dropdown. */}
          <DropdownItem
            icon="step"
            label="Step-by-step"
            onClick={() => { setOpen(false); onStepByStep(); }}
          />

        </div>
      )}
    </div>
  );
}

function DropdownItem({ icon, label, onClick }: { icon: string; label: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className="flex items-center gap-2 w-full text-left px-3 py-1.5 transition-colors"
      style={{
        background: 'transparent',
        color: 'var(--text-primary)',
        border: 'none',
        cursor: 'pointer',
        fontSize: '12px',
        fontWeight: 500,
      }}
      onMouseEnter={(e) => {
        (e.currentTarget as HTMLElement).style.background = 'var(--surface-raised)';
      }}
      onMouseLeave={(e) => {
        (e.currentTarget as HTMLElement).style.background = 'transparent';
      }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: '16px', color: 'var(--text-muted)' }}>
        {icon}
      </span>
      {label}
    </button>
  );
}

// Render a human-friendly session label for the top bar. Preference order:
//   1. User-renamed label (sessions.rename)
//   2. Primary subsystem name from the backend summary (e.g. "TrafficLight")
//   3. Shortened session UUID ("sess abcd1234") once the backend returns it
//   4. "No target selected"
//
// The full session UUID is always available via the element's `title`
// tooltip so the ID remains copyable for debugging.
function renderSessionLabel({
  label,
  subsystemName,
  sessionId,
  targetId,
}: {
  label?: string | null;
  subsystemName?: string | null;
  sessionId?: string | null;
  targetId?: string | null;
}): string {
  if (label && label.length > 0) return label;
  if (subsystemName && subsystemName.length > 0) return subsystemName;
  if (sessionId) return `sess ${sessionId.slice(0, 8)}`;
  if (targetId) return `sess ${targetId.slice(0, 8)}`;
  return 'No target selected';
}

// ── SessionHeader ────────────────────────────────────────────────────

export function SessionHeader() {
  const phase = useSessionStore((s) => s.phase);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const setCompareBaseline = useSessionStore((s) => s.setCompareBaseline);
  const setActiveSession = useSessionStore((s) => s.setActiveSession);
  const activeSessionTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);
  const navigateCompare = useNavigate();
  const controller = useSessionController();
  const forkSession = useForkSession();
  const resetSession = useResetSession();
  const { data: sessionDetail } = useSessionDetail(activeSessionId);
  const { data: topology } = useSessionTopology(activeSessionId);

  // BP5: "Run to breakpoint" needs at least one armed breakpoint — a
  // horizon-less bulk-step drive with none would just burn the safety
  // cap for no reason.
  const armedBreakpointCount = useBreakpointStore(selectArmedCount);
  const breakpointEntries = useBreakpointStore((s) => s.breakpoints);

  // Which breakpoint (if any) the ACTIVE session is currently halted at
  // (BP1 `paused_at_breakpoint` on the summary) — drives the PhaseBadge's
  // "paused · <label>" text (item 3c) so a breakpoint halt reads
  // differently from a plain user Pause click.
  const pausedAtBreakpointId = sessionDetail?.summary.paused_at_breakpoint ?? null;
  const pausedAtBreakpointLabel = useMemo(() => {
    if (!pausedAtBreakpointId) return null;
    const match = breakpointEntries.find((e) => e.id === pausedAtBreakpointId);
    return match ? breakpointLabel(match.breakpoint) : pausedAtBreakpointId;
  }, [pausedAtBreakpointId, breakpointEntries]);


  // A run needs a selected runnable target (or an already-started session).
  // Without one, play()/stepOnce() can't build a start command — they log an
  // error and flip to the error phase with no visible guidance. Drives both the
  // disabled Run control and the inline "pick a target" hint below.
  const hasRunTarget = !!activeSessionTarget || !!activeSessionId;

  // Phase-based button enablement
  const canPlay = phase === 'idle' || phase === 'configuring';
  const canPause = phase === 'running';
  const canResume = phase === 'paused';
  const canStop = phase === 'running' || phase === 'paused';
  const canStep = phase !== 'completed' && phase !== 'error';
  const canFork = !!activeSessionId && phase !== 'idle';
  // Reset rewinds the active session in-place (tick → 0, scalar_vars
  // → initial). Disallowed while running because the result would
  // race the autoplay loop's next step. Allowed in paused / completed
  // / error so the user can recover from a stuck or finished run
  // without losing their run configuration.
  const canReset =
    !!activeSessionId &&
    (phase === 'paused' || phase === 'completed' || phase === 'error');
  // "Run to breakpoint" bulk-steps with no fixed horizon, stopping the
  // instant the backend reports `paused` (BP1) — replaces the old
  // baked-in "Run to trip" 140k-tick default (task #5: the horizon
  // must not be hardcoded to one workflow's event). Allowed from the
  // same settled phases `runToBreakpoint()` itself accepts; needs a
  // runnable target AND at least one armed breakpoint (there is
  // nothing to run TO otherwise).
  const canRunToBreakpoint =
    hasRunTarget &&
    armedBreakpointCount > 0 &&
    (phase === 'idle' || phase === 'configuring' || phase === 'paused');

  const handleReset = useCallback(() => {
    if (!activeSessionId) return;
    // Destructive: tick + state vars rewind to initial, breakpoints
    // and time-series history are dropped. Confirm to avoid an
    // accidental click after a long run.
    // eslint-disable-next-line no-alert
    const ok = window.confirm(
      'Reset this session to its initial state? Tick will return to 0 and accumulated state (sparklines, last-change ticks) will clear.',
    );
    if (!ok) return;
    resetSession.mutate(activeSessionId, {
      onSuccess: () => {
        // The session-live store keeps the same sessionId across a
        // reset, so the bridge's session-change clear doesn't fire.
        // The bridge + change tracker now both detect tick regression
        // and clear themselves on the next snapshot — see
        // `useLiveTimeSeriesBridge` and `useSessionModelTree`.
      },
    });
  }, [activeSessionId, resetSession]);

  const handleFork = useCallback(() => {
    if (!activeSessionId) return;
    forkSession.mutate(
      { sessionId: activeSessionId },
      {
        onSuccess: (forkedSummary) => {
          setCompareBaseline(activeSessionId);
          setActiveSession(forkedSummary.id);
          navigateCompare('/run/compare');
        },
      },
    );
  }, [activeSessionId, forkSession, setCompareBaseline, setActiveSession, navigateCompare]);

  const handleExportSession = useCallback(() => {
    if (!sessionDetail) return;
    const archive = {
      exportedAt: new Date().toISOString(),
      detail: sessionDetail,
      topology: topology ?? null,
    };
    const name = activeSessionTarget ?? activeSessionId ?? 'session';
    exportJSON(archive, `session-${name}.json`);
  }, [sessionDetail, topology, activeSessionTarget, activeSessionId]);


  return (
    <div
      data-testid="session-header"
      className="flex items-center gap-3 px-4 shrink-0"
      style={{
        height: 40,
        background: 'var(--surface-sunken)',
        borderBottom: '1px solid var(--border-default)',
      }}
    >
      {/* Target name */}
      <span
        className="material-symbols-outlined"
        style={{ fontSize: '16px', color: 'var(--accent-fg)' }}
      >
        science
      </span>
      <span
        style={{ fontSize: '13px', fontWeight: 600, color: 'var(--text-primary)' }}
        title={activeSessionId ?? undefined}
      >
        {renderSessionLabel({
          label: sessionDetail?.summary.label,
          subsystemName: sessionDetail?.summary.subsystem_name,
          sessionId: activeSessionId,
          targetId: activeSessionTarget,
        })}
      </span>

      {/* Phase badge */}
      <PhaseBadge phase={phase} pausedAtBreakpointLabel={pausedAtBreakpointLabel} />

      {/* Discovery hint: with no runnable target the Run controls are
          disabled, so point the user at where targets come from — the
          tree's inline ▶ Run action on a state machine / case. */}
      {!hasRunTarget && (
        <span
          data-testid="no-target-hint"
          style={{
            fontSize: 11,
            color: 'var(--text-muted)',
            display: 'inline-flex',
            alignItems: 'center',
            gap: 4,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 13 }}>
            arrow_back
          </span>
          Select a runnable in the tree (click ▶ Run on a state machine, analysis, or verification case)
        </span>
      )}

      <div className="flex-1" />

      {/* Tick speed (compact button group) */}
      <TickSpeedControl />

      {/* dt step size input */}
      <DtMsInput />

      {/* Runtime step-size under-resolution advisory — renders only when a
          subsystem's observed oscillation is step-bound; suggests a finer dt
          but never changes it. */}
      <StepSizeAdvisoryBadge snapshot={sessionDetail?.latest_snapshot} />

      {/* Subtle vertical divider before the playback cluster */}
      <div
        aria-hidden="true"
        style={{
          width: 1,
          height: 20,
          background: 'var(--border-default)',
          margin: '0 4px',
        }}
      />

      {/* Controls */}
      <div className="flex items-center gap-1.5">
        {canPlay && (
          <RunDropdown
            onRun={() => { void controller.play(); }}
            onStepByStep={() => { void controller.stepOnce(); }}
            // No runnable target → Run/Step would no-op (and log an error).
            // Disable with a tooltip that points at the tree's ▶ Run action.
            disabled={!hasRunTarget}
          />
        )}
        {canResume && (
          <ControlButton
            icon="play_arrow"
            label="Resume"
            onClick={controller.resume}
            accent
          />
        )}
        {canPause && (
          <ControlButton
            icon="pause"
            label="Pause"
            onClick={controller.pause}
          />
        )}
        <ControlButton
          icon="skip_next"
          label="Step"
          onClick={() => { void controller.stepOnce(); }}
          disabled={!canStep}
        />
        <ControlButton
          icon="fast_forward"
          label="Run to breakpoint"
          onClick={() => { void controller.runToBreakpoint(); }}
          disabled={!canRunToBreakpoint}
          title={
            armedBreakpointCount === 0
              ? 'Arm a breakpoint (⌘⇧B) to enable Run to breakpoint'
              : 'Run to breakpoint'
          }
        />
        <ControlButton
          icon="stop"
          label="Stop"
          onClick={controller.stop}
          disabled={!canStop}
          danger
        />
        <ControlButton
          icon="restart_alt"
          label="Reset"
          onClick={handleReset}
          disabled={!canReset || resetSession.isPending}
        />
        <ControlButton
          icon="fork_right"
          label="Fork"
          onClick={handleFork}
          disabled={!canFork || forkSession.isPending}
        />
        <ControlButton
          icon="download"
          label="Export"
          onClick={handleExportSession}
          disabled={!sessionDetail}
        />
      </div>
    </div>
  );
}
