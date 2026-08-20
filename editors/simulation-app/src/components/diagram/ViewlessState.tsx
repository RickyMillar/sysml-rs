/**
 * ViewlessState — the first-class "no view selected" surface (ninebar Phase 2
 * W5 / F14).
 *
 * Views are the ONLY diagramming surface (views-first-class): the canvas
 * renders a declared `ViewUsage`/`ViewDefinition`, never a synthesized
 * projection. Landing here is therefore the COMMON path, not an error state —
 * most workspaces declare no views at all. This panel is the canvas' front
 * door:
 *
 *   - declared views exist → list them (kind + expose summary); clicking one
 *     renders it (the same `setSelectedViewId` the topology tree uses);
 *   - none exist → explain the views-first model and offer the guided
 *     scratch-view snippet (`CreateViewPrompt`, 'browse' context) seeded from
 *     the current tree selection. Never a synthesized default view.
 *
 * This is chrome, not canvas — it styles with the app's semantic tokens, not
 * the Rust canvas palette.
 */
import { useWorkspaceStore } from '@/store/workspace';
import { WORKSPACE_URI } from '@/shared/api/model';
import { useSelectionStore } from '@/features/selection/store';
import { useViewsList } from '@/features/views/queries';
import { viewKindLabel, summariseExposed } from '@/features/views/ViewsPanel';
import { CreateViewPrompt } from './CreateViewPrompt';

const shell: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 12,
  padding: 24,
  maxWidth: 560,
  margin: '0 auto',
  fontSize: 12,
  color: 'var(--text, #ddd)',
};

const mutedText: React.CSSProperties = {
  color: 'var(--outline, #999)',
  lineHeight: 1.5,
};

export function ViewlessState() {
  const workspaceRoot = useWorkspaceStore((s) => s.workspaceRoot);
  const setSelectedViewId = useWorkspaceStore((s) => s.setSelectedViewId);
  const selectedId = useSelectionStore((s) => s.selectedElementId);

  // No workspace loaded → null, which drives the "Load a workspace" empty
  // state below. Otherwise the view list is workspace-scoped.
  const viewsUri = workspaceRoot ? WORKSPACE_URI : null;
  const viewsQuery = useViewsList(viewsUri);
  const views = viewsQuery.data ?? [];

  if (!viewsUri) {
    return (
      <div data-testid="viewless-state" style={shell}>
        <div style={mutedText}>Load a workspace to render a diagram.</div>
      </div>
    );
  }
  if (viewsQuery.isLoading) {
    return (
      <div data-testid="viewless-state" style={shell}>
        <div style={mutedText}>Scanning for declared views…</div>
      </div>
    );
  }

  if (views.length > 0) {
    return (
      <div data-testid="viewless-state" style={shell}>
        <div style={{ fontSize: 13, fontWeight: 600 }}>Pick a view to render</div>
        <div style={mutedText}>
          The canvas renders a declared view. This workspace declares {views.length}:
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
          {views.map((v) => (
            <div
              key={v.id}
              role="button"
              tabIndex={0}
              data-testid={`viewless-view-row-${v.id}`}
              onClick={() => setSelectedViewId(v.id)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  setSelectedViewId(v.id);
                }
              }}
              style={{
                display: 'flex',
                alignItems: 'baseline',
                gap: 8,
                padding: '5px 8px',
                borderRadius: 4,
                cursor: 'pointer',
                background: 'var(--surface-sunken)',
              }}
            >
              <span style={{ color: 'var(--outline, #999)', fontSize: 11 }}>
                {viewKindLabel(v.kind)}
              </span>
              <span style={{ fontWeight: 600 }}>{v.name ?? <em>(unnamed)</em>}</span>
              <span style={{ color: 'var(--outline, #999)', fontSize: 11, marginLeft: 'auto' }}>
                {summariseExposed(v)}
              </span>
            </div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div data-testid="viewless-state" style={shell}>
      <div style={{ fontSize: 13, fontWeight: 600 }}>No declared views in this workspace</div>
      <div style={mutedText}>
        Diagrams render <b>declared views</b>: a <code>view</code> in your model that
        exposes the elements to draw. This workspace doesn't declare one yet — the
        topology tree, inspector, and run panels all work without it.
      </div>
      {selectedId ? (
        <CreateViewPrompt targetId={selectedId} context="browse" />
      ) : (
        <div data-testid="viewless-select-hint" style={mutedText}>
          Select an element in the topology tree to generate a starter view exposing it.
        </div>
      )}
    </div>
  );
}
