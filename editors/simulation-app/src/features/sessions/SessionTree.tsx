/**
 * SessionTree — Zone 1 left panel showing session topology.
 *
 * Pre-run: shows target structure from model stats / capabilities.
 * During/after run: enriches with topology data from useSessionTopology.
 * Live values: health dots per module, subsystem state badges.
 *
 * Per simulation-ui-endgame.md §"Zone 1: Session Panel".
 */

import { useSessionStore } from './store';
import { useSessionTopology, useSessionDetail, useSessionSubsystems } from './queries';
import { useSessionLiveStore } from './sessionLiveStore';
import { normalizeTopology } from './normalize';
import { useModelCapabilities } from '@/hooks/useModelCapabilities';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import type { NormalizedTopology } from './types';

// ── Pre-session: static target info ──────────────────────────────────

function PreSessionView() {
  const caps = useModelCapabilities();
  const activeTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);

  return (
    <div className="flex flex-col gap-2 p-3">
      {/* Target summary */}
      <div style={{ fontSize: '11px', color: 'var(--text-muted)', textTransform: 'uppercase', fontWeight: 600, letterSpacing: '0.06em' }}>
        Run Target
      </div>
      <div style={{ fontSize: '13px', color: 'var(--text-primary)', fontWeight: 500 }}>
        {activeTarget ?? 'None selected'}
      </div>

      {/* Capability indicators */}
      <div
        style={{
          marginTop: 8,
          fontSize: '11px',
          color: 'var(--text-muted)',
          textTransform: 'uppercase',
          fontWeight: 600,
          letterSpacing: '0.06em',
        }}
      >
        Model capabilities
      </div>
      <div className="flex flex-col gap-1" style={{ fontSize: '12px', color: 'var(--text-secondary)' }}>
        {caps.smCount > 0 && (
          <CapRow icon="swap_horiz" label={`${caps.smCount} state machine${caps.smCount !== 1 ? 's' : ''}`} />
        )}
        {caps.odeCount > 0 && (
          <CapRow icon="show_chart" label={`${caps.odeCount} ODE${caps.odeCount !== 1 ? 's' : ''}`} />
        )}
        {caps.flowCount > 0 && (
          <CapRow icon="route" label={`${caps.flowCount} flow${caps.flowCount !== 1 ? 's' : ''}`} />
        )}
        {caps.constraintCount > 0 && (
          <CapRow icon="check_circle" label={`${caps.constraintCount} constraint${caps.constraintCount !== 1 ? 's' : ''}`} />
        )}
        {caps.analysisCaseCount > 0 && (
          <CapRow icon="science" label={`${caps.analysisCaseCount} test${caps.analysisCaseCount !== 1 ? 's' : ''}`} />
        )}
        {caps.partCount > 0 && (
          <CapRow icon="category" label={`${caps.partCount} part${caps.partCount !== 1 ? 's' : ''}`} />
        )}
        {caps.smCount === 0 && caps.flowCount === 0 && caps.constraintCount === 0 && (
          <span style={{ color: 'var(--text-muted)', fontSize: '12px', fontStyle: 'italic' }}>
            No runnable capabilities detected
          </span>
        )}
      </div>
    </div>
  );
}

function CapRow({ icon, label }: { icon: string; label: string }) {
  return (
    <div className="flex items-center gap-2">
      <span className="material-symbols-outlined" style={{ fontSize: '14px', color: 'var(--text-secondary)' }}>
        {icon}
      </span>
      <span>{label}</span>
    </div>
  );
}

// ── Live topology view ───────────────────────────────────────────────

