/**
 * CausalTracePanel — detail-pane panel for the "why did this happen?"
 * causal trace (R7.1).
 *
 * Given a root (set by the pass/fail grid or breakpoints panel via
 * `useCausalTraceStore.setRoot`), the panel fetches `sysml.causation.trace`
 * and renders the returned chain as a vertical timeline: root at the top
 * (highlighted), upstream causes below.
 *
 * Click a chain row → scrubs the RunWorkflow playhead to the event's tick
 * via `useInvestigationTrail.push` (reuses the R3.5 drill-receiver pattern
 * so the playhead highlight already exists; see ninebar Phase 1 / audit
 * F15 — this used to write `useSessionStore.setDrilledFrom`, which has
 * been replaced by the multi-hop trail store).
 *
 * Empty / loading / error states are handled inline; the panel is self-
 * contained so the registry host just renders `<CausalTracePanel />`.
 */

import { useMemo } from 'react';
import type { CausationEvent } from '@/engine/types';
import { useInvestigationTrail } from '@/features/investigation/useInvestigationTrail';
import { CausationEventRow } from './CausationEventRow';
import { useCausalTraceStore } from './useCausalTraceStore';
import { useCausationTrace, type CausalTraceRoot } from './useCausationTrace';

// The per-panel accent hue is neutralized (tokens-compat.css Phase 0
// de-rainbow): border weight vs text color still get distinct tokens.
const PANEL_BORDER = 'var(--border-default)';
const PANEL_ICON_COLOR = 'var(--text-secondary)';
const EMPTY_HINT =
  'No causal chain recorded for this event — the runtime may not have seen upstream writes within its retention window.';

export interface CausalTracePanelProps {
  /**
   * Optional override for the active root. When omitted the panel pulls
   * from `useCausalTraceStore` (production path). Used by tests and by
   * embedded previews that want to pin a specific trace.
   */
  root?: CausalTraceRoot | null;
  /**
   * Optional override for the scrub handler. When omitted the panel
   * pushes a hop onto `useInvestigationTrail`. Tests swap this for a
   * stub to assert click behaviour without mounting the whole
   * investigation-trail store machinery.
   */
  onScrubTo?: (event: CausationEvent) => void;
}

export function CausalTracePanel({ root: rootProp, onScrubTo }: CausalTracePanelProps = {}) {
  const storeRoot = useCausalTraceStore((s) => s.root);
  const root = rootProp !== undefined ? rootProp : storeRoot;
  const query = useCausationTrace(root);
  const pushHop = useInvestigationTrail((s) => s.push);

  const handleRowClick = (event: CausationEvent) => {
    if (onScrubTo) {
      onScrubTo(event);
      return;
    }
    // Default: push a trail hop so RunWorkflow highlights the tick. We
    // use the 'verify' origin because the drill-receiver payload shape
    // requires it and the behaviour ("scrub to this tick") is identical.
    pushHop({
      origin: 'verify',
      fromSessionId: root && 'sessionId' in root ? root.sessionId : '',
      tick: event.tick,
      elementId: event.target ?? undefined,
      label: `Causal trace · tick ${event.tick}`,
    });
  };

  return (
    <section
      data-testid="causal-trace-panel"
      style={{
        padding: 'var(--space-3, 12px)',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        gap: 'var(--space-2, 8px)',
      }}
    >
      <header
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--space-2, 8px)',
          borderBottom: `1px solid ${PANEL_BORDER}`,
          paddingBottom: 'var(--space-2, 8px)',
        }}
      >
        <span
          className="material-symbols-outlined"
          aria-hidden="true"
          style={{ color: PANEL_ICON_COLOR, fontSize: '20px' }}
        >
          account_tree
        </span>
        <h2
          style={{
            fontSize: 'var(--text-sm, 13px)',
            fontWeight: 600,
            margin: 0,
            color: 'var(--text-primary)',
          }}
        >
          Causal trace
        </h2>
      </header>

      <div
        style={{
          overflow: 'auto',
          flex: '1 1 auto',
          padding: '0 2px',
        }}
      >
        <CausalTraceBody
          rootSelected={root !== null}
          isLoading={query.isLoading}
          isError={query.isError}
          errorMessage={query.error?.message}
          chain={useMemo(() => query.data?.chain ?? [], [query.data])}
          onRowClick={handleRowClick}
        />
      </div>
    </section>
  );
}

interface CausalTraceBodyProps {
  rootSelected: boolean;
  isLoading: boolean;
  isError: boolean;
  errorMessage?: string;
  chain: CausationEvent[];
  onRowClick: (event: CausationEvent) => void;
}

function CausalTraceBody(props: CausalTraceBodyProps) {
  if (!props.rootSelected) {
    return (
      <EmptyState
        testid="causal-trace-no-root"
        copy="Select a failing verdict or a triggered breakpoint to trace its causal chain."
      />
    );
  }
  if (props.isLoading) {
    return (
      <EmptyState
        testid="causal-trace-loading"
        copy="Walking the causation graph…"
      />
    );
  }
  if (props.isError) {
    return (
      <EmptyState
        testid="causal-trace-error"
        copy={props.errorMessage ?? 'Failed to fetch causal trace.'}
        isError
      />
    );
  }
  if (props.chain.length === 0) {
    return <EmptyState testid="causal-trace-empty" copy={EMPTY_HINT} />;
  }

  return (
    <ol
      data-testid="causal-trace-chain"
      style={{
        listStyle: 'none',
        margin: 0,
        padding: 0,
      }}
    >
      {props.chain.map((event, idx) => (
        <li key={event.id} style={{ margin: 0, padding: 0 }}>
          <CausationEventRow
            event={event}
            index={idx}
            isRoot={idx === 0}
            onClick={props.onRowClick}
          />
        </li>
      ))}
    </ol>
  );
}

function EmptyState({
  testid,
  copy,
  isError,
}: {
  testid: string;
  copy: string;
  isError?: boolean;
}) {
  return (
    <div
      data-testid={testid}
      style={{
        padding: 'var(--space-4, 16px)',
        color: isError
          ? 'var(--severity-error)'
          : 'var(--text-secondary)',
        fontSize: 'var(--text-xs, 12px)',
        lineHeight: 1.4,
      }}
    >
      {copy}
    </div>
  );
}
