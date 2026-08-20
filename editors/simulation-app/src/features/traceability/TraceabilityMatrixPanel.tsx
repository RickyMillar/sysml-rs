/**
 * TraceabilityMatrixPanel — panel wrapper that mounts the viewer in
 * the PanelRegistry pattern (R6.2).
 *
 * Registered in `shared/panels/registry.ts` with `defaultPosition:
 * 'detail'` — trace matrices read wider than the sidebar panels can
 * comfortably host, so the detail position is the right surface.
 *
 * Responsibilities stop at:
 *   - fetching via `useTraceMatrix`
 *   - wiring the workspace URI from the workspace store
 *   - rendering loading / error states around the viewer
 *
 * The viewer itself handles filtering, sorting, density, and row
 * clicks — this panel just supplies data and context.
 */

import { useState } from 'react';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { TraceabilityMatrixViewer } from './TraceabilityMatrixViewer';
import { useTraceMatrix } from './useTraceMatrix';
import { useLensProbe } from './useLensProbe';
import {
  DEFAULT_TRACE_LENS_ID,
  TRACE_LENSES,
  describeSelectors,
  lensById,
  lensForSelectors,
} from './lenses';
import type { TraceSelectors } from './types';

export interface TraceabilityMatrixPanelProps {
  /**
   * Overrides the workspace URI from the store. When `undefined` the
   * panel reads from `useWorkspaceUIStore.workspaceRoot`.
   */
  workspaceUri?: string | null;
  /**
   * Which element-kind / relationship-kind trio to forward to the
   * backend. Defaults to the `useTraceMatrix` default
   * (`PartUsage + Satisfy + RequirementUsage` — requirement = target).
   */
  selectors?: TraceSelectors;
  /** When `false`, hides the viewer's top filter bar. */
  showFilterBar?: boolean;
}

