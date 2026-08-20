/**
 * Bug B + C1 — pin the small helpers behind the fixes.
 *
 *  - `fileUriOf`: strips `file://`, returns null on missing span. The
 *    Bug B contract: ViewsPanel feeds this to `SourcePreviewPopover`
 *    so hover previews hit a real graph URI instead of the
 *    `__workspace__` sentinel.
 *  - `isStdlibView`: matches `/libraries/` somewhere in the span's
 *    file path. The Bug C1 contract: `useViewsList` /
 *    `useViewsByViewpoint` filter these out so the drawer stops
 *    listing 24 stdlib `ViewDefinition` types on every workspace.
 */

import { describe, expect, it } from 'vitest';
import { fileUriOf, isStdlibView, type ViewSummary } from '../queries';

function view(overrides: Partial<ViewSummary> = {}): ViewSummary {
  return {
    id: 'view-1',
    name: 'TestView',
    kind: 'ViewDefinition',
    exposed: [],
    renderings: [],
    filters: [],
    source_span: null,
    ...overrides,
  };
}

describe('fileUriOf', () => {
  it('strips file:// scheme', () => {
    const v = view({
      source_span: { file: 'file:///abs/path/Layout.sysml', start: 0, end: 1 },
    });
    expect(fileUriOf(v)).toBe('/abs/path/Layout.sysml');
  });

  it('returns the raw path when no scheme present', () => {
    const v = view({
      source_span: { file: '/abs/path/Layout.sysml', start: 0, end: 1 },
    });
    expect(fileUriOf(v)).toBe('/abs/path/Layout.sysml');
  });

  it('returns null when the view has no source span', () => {
    expect(fileUriOf(view({ source_span: null }))).toBeNull();
  });
});

describe('isStdlibView', () => {
  it('flags views declared under /libraries/', () => {
    const v = view({
      source_span: {
        file:
          'file:///abs/libraries/standard/library.systems/StandardViewDefinitions.sysml',
        start: 0,
        end: 1,
      },
    });
    expect(isStdlibView(v)).toBe(true);
  });

  it('keeps views declared under examples/', () => {
    const v = view({
      source_span: {
        file:
          'file:///abs/examples/espresso-production-cell/Structure/Layout.sysml',
        start: 0,
        end: 1,
      },
    });
    expect(isStdlibView(v)).toBe(false);
  });

  it('treats span-less views as non-library (lets them through)', () => {
    // Synthesised / inferred views with no recorded source span should
    // NOT be filtered out — they're often legitimate FE products.
    expect(isStdlibView(view({ source_span: null }))).toBe(false);
  });
});
