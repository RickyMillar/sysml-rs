import { useWorkspaceStore } from '@/store/workspace';
import { WORKSPACE_URI } from '@/shared/api/model';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useSessionStore } from '@/features/sessions/store';
import { useViewsList } from '@/features/views/queries';
import { findDeclaredViewForElement } from '@/features/sessions/tree/preferredView';
import { TableView } from './TableView';
import { BrowserView } from './BrowserView';
import { GeometryView } from './GeometryView';
import { CreateViewPrompt } from './CreateViewPrompt';
import { ViewlessState } from './ViewlessState';
import { SvgCanvas } from '@/diagram-svg/SvgCanvas';

/**
 * Renderer dispatcher. Picks a concrete renderer from the **payload shape** the
 * workspace store currently holds:
 *
 *   - tableModel    → TableView (TanStack Table)
 *   - geometryModel → GeometryView
 *   - treeModel     → BrowserView (native React tree)
 *   - graph / none  → SvgCanvas (React-SVG over the Rust ViewModel)
 *
 * The graph renderer is **SvgCanvas**. It fetches the renderer-neutral
 * ViewModel for the selected declared view; table/geometry/tree families use
 * the typed non-graph data carried by that same artifact.
 *
 * The store invariant (see `store/workspace.ts`) guarantees at most one
 * non-graph payload field is non-null at a time, so the dispatch is total.
 */
export function DiagramHost() {
  const tableModel = useWorkspaceStore((s) => s.tableModel);
  const geometryModel = useWorkspaceStore((s) => s.geometryModel);
  const treeModel = useWorkspaceStore((s) => s.treeModel);
  const selectedViewId = useWorkspaceStore((s) => s.selectedViewId);

  // View-less run (3.14): a live session whose run target has no declared view.
  // Rather than a synthesized diagram (views-first-class anti-pattern), offer the
  // "create a view" affordance. Only when there's no payload/view to show anyway.
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const activeSessionTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);
  const { data: views } = useViewsList(WORKSPACE_URI);
  const viewLessRun =
    !tableModel &&
    !geometryModel &&
    !treeModel &&
    !selectedViewId &&
    !!activeSessionId &&
    !!activeSessionTarget &&
    !findDeclaredViewForElement(activeSessionTarget, views ?? []);

  let renderer;
  if (tableModel) {
    renderer = <TableView />;
  } else if (geometryModel) {
    renderer = <GeometryView />;
  } else if (treeModel) {
    renderer = <BrowserView />;
  } else if (viewLessRun) {
    renderer = <CreateViewPrompt targetId={activeSessionTarget} />;
  } else if (!selectedViewId) {
    // First-class view-less state (W5 / F14): no session-specific target — the
    // workspace landing surface. Lists declared views (click renders) or, when
    // none exist (the common path), guides scratch-view creation.
    renderer = <ViewlessState />;
  } else {
    renderer = <SvgCanvas />;
  }

  return (
    <div className="flex flex-col h-full w-full overflow-hidden">
      <div className="flex-1 min-h-0 overflow-hidden">{renderer}</div>
    </div>
  );
}
