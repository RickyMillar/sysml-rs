/**
 * ViewsPanel — sidebar surface for user-authored ViewUsage / ViewDefinition
 * elements (Phase 5).
 *
 * Lists every view discovered in the focused workspace via
 * `sysml.query`. Click a row → fetches `sysml.views.render` and
 * pushes the resulting ViewModel into `useWorkspaceStore` so the Diagram
 * pane re-renders against the authored view's Expose / filter /
 * rendering members.
 *
 * Bucket 5.K1 — viewpoint traceability:
 *   Each row shows the satisfaction chain (Stakeholder → Viewpoint →
 *   View) when the data is available. The backend currently exposes
 *   only the *forward* compatibility filters:
 *     • sysml.query viewpoint(stakeholder_id) → viewpoints
 *     • sysml.query view(viewpoint_id)        → views
 *   We need the *inverse* (view → viewpoints, viewpoint → stakeholders)
 *   to populate these rows efficiently. Bucket 3.4 will land that
 *   query (`sysml-core/src/view_index.rs::viewpoints_for_view`); until
 *   then the rows show a "no traceability data yet" hint rather than
 *   an N-query FE walk.
 *
 * The selected view id is sourced from the workspace store so URL
 * sync (`?view_id=...`) and tree-click (Bucket 5.R2) stay coherent.
 *
 * Empty states:
 *   - No URI focused        → "Open a model to see authored views"
 *   - URI focused, 0 views  → "No `view def` declarations in this file"
 *
 * Works against `__workspace__` URIs too (cross-file authored views).
 */

import { useCallback, useState } from 'react';
import { WORKSPACE_URI } from '@/shared/api/model';
import { useWorkspaceStore } from '@/store/workspace';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useSelectionStore } from '@/features/selection/store';
import {
  fileUriOf,
  useViewsList,
  useViewsByViewpoint,
  type ViewSummary,
  type ViewpointPickerEntry,
} from './queries';
import { ViewRow } from './ViewRow';
import { ViewpointPicker } from './ViewpointPicker';

const styles = {
  root: {
    display: 'flex',
    flexDirection: 'column' as const,
    height: '100%',
    fontSize: 13,
    color: 'var(--text-primary)',
  },
  header: {
    padding: '8px 12px',
    borderBottom: '1px solid var(--border-default)',
    display: 'flex',
    flexDirection: 'column' as const,
    gap: 6,
  },
  headerTitle: {
    fontSize: 12,
    color: 'var(--text-muted)',
    textTransform: 'uppercase' as const,
    letterSpacing: '0.05em',
  },
  list: {
    overflowY: 'auto' as const,
    flex: 1,
    padding: 4,
  },
  row: {
    padding: '8px 10px',
    margin: '2px 0',
    borderRadius: 6,
    cursor: 'pointer',
    background: 'var(--surface-panel)',
    border: '1px solid transparent',
  },
  rowSelected: {
    background: 'var(--accent-tint)',
    border: '1px solid var(--accent)',
  },
  rowName: {
    fontWeight: 500,
    color: 'var(--text-primary)',
  },
  rowMeta: {
    marginTop: 2,
    fontSize: 11,
    color: 'var(--text-muted)',
  },
  trace: {
    marginTop: 4,
    fontSize: 11,
    color: 'var(--text-muted)',
    fontStyle: 'italic' as const,
  },
  empty: {
    padding: 24,
    textAlign: 'center' as const,
    fontSize: 12,
    color: 'var(--text-muted)',
  },
  error: {
    padding: 12,
    color: 'var(--severity-error)',
    fontSize: 12,
  },
};

export function viewKindLabel(kind: string): string {
  // Strip the trailing "Definition" / "Usage" so the row reads as a
  // human-friendly stereotype.
  if (kind.endsWith('Definition')) return 'view def';
  if (kind.endsWith('Usage')) return 'view';
  return kind;
}

export function summariseExposed(s: ViewSummary): string {
  if (s.exposed.length === 0) return 'no exposed elements';
  const names = s.exposed
    .map((e) => e.qualified_name ?? '?')
    .filter((n) => n !== '?');
  if (names.length === 0) return `${s.exposed.length} expose(s)`;
  if (names.length <= 3) return names.join(', ');
  return `${names.slice(0, 3).join(', ')} +${names.length - 3} more`;
}

export interface ViewsPanelProps {
  /** Override the focused URI (mostly for tests). */
  uri?: string | null;
}

