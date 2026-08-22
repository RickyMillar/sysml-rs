/**
 * Renderer dispatcher for the standalone embed — the payload-shape dispatch of
 * the app's DiagramHost, minus its view-less/session arms.
 *
 * The embed always has a selected view (main.tsx seeds `selectedViewId` before
 * the first render), so DiagramHost's ViewlessState / CreateViewPrompt branches
 * are unreachable here — and deliberately not imported: that chain
 * (ViewlessState → ViewsPanel → … → MonacoSysmlEditor) is what dragged the
 * whole Monaco editor (~10 MB of module input plus its workers) into the spike
 * bundle. The concrete renderers are the app's own components, not copies.
 */
import { useWorkspaceStore } from '@/store/workspace';
import { TableView } from '@/components/diagram/TableView';
import { BrowserView } from '@/components/diagram/BrowserView';
import { GeometryView } from '@/components/diagram/GeometryView';
import { SvgCanvas } from '@/diagram-svg/SvgCanvas';

export function EmbedDiagramHost() {
  const tableModel = useWorkspaceStore((s) => s.tableModel);
  const geometryModel = useWorkspaceStore((s) => s.geometryModel);
  const treeModel = useWorkspaceStore((s) => s.treeModel);

  let renderer;
  if (tableModel) {
    renderer = <TableView />;
  } else if (geometryModel) {
    renderer = <GeometryView />;
  } else if (treeModel) {
    renderer = <BrowserView />;
  } else {
    renderer = <SvgCanvas />;
  }

  return (
    <div className="flex flex-col h-full w-full overflow-hidden">
      <div className="flex-1 min-h-0 overflow-hidden">{renderer}</div>
    </div>
  );
}
