/**
 * StateTimelineStrip — the state-machine swimlane re-homed into the
 * bottom strip (ninebar Phase 3 W3-D; plan: "state timeline → strip,
 * capability-gated, ghost otherwise") + the guard-diagnosis drill
 * migrated from the retired `simulation-ui-endgame.md`.
 *
 * Reuses `SwimlaneTimeline` (loops/fragments/trigger detection
 * included) over `useStateTimelineStore` — the same data the legacy
 * workbench tab renders, one source two shells.
 *
 * Guard drill: clicking a state segment shows every `GuardDiagnosis`
 * whose transition LEAVES that state — guard expression, per-variable
 * values, satisfied/blocked with the runtime's own explanation string.
 * Diagnoses come from `sessions.detail.latest_snapshot.guard_diagnoses`
 * (wire check 2026-07-14: already serialized, no backend work), which
 * is CURRENT-tick truth — the panel says so explicitly rather than
 * pretending to be historical.
 */
import { useEffect, useMemo, useRef, useState } from 'react';
import { SwimlaneTimeline } from '@/components/charts/SwimlaneTimeline';
import { detectFragments, detectLoops, detectTriggers } from '@/features/results/fragmentDetection';
import { useStateTimelineStore } from '@/features/results/selectors';
import { useSessionStore } from '@/features/sessions/store';
import { useSessionDetail } from '@/features/sessions/queries';
import { useTick } from '@/features/sessions/sessionLiveStore';

/** FE mirror of `sysml_runtime::statemachine::GuardDiagnosis`. */
interface GuardDiagnosisWire {
  guard_expr: string;
  transition: [string, string];
  event?: string | null;
  dependencies: string[];
  dependency_values: Record<string, unknown>;
  satisfied: boolean;
  explanation: string;
}

function readGuardDiagnoses(latestSnapshot: Record<string, unknown> | null | undefined): GuardDiagnosisWire[] {
  const raw = latestSnapshot?.guard_diagnoses;
  return Array.isArray(raw) ? (raw as GuardDiagnosisWire[]) : [];
}

interface Drill {
  subsystem: string;
  state: string;
}

export function StateTimelineStrip() {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const tick = useTick();
  const entriesBySession = useStateTimelineStore((s) => s.bySession);
  const entries = useMemo(
    () => (activeSessionId ? entriesBySession[activeSessionId] ?? [] : []),
    [activeSessionId, entriesBySession],
  );
  const { data: sessionDetail } = useSessionDetail(activeSessionId);

  const loops = useMemo(() => detectLoops(entries), [entries]);
  const fragments = useMemo(() => detectFragments(entries), [entries]);
  const triggers = useMemo(() => detectTriggers(entries), [entries]);

  const [drill, setDrill] = useState<Drill | null>(null);

  // Fill the strip width (same ResizeObserver pattern the workbench
  // card uses; the SVG renders at a fixed viewBox width otherwise).
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const [width, setWidth] = useState(600);
  useEffect(() => {
    const el = wrapRef.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver((obs) => {
      const w = obs[0]?.contentRect.width ?? 0;
      if (w > 0) setWidth(Math.max(Math.floor(w), 200));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const diagnoses = useMemo(() => {
    if (!drill) return [];
    const all = readGuardDiagnoses(
      sessionDetail?.latest_snapshot as Record<string, unknown> | null | undefined,
    );
    // Transitions LEAVING the clicked state.
    return all.filter((d) => d.transition?.[0] === drill.state);
  }, [drill, sessionDetail]);

  if (entries.length === 0) {
    return (
      <div
        data-testid="timeline-strip-empty"
        className="flex items-center justify-center h-full"
        style={{ color: 'var(--text-muted)', fontSize: 11 }}
      >
        No state-machine activity yet — run or step the model.
      </div>
    );
  }

  return (
    <div ref={wrapRef} data-testid="timeline-strip" className="relative h-full overflow-auto">
      <SwimlaneTimeline
        entries={entries}
        currentTick={tick ?? undefined}
        width={width}
        loops={loops}
        fragments={fragments}
        triggers={triggers}
        onSegmentClick={(info) => setDrill({ subsystem: info.subsystem, state: info.state })}
      />

      {drill && (
        <div
          data-testid="guard-drill"
          className="absolute flex flex-col gap-1"
          style={{
            top: 6,
            right: 6,
            maxWidth: 360,
            maxHeight: '85%',
            overflowY: 'auto',
            padding: 10,
            background: 'var(--surface-overlay)',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-md)',
            boxShadow: 'var(--shadow-float)',
            zIndex: 10,
          }}
        >
          <div className="flex items-baseline gap-2">
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-secondary)', textTransform: 'uppercase', letterSpacing: '0.03em' }}>
              Guards from
            </span>
            <span className="mono-text" style={{ fontSize: 'var(--text-sm)', color: 'var(--text-primary)' }}>
              {drill.state}
            </span>
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
              at tick {tick ?? '—'}
            </span>
            <button
              type="button"
              data-testid="guard-drill-close"
              onClick={() => setDrill(null)}
              aria-label="Close guard drill"
              className="material-symbols-outlined"
              style={{ marginLeft: 'auto', fontSize: 14, background: 'none', border: 'none', color: 'var(--text-muted)', cursor: 'pointer', padding: 2 }}
            >
              close
            </button>
          </div>

          {diagnoses.length === 0 ? (
            <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
              No guarded transitions leave this state (or the session hasn't produced a
              diagnosis yet — diagnoses reflect the current tick).
            </span>
          ) : (
            diagnoses.map((d, i) => (
              <div
                key={`${d.transition[0]}-${d.transition[1]}-${i}`}
                className="flex flex-col gap-0.5"
                style={{
                  padding: '6px 8px',
                  border: '1px solid var(--border-default)',
                  borderLeft: `2px solid ${d.satisfied ? 'var(--verdict-pass)' : 'var(--verdict-fail)'}`,
                  borderRadius: 'var(--radius-sm)',
                }}
              >
                <div className="mono-text" style={{ fontSize: 'var(--text-sm)', color: 'var(--text-primary)' }}>
                  {d.transition[0]} → {d.transition[1]}
                  {d.event ? <span style={{ color: 'var(--text-muted)' }}> on {d.event}</span> : null}
                </div>
                <div className="mono-text" style={{ fontSize: 'var(--text-xs)', color: 'var(--text-secondary)' }}>
                  [{d.guard_expr}]
                </div>
                {Object.entries(d.dependency_values ?? {}).map(([k, v]) => (
                  <div key={k} className="mono-text" style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
                    {k} = {String(typeof v === 'object' && v !== null ? JSON.stringify(v) : v)}
                  </div>
                ))}
                <div style={{ fontSize: 'var(--text-xs)', color: d.satisfied ? 'var(--verdict-pass)' : 'var(--verdict-fail)' }}>
                  {d.explanation}
                </div>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