export function ViewsPanel({ uri: uriOverride }: ViewsPanelProps = {}) {
  const selectedViewId = useWorkspaceStore((s) => s.selectedViewId);
  const setSelectedViewId = useWorkspaceStore((s) => s.setSelectedViewId);
  const setFocusedUri = useWorkspaceStore((s) => s.setFocusedUri);
  const setActiveUtility = useWorkspaceUIStore((s) => s.setActiveUtility);
  const select = useSelectionStore((s) => s.select);

  // Authored views can live in any file and reference elements across
  // files (Bucket 5 demo-fix, 2026-05-05): the panel always queries the
  // merged workspace graph so cross-file views show up. An explicit
  // `uriOverride` prop still wins for the rare caller that scopes to one
  // file; a per-file scope toggle can come back as a future affordance.
  const uri = uriOverride ?? WORKSPACE_URI;

  const [selectedViewpoint, setSelectedViewpoint] =
    useState<ViewpointPickerEntry | null>(null);
  const viewpointId = selectedViewpoint?.id ?? null;

  // Keep both hooks always enabled so flipping the viewpoint filter
  // never re-fetches the underlying unfiltered view list. The
  // viewpoint-scoped hook is `enabled` only when an id is selected.
  const allViews = useViewsList(uri);
  const filtered = useViewsByViewpoint(uri, viewpointId);
  const list = viewpointId ? filtered : allViews;

  // Render-on-select moved to top-level <SelectedViewRenderer /> in
  // App.tsx so SessionTree's inline view children (which fire while
  // ViewsPanel may not be mounted) drive the diagram correctly.
  const onPickView = useCallback(
    (id: string) => {
      setSelectedViewId(id);
    },
    [setSelectedViewId],
  );

  /**
   * Phase 3 — promote a hover-preview into the Source drawer. Pushes
   * the view's element id onto selection, focuses the file the view
   * is declared in (when known), and opens the Source utility panel.
   * The picked-view side-effect is intentionally left alone — the
   * user is reaching for source, not for a re-render.
   *
   * The URI we hold here is workspace-scoped (`__workspace__`) when a
   * workspace root is loaded — that's not a real file URI, so we
   * don't push it onto focusedUri. The Source drawer falls back to
   * whichever file the user already had focused, which is the right
   * UX: hovering a view shows the view's declaration site as a
   * preview, but the editor stays on whatever file the user was
   * working in.
   */
  const handlePromote = useCallback(
    (id: string) => {
      // Bug B fix: focus the view's *own* file when promoting from a
      // hover. The workspace URI is `__workspace__` when the panel runs
      // workspace-wide; that sentinel isn't a real file. Resolve via
      // the view's source_span instead.
      const view = (list.data ?? []).find((v) => v.id === id);
      const fileUri = view ? fileUriOf(view) : null;
      const resolvedUri =
        fileUri ?? (uri === WORKSPACE_URI ? null : uri);
      select(resolvedUri, id);
      if (resolvedUri) setFocusedUri(resolvedUri);
      setActiveUtility('source');
    },
    [list.data, select, setActiveUtility, setFocusedUri, uri],
  );

  const selectedViewpointLabel = selectedViewpoint
    ? selectedViewpoint.name ?? selectedViewpoint.qualified_name ?? selectedViewpoint.id
    : null;

  if (list.error) {
    return (
      <div style={styles.root}>
        <div style={styles.header}>
          <div style={styles.headerTitle}>Views</div>
        </div>
        <div style={styles.error}>Failed to load views: {String(list.error)}</div>
      </div>
    );
  }

  const views = list.data ?? [];

  return (
    <div style={styles.root}>
      <div style={styles.header}>
        <div style={styles.headerTitle}>
          Views ({views.length})
          {viewpointId ? ' — filtered' : ''}
        </div>
        {/* Guided create-view (F14) for workspaces that already declare
            views — authoring belongs here (the views home / Browse),
            not on Run's canvas picker (§6 authoring loop). */}
        <button
          type="button"
          data-testid="views-panel-new"
          onClick={() => {
            void Promise.all([
              import('@/shared/overlays/modalStore'),
              import('@/components/diagram/CreateViewModal'),
            ]).then(([{ useModalStore }, { CREATE_VIEW_MODAL_ID }]) =>
              useModalStore.getState().openModal(CREATE_VIEW_MODAL_ID),
            );
          }}
          title="Create a new view (guided)"
          className="material-symbols-outlined"
          style={{
            fontSize: 14,
            width: 22,
            height: 20,
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'var(--text-secondary)',
            background: 'none',
            border: '1px dashed var(--border-default)',
            borderRadius: 'var(--radius-sm)',
            cursor: 'pointer',
          }}
        >
          add
        </button>
        <ViewpointPicker
          uri={uri}
          selectedId={viewpointId}
          selectedLabel={selectedViewpointLabel}
          onSelect={(entry) => setSelectedViewpoint(entry)}
          onClear={() => setSelectedViewpoint(null)}
        />
      </div>
      <div style={styles.list}>
        {list.isLoading && (
          <div style={styles.empty}>Loading…</div>
        )}
        {!list.isLoading && views.length === 0 && (
          <div style={styles.empty}>
            No <code>view def</code> / <code>view</code> declarations found.
          </div>
        )}
        {views.map((v) => (
          <ViewRow
            key={v.id}
            view={v}
            selected={v.id === selectedViewId}
            onPick={onPickView}
            onPromote={handlePromote}
            // Bug B fix: hover preview needs the view's *own* declaration
            // file URI, not the workspace-scope sentinel `__workspace__`.
            // `sysml.get_source` rejects the sentinel with "no graph for
            // URI", which left the popover stuck on "Loading source…"
            // for every view row. The span's `file` already lives on the
            // backend response — pull it through `fileUriOf` (strips
            // the `file://` scheme the rest of the FE doesn't use).
            previewUri={fileUriOf(v)}
            styles={styles as Record<string, React.CSSProperties>}
            kindLabel={viewKindLabel(v.kind)}
            exposedSummary={summariseExposed(v)}
          />
        ))}
      </div>
    </div>
  );
}
