/**
 * BrowseWorkflow — the `/browse` route (ninebar Phase 1.5, "Model
 * readiness & Browse floor").
 *
 * A systems engineer's first move is "is this model well-formed, and
 * what's in it?" — not "run it". This is the smallest viable reading +
 * navigation surface: package+element tree (left rail, portaled) +
 * a primary surface that switches between the source slice of the
 * selected element and the flat traceability matrix, plus the
 * inspector rail context on selection. This is a FLOOR, not the full
 * Browse from Phase 7 (views-first-class, hover previews, multi-file
 * tabs land later).
 *
 * No session anywhere: the tree (`useSessionModelTree` with
 * `expectedSessionId: null`), the source slice (`sysml.get_source`),
 * and the trace matrix (`sysml.trace_matrix`) are all workspace-level
 * reads — none of the three takes a session id, and none of this
 * component's own code touches `useSessionStore`.
 */
import { useEffect, useState } from 'react';
import { LeftRailContent } from '@/app/slots';
import { useRightRailStore } from '@/app/rail/railStore';
import { useSelectionStore } from '@/features/selection/store';
import { TraceabilityMatrixPanel } from '@/features/traceability/TraceabilityMatrixPanel';
import { BrowseTree } from './BrowseTree';
import { BrowseReadingSurface } from './BrowseReadingSurface';

type BrowseView = 'source' | 'trace';

const VIEW_OPTIONS: Array<{ value: BrowseView; label: string }> = [
  { value: 'source', label: 'Source' },
  { value: 'trace', label: 'Trace matrix' },
];

export function BrowseWorkflow() {
  const [view, setView] = useState<BrowseView>('source');

  // Selecting an element — from the tree OR a trace-matrix row click —
  // opens the inspector rail context. Centralised here (rather than in
  // each selection source) so both surfaces get the same behaviour for
  // free and neither has to know the rail exists.
  const selectedElementId = useSelectionStore((s) => s.selectedElementId);
  const openRailContext = useRightRailStore((s) => s.open);
  useEffect(() => {
    if (selectedElementId) openRailContext('inspector');
  }, [selectedElementId, openRailContext]);

  return (
    <>
      <LeftRailContent>
        <BrowseTree />
      </LeftRailContent>

      <div data-testid="browse-workflow" className="flex flex-col h-full w-full overflow-hidden">
        <div
          className="flex items-center gap-2 px-3 shrink-0"
          style={{ height: 'var(--row-default)', borderBottom: '1px solid var(--border-default)' }}
        >
          <div
            data-testid="browse-view-switch"
            role="radiogroup"
            aria-label="Browse view"
            className="inline-flex"
            style={{ border: '1px solid var(--border-default)', borderRadius: 6, overflow: 'hidden' }}
          >
            {VIEW_OPTIONS.map((opt, idx) => {
              const active = opt.value === view;
              return (
                <button
                  key={opt.value}
                  type="button"
                  role="radio"
                  aria-checked={active}
                  data-testid={`browse-view-switch-${opt.value}`}
                  data-active={active}
                  onClick={() => setView(opt.value)}
                  style={{
                    border: 'none',
                    borderLeft: idx > 0 ? '1px solid var(--border-default)' : 'none',
                    background: active ? 'var(--surface-raised)' : 'transparent',
                    color: active ? 'var(--text-primary)' : 'var(--text-secondary)',
                    padding: '4px 10px',
                    fontSize: 'var(--text-xs)',
                    fontWeight: active ? 600 : 500,
                    cursor: 'pointer',
                  }}
                >
                  {opt.label}
                </button>
              );
            })}
          </div>
        </div>

        <div className="flex-1 min-h-0 overflow-hidden">
          {view === 'source' ? <BrowseReadingSurface /> : <TraceabilityMatrixPanel />}
        </div>
      </div>
    </>
  );
}