function TopologyView({
  topology,
  sessionId,
}: {
  topology: NormalizedTopology;
  sessionId: string;
}) {
  const activeTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);
  const { data: detail } = useSessionDetail(sessionId);
  const { data: subsystems } = useSessionSubsystems(sessionId);
  // Stage 6b: prefer the WS stream for tick / time_ms (30 Hz) over the
  // cheap metadata poll (2 Hz). Falls back to `detail.summary` when the
  // stream isn't active (flag unset / mid-reconnect).
  const liveTick = useSessionLiveStore((s) =>
    s.sessionId === sessionId ? s.snapshot?.tick ?? null : null,
  );
  const liveTimeMs = useSessionLiveStore((s) =>
    s.sessionId === sessionId ? s.snapshot?.time_ms ?? null : null,
  );
  const displayTick = liveTick ?? detail?.summary.tick;
  const displayTimeMs = liveTimeMs ?? detail?.summary.time_ms;
  const focusedActionPath = useSessionStore((s) => s.focusedActionPath);
  const pushFocusedAction = useSessionStore((s) => s.pushFocusedAction);
  const selectedScope = useSessionStore((s) => s.selectedScope);
  const setSelectedScope = useSessionStore((s) => s.setSelectedScope);

  // Build subsystem state map for live badges
  const subsystemStates = new Map<string, { state: string; completed: boolean; deferredCount: number }>();
  if (subsystems) {
    for (const sub of subsystems) {
      subsystemStates.set(sub.name, {
        state: sub.current_state,
        completed: sub.completed,
        deferredCount: sub.deferred_event_count ?? 0,
      });
    }
  }

  // Filter modules to show only children of the focused action scope.
  // When focusedActionPath is non-empty, filter modules whose label starts
  // with the focused path prefix. This is structural — refined once the
  // backend exposes nested action data more explicitly.
  const focusedPrefix = focusedActionPath.length > 0
    ? focusedActionPath.join('.') + '.'
    : '';
  const filteredModules = focusedPrefix
    ? topology.modules.filter(
        (mod) =>
          mod.label.startsWith(focusedPrefix) ||
          focusedActionPath.includes(mod.label),
      )
    : topology.modules;

  return (
    <div className="flex flex-col gap-1 p-3">
      {/* Run Target — preserved during the running session (P3).
          PreSessionView shows this; TopologyView previously dropped it in
          favour of `topology.rootLabel` / the drill-crumb, leaving the
          user without context for which target was actually running. */}
      {activeTarget && (
        <>
          <div
            data-testid="session-topology-run-target-label"
            style={{
              fontSize: '9px',
              color: 'var(--text-muted)',
              textTransform: 'uppercase',
              fontWeight: 600,
              letterSpacing: '0.06em',
            }}
          >
            Run Target
          </div>
          <div
            data-testid="session-topology-run-target"
            style={{
              fontSize: '12px',
              color: 'var(--text-primary)',
              fontWeight: 500,
              marginBottom: 4,
            }}
          >
            {activeTarget}
          </div>
        </>
      )}

      {/* Root label (or active drill crumb) */}
      <div style={{ fontSize: '13px', fontWeight: 600, color: 'var(--text-primary)', marginBottom: 2 }}>
        {focusedActionPath.length > 0
          ? focusedActionPath[focusedActionPath.length - 1]
          : topology.rootLabel}
      </div>

      {/* Tick/time + session id — subtitle that contextualises the
          root label now the target name has its own row above (P3). */}
      <div
        className="mono-text"
        style={{ fontSize: '10px', color: 'var(--text-muted)', marginBottom: 6 }}
        data-testid="session-topology-subtitle"
      >
        {sessionId.slice(0, 8)}
        {typeof displayTick === 'number' && typeof displayTimeMs === 'number'
          ? ` \u00b7 tick ${displayTick} \u00b7 t = ${(displayTimeMs / 1000).toFixed(3)}s`
          : ''}
      </div>

      {/* Modules */}
      {filteredModules.map((mod) => (
        <div key={mod.id} style={{ marginBottom: 6 }}>
          {/* Module header — clickable to drill down + scope variables */}
          <div
            className="flex items-center gap-2"
            style={{
              marginBottom: 2,
              cursor: 'pointer',
              background:
                selectedScope[0] === mod.label && selectedScope.length === 1
                  ? 'color-mix(in srgb, var(--accent) 10%, transparent)'
                  : undefined,
              borderRadius: 2,
            }}
            onClick={() => {
              pushFocusedAction(mod.label);
              setSelectedScope([mod.label]);
            }}
            title={`Drill into ${mod.label}`}
          >
            <span
              className="material-symbols-outlined"
              style={{ fontSize: '14px', color: 'var(--chart-series-6)' }}
            >
              view_module
            </span>
            <span style={{ fontSize: '12px', fontWeight: 600, color: 'var(--text-primary)' }}>
              {mod.label}
            </span>
            <span
              className="mono-text"
              style={{ fontSize: '9px', color: 'var(--text-muted)', textTransform: 'uppercase' }}
            >
              {mod.domain}
            </span>
            <span
              className="material-symbols-outlined"
              style={{ fontSize: '12px', color: 'var(--text-muted)', marginLeft: 'auto' }}
            >
              chevron_right
            </span>
          </div>

          {/* Subsystems */}
          <div style={{ paddingLeft: 20 }}>
            {mod.subsystemNames.map((name) => {
              const live = subsystemStates.get(name);
              const isScoped =
                selectedScope.length === 2 &&
                selectedScope[0] === mod.label &&
                selectedScope[1] === name;
              return (
                <div
                  key={name}
                  className="flex items-center gap-2 py-0.5"
                  style={{
                    fontSize: '11px',
                    cursor: 'pointer',
                    background: isScoped
                      ? 'color-mix(in srgb, var(--accent) 10%, transparent)'
                      : undefined,
                    borderRadius: 2,
                  }}
                  onClick={() => {
                    pushFocusedAction(name);
                    setSelectedScope([mod.label, name]);
                  }}
                  title={`Drill into ${name}`}
                >
                  {/* Health dot */}
                  <span
                    style={{
                      width: 6,
                      height: 6,
                      borderRadius: '50%',
                      background: live
                        ? live.completed
                          ? 'var(--sim-state-completed)'
                          : 'var(--sim-state-active)'
                        : 'var(--border-default)',
                      display: 'inline-block',
                      flexShrink: 0,
                    }}
                  />
                  <span style={{ color: 'var(--text-secondary)' }}>{name}</span>
                  {live && (
                    <span
                      className="mono-text"
                      style={{ fontSize: '10px', color: 'var(--text-muted)', marginLeft: 'auto' }}
                    >
                      {live.state}
                    </span>
                  )}
                  {/* Deferred event queue badge */}
                  {live && live.deferredCount > 0 && (
                    <span
                      title={`${live.deferredCount} deferred event${live.deferredCount !== 1 ? 's' : ''} queued`}
                      style={{
                        display: 'inline-flex',
                        alignItems: 'center',
                        justifyContent: 'center',
                        minWidth: 16,
                        height: 14,
                        padding: '0 3px',
                        borderRadius: 7,
                        background: 'var(--severity-warning)',
                        color: 'var(--on-verdict)',
                        fontSize: '8px',
                        fontWeight: 700,
                        lineHeight: 1,
                        flexShrink: 0,
                      }}
                    >
                      {live.deferredCount}
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}

// ── SessionTree ──────────────────────────────────────────────────────

export function SessionTree() {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const phase = useSessionStore((s) => s.phase);

  const { data: rawTopology } = useSessionTopology(activeSessionId);
  const topology = rawTopology ? normalizeTopology(rawTopology) : null;

  const hasSession = !!activeSessionId && phase !== 'idle';

  return (
    <div
      className="flex flex-col h-full overflow-y-auto"
      style={{
        background: 'var(--surface-sunken)',
        borderRight: '1px solid var(--border-default)',
      }}
    >
      {/* Section header */}
      <div
        className="flex items-center gap-2 px-3 shrink-0"
        style={{
          height: 28,
          borderBottom: '1px solid var(--border-default)',
          fontSize: '9px',
          fontWeight: 600,
          letterSpacing: '0.1em',
          textTransform: 'uppercase',
          color: 'var(--text-muted)',
        }}
      >
        <span className="material-symbols-outlined" style={{ fontSize: '14px' }}>
          account_tree
        </span>
        Session
      </div>

      {/* Content: pre-session or live topology */}
      {hasSession && topology ? (
        <TopologyView topology={topology} sessionId={activeSessionId!} />
      ) : (
        <PreSessionView />
      )}
    </div>
  );
}
