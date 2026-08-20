import { useMemo } from 'react';
import { useInjectEvent } from '@/features/sessions/mutations';
import type { SmTreeNode, SmTransitionDescriptor } from '../types';
import { useSessionStore } from '../../store';
import { DetailMeta, DetailShell } from './common';
import { StateGraph } from './StateGraph';

export function SmDetail({
  node,
  testIdPrefix,
}: {
  node: SmTreeNode;
  testIdPrefix: string;
}) {
  const states = node.states ?? [];
  const transitions = node.transitions ?? [];
  const available = node.availableTransitions ?? [];
  const currentState = node.currentState;

  return (
    <DetailShell testIdPrefix={testIdPrefix} suffix="sm">
      <DetailMeta node={node} />
      <div
        data-testid={`${testIdPrefix}-sm-state`}
        className="flex items-center gap-2"
      >
        <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>state</span>
        <span
          className="mono-text"
          style={{
            fontSize: 13,
            color: 'var(--text-primary)',
            background: 'var(--surface-panel)',
            padding: '2px 8px',
            borderRadius: 3,
            fontWeight: 600,
          }}
        >
          {currentState ?? '—'}
        </span>
        {typeof node.deferredCount === 'number' && node.deferredCount > 0 && (
          <span
            style={{
              fontSize: 10,
              color: 'var(--severity-warning)',
              fontWeight: 600,
            }}
            title="Pending deferred events"
          >
            {node.deferredCount} deferred
          </span>
        )}
        <span
          className="mono-text"
          style={{
            marginLeft: 'auto',
            fontSize: 9,
            color: 'var(--text-muted)',
          }}
        >
          {states.length} state{states.length === 1 ? '' : 's'} ·{' '}
          {transitions.length} transition{transitions.length === 1 ? '' : 's'}
        </span>
      </div>

      <StateGraph
        states={states}
        transitions={transitions}
        currentState={currentState}
        testId={`${testIdPrefix}-sm-graph`}
      />

      <AcceptedEventsRail
        available={available}
        subsystemName={node.name}
        testIdPrefix={testIdPrefix}
      />

      <TransitionTable
        staticTransitions={transitions}
        available={available}
        currentState={currentState}
        subsystemName={node.name}
        testIdPrefix={testIdPrefix}
      />
    </DetailShell>
  );
}

// ─── Accepted-events rail ─────────────────────────────────────────────

function AcceptedEventsRail({
  available,
  subsystemName,
  testIdPrefix,
}: {
  available: ReadonlyArray<readonly [string, string]>;
  subsystemName: string;
  testIdPrefix: string;
}) {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const injectEvent = useInjectEvent();

  // Dedupe events — a state may expose the same event with multiple
  // guards / targets. The rail shows one chip per unique event name;
  // click routes through the inject command which lets the backend
  // resolve which eligible transition fires.
  const uniqueEvents = useMemo(() => {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const [event] of available) {
      if (!seen.has(event)) {
        seen.add(event);
        out.push(event);
      }
    }
    return out;
  }, [available]);

  if (uniqueEvents.length === 0) {
    return (
      <div
        data-testid={`${testIdPrefix}-sm-rail-empty`}
        style={{
          fontSize: 11,
          color: 'var(--text-muted)',
          border: '1px dashed var(--border-default)',
          padding: 8,
          borderRadius: 4,
        }}
      >
        No events accepted from this state.
      </div>
    );
  }

  const disabled = !activeSessionId;

  return (
    <div data-testid={`${testIdPrefix}-sm-rail`}>
      <div
        style={{
          fontSize: 9,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          color: 'var(--text-muted)',
          marginBottom: 4,
        }}
      >
        Accepted events
      </div>
      <div className="flex flex-wrap gap-2">
        {uniqueEvents.map((event) => (
          <button
            key={event}
            type="button"
            data-testid={`${testIdPrefix}-sm-rail-${event}`}
            disabled={disabled || injectEvent.isPending}
            title={
              disabled
                ? 'Start or select a session to inject events.'
                : `Inject "${event}" into ${subsystemName}`
            }
            onClick={(e) => {
              e.stopPropagation();
              if (!activeSessionId) return;
              injectEvent.mutate({
                sessionId: activeSessionId,
                subsystem: subsystemName,
                event,
              });
            }}
            style={{
              fontSize: 11,
              padding: '4px 10px',
              borderRadius: 999,
              border: '1px solid var(--border-default)',
              background: disabled
                ? 'var(--surface-panel)'
                : 'var(--accent-tint)',
              color: disabled
                ? 'var(--text-muted)'
                : 'var(--text-primary)',
              cursor: disabled ? 'not-allowed' : 'pointer',
              fontFamily: 'var(--font-mono, ui-monospace)',
            }}
          >
            {event}
          </button>
        ))}
      </div>
    </div>
  );
}

