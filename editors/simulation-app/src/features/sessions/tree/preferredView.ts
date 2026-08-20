/**
 * Find the declared view that exposes a given element, if any.
 *
 * Bucket 5.R2 rewrite: this used to map a `ModelTreeNodeKind` to a
 * hardcoded `DiagramView` string (parts→IBD, states→StateTransition,
 * …) so that clicking a tree row would yank the diagram to whichever
 * view-kind preset best surfaced the clicked element. The
 * views-first-class roadmap rejects that — every diagram the editor
 * shows must be a *declared* view (a `ViewDefinition` / `ViewUsage`
 * the user authored). We search the live `sysml.query view-list` rows for one
 * whose `Expose` chain reaches the clicked ElementId.
 *
 * Returns `null` when no declared view exposes the element — in that
 * case, the caller should surface the "Create view for this element"
 * affordance (Bucket 5.A2) rather than silently falling back to a
 * default render. That fallback was the central anti-pattern Bucket 5
 * exists to remove.
 */

import type { ViewSummary } from '@/features/views/queries';

export function findDeclaredViewForElement(
  elementId: string | null | undefined,
  views: ViewSummary[],
): ViewSummary | null {
  if (!elementId) return null;
  for (const v of views) {
    for (const e of v.exposed) {
      if (e.exposed_element_id === elementId) return v;
    }
  }
  return null;
}
