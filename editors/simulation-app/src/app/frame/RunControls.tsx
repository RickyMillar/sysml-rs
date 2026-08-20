/**
 * RunControls — the frame's compact run-control cluster (ninebar
 * screenshot-comparison ruling A, 2026-07-14: "Transport moves to the
 * strip").
 *
 * Mounts `useSessionController` HERE — and ONLY here. In the ninebar
 * shell the frame is the one always-on surface that survives route
 * changes, so owning the step loop here (rather than in a per-route
 * header like the legacy `SessionHeader`) is what makes `PhasePill`
 * accurate off `/run`, and what lets a single mount back BOTH this
 * component's own run-N-ticks/stop controls AND the play/pause/resume/
 * step cluster that ruling A re-homes into `WaveformCard`'s header row
 * (`TransportBar.tsx`) — see `sessionControllerBridge.ts` for the full
 * wiring rationale. `SessionHeader` is the legacy shell's equivalent
 * mount; the two must never both be live at once for the same shell
 * (see `RunWorkflow.tsx`'s ninebar conditional, which suppresses
 * `SessionHeader` when this frame is active).
 *
 * Ruling A trims the frame itself to ONLY the generic bulk-step
 * ("Run N ticks") and Stop — both compact, icon-sized. Play / Pause /
 * Resume / Step (plus the running-state badge, which already lived in
 * `WaveformCard`'s header) move to `TransportBar`, rendered inside the
 * bottom strip. `RunControls` publishes the controller's action
 * callbacks into `useSessionControllerBridge` on every identity change
 * so `TransportBar` can invoke them without a second hook mount.
 *
 * GUARDRAIL (plan §4 "No model semantics in tool chrome" / §0 frame row):
 * every control here is named after the generic mechanism, never a
 * domain outcome. There is no "Run to trip" / "Run to breakpoint" named
 * button — "run until X" is expressed by arming a breakpoint (Right
 * Rail → Breakpoints, Phase 3) and using the plain "Run N ticks" bulk
 * step below, which stops early on its own the moment the backend
 * reports a halt (see `useSessionController`'s `interpretChunkResult`).
 * The old baked-in "Run to trip" button (`SessionHeader.tsx`, pre-BP5)
 * and its hybrid-tuned ~140k-tick default horizon are NOT migrated here —
 * confirmed removed from both `SessionHeader.tsx` and
 * `useSessionController.ts` (see `ecfac6ea`, which replaced them with
 * the generic `runToBreakpoint()` primitive / `RUN_TO_BREAKPOINT_SAFETY_CAP_TICKS`).
 *
 * Colour discipline: amber (`--accent-*`) marks the active/primary
 * action only — never a status. Disabled controls carry an explanatory
 * `title` (mirrors `SessionHeader`'s existing "no run target"
 * messaging) rather than silently no-op'ing.
 */
import { useEffect } from 'react';
import { useSessionController } from '@/features/sessions/useSessionController';
import { useSessionControllerBridge } from './sessionControllerBridge';
import { useModalStore } from '@/shared/overlays/modalStore';
// Side-effect import: registers the modal in the registry so openModal
// by id resolves (same pattern as the readiness StaticVerifyModal).
import { CONFIGURE_RUN_MODAL_ID } from '@/features/sessions/ConfigureRunModal';

/** Generic default batch size for the "Run N ticks" control (now in
 *  `TransportBar`). Not tuned to any one workflow's event — just a
 *  reasonable interactive default the user can change before Go. */
export const DEFAULT_RUN_N_TICKS = 100;

/** Shared with `TransportBar` (bottom-strip play/resume/step controls) —
 *  the same messaging regardless of which surface the button lives on.
 *
 *  There is only ONE state in which running is impossible: no model is
 *  loaded. "No element picked" is NOT that state — it means run the whole
 *  workspace, which is what `sessions.create` does with no `target` (see
 *  `TransportBar`'s `canStartRun`). The former `NO_TARGET_TITLE` told the
 *  user to go find a ▶ in the tree, which on a large model has none to
 *  find (punch-list finding 15). */
