/**
 * SessionBreadcrumb — thin store-connected wrapper around the pure
 * `<Breadcrumb>` component.
 *
 * Reads `focusPath` from the session store + resolves it to display
 * labels via the current model tree (from `useSessionModelTree`), then
 * calls `navigateFocusToDepth` on click. Phase B3 integration.
 */
import { useSessionStore } from '../store';
import { Breadcrumb } from './Breadcrumb';
import { useSessionModelTree } from './useSessionModelTree';
import { resolveFocusPath } from './buildModelTree';

export function SessionBreadcrumb() {
  const focusPath = useSessionStore((s) => s.focusPath);
  const navigateFocusToDepth = useSessionStore((s) => s.navigateFocusToDepth);
  const { tree } = useSessionModelTree();

  // Map ids → display labels via the live tree. Stale ids short-circuit
  // the chain (resolveFocusPath returns the best prefix); missing ids
  // just render a shorter breadcrumb rather than breaking.
  const nodes = resolveFocusPath(tree, focusPath);
  const segments = nodes.map((n) => ({ id: n.id, label: n.name }));

  return (
    <Breadcrumb
      segments={segments}
      onNavigateToDepth={navigateFocusToDepth}
      testIdPrefix="session-breadcrumb"
    />
  );
}
