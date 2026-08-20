/**
 * RunWorkflow — the live session workflow (route: /run).
 *
 * Previously `SessionWorkspace` under `features/sessions/`. Moved here as
 * part of R2.1 (workflow-as-route refactor).
 *
 * **Flag-off** (legacy `AppLayout` shell): behaviour unchanged — the
 * four-zone layout described in simulation-ui-endgame.md:
 *
 *   Zone 1 (left):        SessionTree — topology / target info
 *   Zone 2 (center):      Primary canvas (DiagramView) + subaction breadcrumb
 *   Zone 3 (bottom):      Results workbench (tabbed inspection surface)
 *   Zone 4 (bottom edge): SessionStatusBar
 *
 * **Flag-on** (`isFlagEnabled('ninebar')`, ninebar Phase 3 "Run,
 * re-composed"): the five-slot composition per the plan (§1 rows 2/4a,
 * §3). The old four-zone body is retired behind the flag (not deleted —
 * Phase 8 deletes it, per audit F17):
 *
 *   Left rail (portaled): SessionTreeV2 — same tree, same behaviour
 *     (selection, pin/promote, focus path); NOT rendered as an inline
 *     300px column any more (that would double-rail against the shell's
 *     own `left-rail` slot — audit F17's "no double-rail collision").
 *   Primary: SessionBreadcrumb (thin header, stays with the diagram it
 *     annotates) + DiagramView — full-bleed hero, no side column.
 *   Bottom strip (portaled): `WaveformCard` — the one always-useful live
 *     result (plan §1 row 4a). The ninebar KPI meter row + constraint
 *     chip have no demo reference (plan §5 reconciliation #4) and are
 *     explicitly Wave-2 scope — not built here.
 *   NOT rendered flag-on: `ResultsWorkbench` (the tabbed cram this phase
 *     retires) and `SessionStatusBar` (frame chips + `WaveformCard`'s
 *     footer readout now carry what it showed — see below).
 *
 * Reads from the session store (activeSessionId, phase) and session
 * queries. The session store is global — other workflows can observe
 * the active session even while they are the active route.
 */

import { useEffect, useRef } from 'react';
import { SessionHeader } from '@/features/sessions/SessionHeader';
import { SessionTreeV2 } from '@/features/sessions/tree/SessionTreeV2';
import { SessionBreadcrumb } from '@/features/sessions/tree/SessionBreadcrumb';
import { SessionStatusBar } from '@/features/sessions/SessionStatusBar';
import { DiagramView } from '@/components/DiagramView';
import { ResultsWorkbench } from '@/features/results/ResultsWorkbench';
import { WaveformCard } from '@/features/results/WaveformCard';
import { useSessionStore } from '@/features/sessions/store';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { isFlagEnabled } from '@/featureFlags';
import { LeftRailContent, BottomStripContent } from '@/app/slots';
import { useDrillReceiver } from './useDrillReceiver';
import { useDiagramUrlSync } from './useDiagramUrlSync';
import { DrilledFromBanner } from './DrilledFromBanner';
import { StepErrorBanner } from './StepErrorBanner';
import { ReadinessTeachingLine } from './ReadinessTeachingLine';

// ── RunWorkflow ─────────────────────────────────────────────────