export const NO_MODEL_TITLE =
  'No model loaded — open a workspace before running';

/** Title for Play when no element is picked: says what will actually run,
 *  so "nothing selected" reads as a default rather than an omission. */
export const WHOLE_WORKSPACE_TITLE =
  'Run the whole workspace — every subsystem advances in lockstep. Pick a single element under Configure to narrow it';

/**
 * Shared chip/icon button. `icon` renders a compact square (matches the
 * crib sheet's 24×22px transport-button box, §4) with a Material Symbol
 * instead of text — used by the frame's icon-size Stop control and by
 * `TransportBar`'s play/pause/resume/step cluster. Text buttons (no
 * `icon`) keep the wider padded shape for labelled controls like
 * "Run N ticks".
 */
export function ChipButton({
  testId,
  onClick,
  disabled,
  title,
  accent,
  danger,
  icon,
  children,
}: {
  testId: string;
  onClick: () => void;
  disabled?: boolean;
  title: string;
  accent?: boolean;
  danger?: boolean;
  /** Material Symbols icon name. When set, renders icon-only (no children). */
  icon?: string;
  children?: React.ReactNode;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      onClick={onClick}
      disabled={disabled}
      title={title}
      aria-label={icon ? title : undefined}
      style={{
        height: 'var(--row-compact)',
        width: icon ? 'var(--row-compact)' : undefined,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: icon ? 'center' : 'flex-start',
        gap: 4,
        padding: icon ? 0 : '0 8px',
        fontSize: 'var(--text-sm)',
        fontWeight: accent ? 600 : 500,
        color: disabled
          ? 'var(--text-disabled)'
          : danger
            ? 'var(--severity-error)'
            : accent
              ? 'var(--accent-fg)'
              : 'var(--text-primary)',
        background: accent && !disabled ? 'var(--accent-tint)' : 'transparent',
        border: `1px solid ${accent && !disabled ? 'var(--accent)' : 'var(--border-default)'}`,
        borderRadius: 'var(--radius-sm)',
        cursor: disabled ? 'not-allowed' : 'pointer',
        whiteSpace: 'nowrap',
        flexShrink: 0,
      }}
    >
      {icon ? (
        <span className="material-symbols-outlined" aria-hidden="true" style={{ fontSize: 14 }}>
          {icon}
        </span>
      ) : (
        children
      )}
    </button>
  );
}

export function RunControls() {
  const controller = useSessionController();

  // Publish the controller's action callbacks for `TransportBar`
  // (bottom-strip, ruling A) to consume — see sessionControllerBridge.ts.
  // Deps are the individual callbacks (each already stable via
  // useCallback in useSessionController), not the fresh object literal
  // useSessionController returns every render, so this only re-publishes
  // when an action's identity actually changes.
  const setBridgeController = useSessionControllerBridge((s) => s.setController);
  useEffect(() => {
    setBridgeController(controller);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    controller.play,
    controller.pause,
    controller.resume,
    controller.stop,
    controller.stepOnce,
    controller.fastForward,
    controller.runToBreakpoint,
    controller.startSession,
    setBridgeController,
  ]);

  // User feedback 2026-07-14 ("half the playback controls at the top"):
  // ruling A's remainder moves down too. The frame now carries ZERO
  // playback controls — bulk-step, Stop, and the injector dock all live
  // in `TransportBar` (bottom strip) with the rest of the transport,
  // matching the demo frame exactly. This component's remaining jobs:
  // (1) own the single `useSessionController` mount + bridge publish,
  // (2) the Configure gear — setup, not playback, so it stays in chrome.
  return (
    <div
      data-testid="run-controls"
      className="flex items-center gap-1"
      style={{ height: 'var(--row-compact)' }}
    >
      {/* Configure-run modal (plan §3: setup never clutters the live
          surface) — target / dt / speed live behind this gear. */}
      <ChipButton
        testId="frame-control-configure"
        icon="settings"
        onClick={() => useModalStore.getState().openModal(CONFIGURE_RUN_MODAL_ID)}
        title="Configure run — target, step size, speed"
      >
        Configure
      </ChipButton>
    </div>
  );
}
