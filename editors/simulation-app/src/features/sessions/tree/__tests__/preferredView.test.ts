import { describe, expect, it } from 'vitest';
import { findDeclaredViewForElement } from '../preferredView';
import type { ViewSummary } from '@/features/views/queries';

function viewExposing(id: string, exposedIds: string[]): ViewSummary {
  return {
    id,
    name: id,
    kind: 'ViewDefinition',
    exposed: exposedIds.map((eid, i) => ({
      id: `${id}-expose-${i}`,
      is_namespace: false,
      qualified_name: null,
      exposed_element_id: eid,
    })),
    renderings: [],
    filters: [],
    source_span: null,
  };
}

describe('findDeclaredViewForElement', () => {
  it('returns null when no element is selected', () => {
    expect(findDeclaredViewForElement(null, [])).toBeNull();
    expect(findDeclaredViewForElement(undefined, [])).toBeNull();
    expect(findDeclaredViewForElement('', [])).toBeNull();
  });

  it('returns null when no declared view exposes the element', () => {
    // Bucket 5: when no view exposes the clicked element, the caller
    // is expected to surface "Create view for this element" rather
    // than fall back to a kind-based default render. Returning null
    // is the explicit "no answer" signal.
    const views = [viewExposing('view-a', ['part-1', 'part-2'])];
    expect(findDeclaredViewForElement('port-7', views)).toBeNull();
  });

  it('returns the first declared view exposing the element', () => {
    const views = [
      viewExposing('view-a', ['part-1', 'part-2']),
      viewExposing('view-b', ['part-3']),
    ];
    expect(findDeclaredViewForElement('part-1', views)?.id).toBe('view-a');
    expect(findDeclaredViewForElement('part-3', views)?.id).toBe('view-b');
  });

  it('finds the element among multiple Expose members of the same view', () => {
    const views = [viewExposing('multi', ['part-1', 'part-2', 'part-3'])];
    expect(findDeclaredViewForElement('part-2', views)?.id).toBe('multi');
  });

  it('respects declaration order when several views expose the same element', () => {
    // The first declared view wins — keeps tree-click determinism
    // alongside the ViewsPanel's own listing order.
    const views = [
      viewExposing('first', ['shared']),
      viewExposing('second', ['shared']),
    ];
    expect(findDeclaredViewForElement('shared', views)?.id).toBe('first');
  });
});