export function RunWorkflow() {
  // R3.5: pick up drill-from-verdict handshake from the URL on mount.
  // No-op when the query string has no drill keys.
  useDrillReceiver();

  // Phase 4 step 2: keep `?uri=&view=` in sync with the workspace
  // store so diagrams are shareable and deep-linkable. On mount it
  // applies any URL params; afterward it writes store changes back
  // into the URL.
  useDiagramUrlSync();

  // Streaming is owned by the app-level <SessionStreamProvider/> (ninebar
  // Phase 1, audit F15) — this workflow only reads the stores it feeds.

  // When the user picks a different run target, abandon the
  // previously-started backend session so the next Run/Step creates
  // a fresh one tied to the new target.
  const activeSessionTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);
  const setActiveSession = useSessionStore((s) => s.setActiveSession);
  const setPhase = useSessionStore((s) => s.setPhase);
  const prevTargetRef = useRef<string | null>(activeSessionTarget);
  useEffect(() => {
    if (prevTargetRef.current !== activeSessionTarget) {
      prevTargetRef.current = activeSessionTarget;
      setActiveSession(null);
      setPhase('idle');
    }
  }, [activeSessionTarget, setActiveSession, setPhase]);

  const ninebar = isFlagEnabled('ninebar');
  const activeSessionId = useSessionStore((s) => s.activeSessionId);

  return (
    <div data-testid="session-workspace" className="flex flex-col h-full w-full overflow-hidden">
      {/* Drill-from-verdict banner (R3.5, shown only when the investigation trail has a hop in view) */}
      <DrilledFromBanner />

      {/* Step failure banner (P5, shown only when stepError is set) */}
      <StepErrorBanner />

      {/* Readiness teaching line (ninebar Phase 1.5, audit F12 — shown
          only when idle/no-target and the readiness aggregation is red) */}
      <ReadinessTeachingLine />

      {/* Session header with controls — legacy shell only. Under the
          ninebar shell, `AppShell`'s frame (`RunControls`) owns the step
          loop and run controls instead (interim primary-only mode, plan
          F17); rendering both here would double-mount
          `useSessionController`'s step loop for the same shell. */}
      {!ninebar && <SessionHeader />}

      {ninebar ? (
        <>
          {/* Left rail (portaled, `app/slots.tsx`): the SAME SessionTreeV2
              — same behaviour (selection, pin/promote, focus path) — but
              no longer an inline 300px column (F17: a workflow body must
              not own a second rail once the shell has one). `variant="rail"`
              (ruling B) gives it the calm treatment: quiet header, filters
              behind a popover, detail → the inspector rail context. */}
          <LeftRailContent>
            <SessionTreeV2 variant="rail" />
          </LeftRailContent>

          {/* Primary surface: full-bleed diagram hero. The breadcrumb
              stays here (not the rail) — it annotates the diagram's
              focus path, not the tree. */}
          <div className="flex flex-col flex-1 overflow-hidden">
            <SessionBreadcrumb />
            <div className="flex-1 overflow-hidden">
              <DiagramView />
            </div>
          </div>

          {/* Bottom strip (portaled): waveform only — no ResultsWorkbench,
              no SessionStatusBar (see WaveformCard's doc comment for what
              of the status bar moved into its footer readout). Ghost
              mode while no session exists (plan §0 "ghost/collapsed"):
              the transport row stays reachable — Play lazily creates the
              session — but the chart area reserves no dead space. */}
          <BottomStripContent collapsed={activeSessionId === null}>
            <WaveformCard />
          </BottomStripContent>
        </>
      ) : (
        <>
          {/* Main body: Zone 1 + Zone 2 */}
          <div className="flex flex-1 overflow-hidden">
            {/* Zone 1: Session tree V2 (part/sm/attr/constraint/ode rows).
                Replaces the flat TopologyView + global EventInjector per
            <div
              className="shrink-0 flex flex-col overflow-hidden"
              style={{ width: 300 }}
            >
              <SessionTreeV2 />
            </div>

            {/* Zone 2: Primary canvas — with live session overlays */}
            <div className="flex flex-col flex-1 overflow-hidden">
              {/* Hierarchical breadcrumb driven by focusPath (Phase B3). */}
              <SessionBreadcrumb />
              <div className="flex-1 overflow-hidden">
                <DiagramView />
              </div>
            </div>

          </div>

          {/* Zone 3: tabbed Results workbench */}
          <ResultsWorkbench />

          {/* Zone 4: Status bar */}
          <SessionStatusBar />
        </>
      )}
    </div>
  );
}