export function TraceabilityMatrixPanel(props: TraceabilityMatrixPanelProps = {}) {
  const storeWorkspaceUri = useWorkspaceUIStore((s) => s.workspaceRoot);
  const effectiveUri =
    props.workspaceUri !== undefined ? props.workspaceUri : storeWorkspaceUri;

  // A caller-supplied `selectors` prop still wins outright — this panel is
  // embedded by Requirements with its own triple, and that caller is asking a
  // specific question, not browsing. The picker drives the unpinned case.
  const [lensId, setLensId] = useState(DEFAULT_TRACE_LENS_ID);
  const pinned = props.selectors !== undefined;
  const activeSelectors = props.selectors ?? lensById(lensId).selectors;
  const activeLens = pinned ? lensForSelectors(activeSelectors) : lensById(lensId);

  const { matrix, query } = useTraceMatrix({
    workspace_uri: effectiveUri,
    selectors: activeSelectors,
  });

  // Probe the other lenses ONLY when this one came back empty.
  const isEmpty = query.isSuccess && (query.data?.length ?? 0) === 0;
  const probe = useLensProbe(effectiveUri ?? null, activeLens?.id ?? '', isEmpty);

  if (!effectiveUri) {
    return (
      <div
        data-testid="trace-matrix-panel-no-workspace"
        style={{
          padding: 24,
          textAlign: 'center',
          color: 'var(--text-muted)',
          fontSize: 12,
          fontStyle: 'italic',
        }}
      >
        Load a workspace to see the traceability matrix.
      </div>
    );
  }

  if (query.isLoading) {
    return (
      <div
        data-testid="trace-matrix-panel-loading"
        style={{
          padding: 24,
          textAlign: 'center',
          color: 'var(--text-muted)',
          fontSize: 12,
        }}
      >
        Loading trace matrix…
      </div>
    );
  }

  if (query.isError) {
    const message =
      query.error instanceof Error ? query.error.message : String(query.error);
    return (
      <div
        data-testid="trace-matrix-panel-error"
        style={{
          padding: 24,
          textAlign: 'center',
          color: 'var(--severity-error)',
          fontSize: 12,
          display: 'flex',
          flexDirection: 'column',
          gap: 8,
          alignItems: 'center',
        }}
      >
        <div>Failed to load trace matrix.</div>
        <div style={{ color: 'var(--text-muted)', fontSize: 11 }}>{message}</div>
        <button
          type="button"
          onClick={() => void query.refetch()}
          data-testid="trace-matrix-panel-retry"
          style={{
            padding: '4px 12px',
            fontSize: 11,
            background: 'var(--accent)',
            color: 'var(--on-accent)',
            border: 'none',
            borderRadius: 3,
            cursor: 'pointer',
          }}
        >
          Retry
        </button>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full min-h-0" data-testid="trace-matrix-panel">
      {/* The active lens, always visible. An empty grid with no statement of
          which question produced it is unreadable — the reader cannot tell a
          model with no traceability from a model being asked the wrong
          thing. */}
      <div
        className="flex items-center gap-2"
        style={{
          padding: '6px 10px',
          borderBottom: '1px solid var(--border-hairline)',
          fontSize: 11,
          color: 'var(--text-muted)',
          flexWrap: 'wrap',
        }}
        data-testid="trace-matrix-lens-bar"
      >
        <span>Lens</span>
        {pinned ? (
          <span
            className="mono-text"
            style={{ color: 'var(--text-primary)' }}
            data-testid="trace-matrix-lens-pinned"
            title="This surface asks one fixed question"
          >
            {activeLens?.label ?? describeSelectors(activeSelectors)}
          </span>
        ) : (
          <select
            data-testid="trace-matrix-lens-select"
            value={lensId}
            onChange={(e) => setLensId(e.target.value)}
            style={{
              background: 'var(--surface-sunken)',
              color: 'var(--text-primary)',
              border: '1px solid var(--border-default)',
              borderRadius: 'var(--radius-sm)',
              padding: '2px 6px',
              fontSize: 11,
            }}
          >
            {TRACE_LENSES.map((l) => (
              <option key={l.id} value={l.id}>
                {l.label}
              </option>
            ))}
          </select>
        )}
        <span className="mono-text" style={{ fontSize: 10.5, opacity: 0.8 }} data-testid="trace-matrix-lens-triple">
          {describeSelectors(activeSelectors)}
        </span>
      </div>

      {isEmpty ? (
        <div
          className="flex flex-col items-center gap-3"
          style={{ padding: 24, textAlign: 'center', fontSize: 12, color: 'var(--text-muted)' }}
          data-testid="trace-matrix-empty"
        >
          <div>
            No {activeSelectors.relation_kind} links from{' '}
            <span className="mono-text">{activeSelectors.source_kind}</span> to{' '}
            <span className="mono-text">{activeSelectors.target_kind}</span> in this workspace.
          </div>

          {probe.isProbing ? (
            <div data-testid="trace-matrix-empty-probing" style={{ fontStyle: 'italic' }}>
              checking the other lenses…
            </div>
          ) : probe.suggestions.length > 0 ? (
            <div className="flex flex-col items-center gap-2" data-testid="trace-matrix-suggestions">
              <div>This workspace does have traceability — under a different lens:</div>
              {probe.suggestions.map(({ lens, edgeCount }) => (
                <button
                  key={lens.id}
                  type="button"
                  data-testid={`trace-matrix-suggest-${lens.id}`}
                  onClick={() => setLensId(lens.id)}
                  disabled={pinned}
                  title={pinned ? 'This surface is pinned to one lens' : `Show ${lens.question}`}
                  style={{
                    padding: '4px 12px',
                    fontSize: 11,
                    background: 'var(--accent-tint)',
                    color: 'var(--accent-fg)',
                    border: '1px solid var(--accent)',
                    borderRadius: 'var(--radius-sm)',
                    cursor: pinned ? 'default' : 'pointer',
                  }}
                >
                  {lens.label} — {edgeCount} {edgeCount === 1 ? 'link' : 'links'}
                </button>
              ))}
            </div>
          ) : probe.probedAndEmpty ? (
            <div data-testid="trace-matrix-empty-everywhere">
              No lens finds any links here — this workspace has no modelled traceability yet.
            </div>
          ) : null}
        </div>
      ) : (
        <div className="flex-1 min-h-0">
          <TraceabilityMatrixViewer
            data={matrix}
            workspaceUri={effectiveUri}
            showFilterBar={props.showFilterBar}
          />
        </div>
      )}
    </div>
  );
}
