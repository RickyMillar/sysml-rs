/**
 * TransportBar — the play/pause/resume/step cluster, re-homed into the
 * bottom strip (ninebar screenshot-comparison ruling A, 2026-07-14:
 * "Transport moves to the strip"). Rendered inside `WaveformCard`'s
 * header row, at the TOP of the bottom strip per the demo (crib sheet
 * §4) — the running-state badge and tick/time readout it sits beside
 * already lived there (see `WaveformCard.tsx`'s doc comment); this
 * component only adds the transport buttons themselves.
 *
 * Does NOT mount `useSessionController` — that single mount stays in
 * the frame's `RunControls` (see its doc comment +
 * `sessionControllerBridge.ts`). This component reads the published
 * action callbacks from the bridge and computes its own phase-derived
 * enablement (`canPlay`/`canPause`/…) directly off `useSessionStore` /
 * `useWorkspaceUIStore` — the same global stores `RunControls` reads,
 * so both surfaces agree without any extra plumbing.
 *
 * Icon-size buttons (crib sheet §4: `24×22px` box, `1px` hairline
 * border, `4px` radius) — matches the frame's own icon-size Stop
 * control for a consistent transport vocabulary across the two
 * surfaces.
 */
import { useState } from 'react';
import {
  ChipButton,
  NO_MODEL_TITLE,
  WHOLE_WORKSPACE_TITLE,
  DEFAULT_RUN_N_TICKS,
} from './RunControls';
import { useSessionControllerBridge } from './sessionControllerBridge';
import { useSessionStore } from '@/features/sessions/store';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceStore } from '@/store/workspace';
import { InjectorDock } from './InjectorDock';

export function TransportBar() {
  const controller = useSessionControllerBridge((s) => s.controller);
  const phase = useSessionStore((s) => s.phase);
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const activeSessionTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);
  const hasLoadedModel = useWorkspaceStore((s) => s.loadedFiles.size > 0);

  const [ticksInput, setTicksInput] = useState(String(DEFAULT_RUN_N_TICKS));

  // "Can we start a run?" — NOT "has the user picked an element?".
  //
  // No explicit target is a legitimate, fully supported run: it means the
  // whole-workspace orchestrator. `createParamsForTarget(null)` has always
  // produced `{ uri: '__workspace__' }`, and `sessions.create` documents
  // "omit (or pass __workspace__) to run the whole multi-subsystem workspace
  // orchestrator". Gating Play on `activeSessionTarget` therefore made the
  // single most important action in the product unreachable whenever the
  // tree surfaced no ▶ launcher — which on a large model is the normal case
  // (punch-list findings 9, 15, 31: the only remaining route was the Cmd-K
  // developer console). One loaded model is enough; if that workspace has
  // nothing runnable the backend says so with a real error, which is the
  // fail-hard behaviour we want rather than a permanently disabled button.
  const canStartRun = hasLoadedModel || !!activeSessionTarget || !!activeSessionId;
  /** Whether Play would run the whole workspace rather than a picked element. */
  const runsWholeWorkspace = !activeSessionTarget && !activeSessionId;

  const canPlay = phase === 'idle' || phase === 'configuring';
  const canPause = phase === 'running';
  const canResume = phase === 'paused';
  const canStep = phase !== 'completed' && phase !== 'error';
  const canStopHere = phase === 'running' || phase === 'paused';
  // Mirrors `fastForward`'s own settled-phase gate (never race the
  // autoplay loop or restart a finished/errored run).
  const canRunNTicks = canStartRun && phase !== 'running' && phase !== 'completed' && phase !== 'error';
  const parsedTicks = Math.max(1, Math.floor(Number(ticksInput) || 0));

  // Bridge not published yet (brief window before RunControls' mount
  // effect fires) — render disabled rather than throwing on a null
  // controller. See sessionControllerBridge.ts's doc comment.
  const ready = controller !== null;

  return (
    <div
      data-testid="transport-bar"
      className="flex items-center gap-1"
      style={{ height: 'var(--row-compact)' }}
    >
      {canPlay && (
        <ChipButton
          testId="transport-run"
          icon="play_arrow"
          onClick={() => { void controller?.play(); }}
          disabled={!ready || !canStartRun}
          title={
            !canStartRun
              ? NO_MODEL_TITLE
              : runsWholeWorkspace
                ? WHOLE_WORKSPACE_TITLE
                : 'Run'
          }
          accent
        />
      )}

      {canResume && (
        <ChipButton
          testId="transport-resume"
          icon="play_arrow"
          onClick={() => controller?.resume()}
          disabled={!ready}
          title="Resume"
          accent
        />
      )}

      {canPause && (
        <ChipButton
          testId="transport-pause"
          icon="pause"
          onClick={() => controller?.pause()}
          disabled={!ready}
          title="Pause"
        />
      )}

      <ChipButton
        testId="transport-step"
        icon="skip_next"
        onClick={() => { void controller?.stepOnce(); }}
        disabled={!ready || !canStep}
        title={canStep ? 'Advance one tick' : 'Session has finished — Reset or start a new run to step again'}
      />

      {/* Bulk-step (moved down from the frame, user feedback 2026-07-14
          — the frame carries no playback controls now). Generic
          mechanism only: never a domain-named "run until X" button; it
          stops early on its own if a breakpoint fires. */}
      <input
        type="number"
        min={1}
        data-testid="transport-run-n-ticks-input"
        value={ticksInput}
        onChange={(e) => setTicksInput(e.target.value)}
        disabled={!canRunNTicks}
        title="Number of ticks to advance"
        className="mono-text"
        style={{
          width: 48,
          height: 'var(--row-compact)',
          padding: '0 6px',
          fontSize: 'var(--text-sm)',
          color: canRunNTicks ? 'var(--text-primary)' : 'var(--text-disabled)',
          background: 'transparent',
          border: '1px solid var(--border-default)',
          borderRadius: 'var(--radius-sm)',
        }}
      />
      <ChipButton
        testId="transport-run-n-ticks-go"
        icon="fast_forward"
        onClick={() => { void controller?.fastForward(parsedTicks); }}
        disabled={!ready || !canRunNTicks}
        title={
          !canStartRun
            ? NO_MODEL_TITLE
            : runsWholeWorkspace
              ? 'Run N ticks on the whole workspace — stops early if a breakpoint fires'
              : 'Run N ticks — stops early if a breakpoint fires'
        }
      />

      <ChipButton
        testId="transport-stop"
        icon="stop"
        onClick={() => controller?.stop()}
        disabled={!ready || !canStopHere}
        title="Stop"
        danger
      />

      {/* In-loop event injection (audit F1) — lives with the transport. */}
      <InjectorDock />
    </div>
  );
}
