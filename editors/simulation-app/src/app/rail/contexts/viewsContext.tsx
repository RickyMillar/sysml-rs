/**
 * views — right-rail context (ninebar Phase 1, plan §1 row 16 /
 * Phase 1 task "Re-home the always-on panels").
 *
 * Browse (the tree/packages/views/traceability reading surface) doesn't
 * exist as its own route until Phase 7 — per the plan's task list,
 * "packages/views/diagnostics/traceability panels stay reachable as
 * rail contexts until Phase 7 lands; do not strand them." This is one
 * of those interim homes: it re-homes the exact `ViewsPanel` the
 * `views` panel descriptor (`shared/panels/views.ts`) renders in the
 * old shell's utility drawer, unforked. `ViewsPanel` sets no fixed
 * width (it fills its container) and scrolls its list internally
 * (`styles.list` — `overflowY: 'auto'`), so it fits the rail cleanly.
 */
import { registerRailContext } from '../railRegistry';
import { ViewsPanel } from '@/features/views/ViewsPanel';

function ViewsRailContext() {
  return (
    <div
      data-testid="rail-context-views"
      className="flex flex-col h-full overflow-hidden"
    >
      <ViewsPanel />
    </div>
  );
}

registerRailContext({
  id: 'views',
  title: 'Views',
  icon: 'visibility',
  render: () => <ViewsRailContext />,
});