// ─── Transition table ─────────────────────────────────────────────────

function TransitionTable({
  staticTransitions,
  available,
  currentState,
  subsystemName,
  testIdPrefix,
}: {
  staticTransitions: readonly SmTransitionDescriptor[];
  available: ReadonlyArray<readonly [string, string]>;
  currentState: string | undefined;
  subsystemName: string;
  testIdPrefix: string;
}) {
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const injectEvent = useInjectEvent();

  // Match live rows to static transitions by `target` (not by event
  // name): the static `TransitionUsage.name` is conventionally
  // `source_to_target`, while the runtime's event names come from
  // `trigger_events` on the same transition and rarely align with the
  // name. A row from the current state is "live" iff an entry in
  // `available_transitions` carries the same target — and the live
  // entry's event name drives the inject payload.
  const liveEventByTarget = useMemo(() => {
    const out = new Map<string, string>();
    for (const [event, target] of available) {
      if (!out.has(target)) out.set(target, event);
    }
    return out;
  }, [available]);

  if (staticTransitions.length === 0) {
    return (
      <div
        data-testid={`${testIdPrefix}-sm-ttbl-empty`}
        style={{ fontSize: 11, color: 'var(--text-muted)' }}
      >
        No transitions declared.
      </div>
    );
  }

  return (
    <div data-testid={`${testIdPrefix}-sm-ttbl`}>
      <div
        style={{
          fontSize: 9,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          color: 'var(--text-muted)',
          marginBottom: 4,
        }}
      >
        Transitions
      </div>
      <table
        style={{
          width: '100%',
          borderCollapse: 'collapse',
          fontSize: 11,
          fontFamily: 'var(--font-mono, ui-monospace)',
        }}
      >
        <thead>
          <tr
            style={{
              textAlign: 'left',
              color: 'var(--text-muted)',
              fontSize: 9,
              textTransform: 'uppercase',
            }}
          >
            <th style={{ padding: '2px 6px', fontWeight: 500 }}>from</th>
            <th style={{ padding: '2px 6px', fontWeight: 500 }}>event</th>
            <th style={{ padding: '2px 6px', fontWeight: 500 }}>to</th>
            <th
              style={{ padding: '2px 6px', fontWeight: 500, textAlign: 'right' }}
            >
              action
            </th>
          </tr>
        </thead>
        <tbody>
          {staticTransitions.map((t) => {
            const source = t.source;
            const target = t.target;
            const isFromCurrent =
              currentState !== undefined && source === currentState;
            const liveEvent =
              isFromCurrent && target !== undefined
                ? liveEventByTarget.get(target)
                : undefined;
            const isLiveEligible = liveEvent !== undefined;
            // Label: prefer the live event name once known; fall back
            // to the static transition name so the column isn't empty
            // for rows that aren't currently eligible.
            const label = liveEvent ?? t.name;
            return (
              <tr
                key={t.id}
                data-testid={`${testIdPrefix}-sm-ttbl-row-${t.id}`}
                data-live={isLiveEligible || undefined}
                style={{
                  borderTop: '1px solid var(--border-default)',
                  opacity: isFromCurrent ? 1 : 0.55,
                }}
              >
                <td style={{ padding: '3px 6px' }}>{source ?? '—'}</td>
                <td style={{ padding: '3px 6px' }}>{label}</td>
                <td style={{ padding: '3px 6px' }}>{target ?? '—'}</td>
                <td style={{ padding: '3px 6px', textAlign: 'right' }}>
                  {isLiveEligible && liveEvent ? (
                    <button
                      type="button"
                      data-testid={`${testIdPrefix}-sm-ttbl-inject-${t.id}`}
                      disabled={!activeSessionId || injectEvent.isPending}
                      onClick={(e) => {
                        e.stopPropagation();
                        if (!activeSessionId) return;
                        injectEvent.mutate({
                          sessionId: activeSessionId,
                          subsystem: subsystemName,
                          event: liveEvent,
                        });
                      }}
                      style={{
                        fontSize: 10,
                        padding: '1px 6px',
                        border: '1px solid var(--border-default)',
                        borderRadius: 3,
                        background: 'transparent',
                        color: 'var(--text-primary)',
                        cursor: 'pointer',
                      }}
                    >
                      inject
                    </button>
                  ) : (
                    <span style={{ color: 'var(--text-muted)' }}>—</span>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
