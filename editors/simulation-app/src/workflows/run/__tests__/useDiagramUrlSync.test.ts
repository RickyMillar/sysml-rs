import { describe, expect, it } from 'vitest';
import {
  applyDiagramUrlParams,
  parseDiagramUrlParams,
} from '../useDiagramUrlSync';

describe('parseDiagramUrlParams', () => {
  it('returns null when no diagram keys are present', () => {
    expect(parseDiagramUrlParams('')).toBeNull();
    expect(parseDiagramUrlParams('?session=abc&tick=5')).toBeNull();
  });

  it('extracts the focused URI', () => {
    expect(parseDiagramUrlParams('?uri=file:%2F%2Ffoo.sysml')).toEqual({
      uri: 'file://foo.sysml',
      viewId: null,
    });
  });

  it('extracts the view id without validating it', () => {
    // Bucket 5: the URL carries `view_id=<ElementId>` instead of the
    // legacy `view=<kind>`. The parser is intentionally pass-through —
    // it isn't aware of which views exist; the consumer (ViewsPanel)
    // resolves the id and decides what to do if it's stale.
    expect(parseDiagramUrlParams('?view_id=abc-123')).toEqual({
      uri: null,
      viewId: 'abc-123',
    });
  });

  it('combines uri and view_id in a single read', () => {
    expect(
      parseDiagramUrlParams('?uri=file%3A%2F%2Fa.sysml&view_id=elem-7'),
    ).toEqual({ uri: 'file://a.sysml', viewId: 'elem-7' });
  });

  it('ignores the legacy ?view= param so old links no longer drive the renderer', () => {
    expect(parseDiagramUrlParams('?view=interconnection')).toBeNull();
  });
});

describe('applyDiagramUrlParams', () => {
  it('preserves unrelated keys', () => {
    const out = applyDiagramUrlParams(
      '?session=abc&tick=3',
      'file://x.sysml',
      'view-1',
    );
    expect(out).toContain('session=abc');
    expect(out).toContain('tick=3');
    expect(out).toContain('uri=file%3A%2F%2Fx.sysml');
    expect(out).toContain('view_id=view-1');
  });

  it('removes keys when values are null', () => {
    const out = applyDiagramUrlParams('?uri=old&view_id=v', null, null);
    expect(out).toBe('');
  });

  it('overwrites existing keys', () => {
    const out = applyDiagramUrlParams('?uri=old&view_id=v1', 'new', 'v2');
    const params = new URLSearchParams(out.startsWith('?') ? out.slice(1) : out);
    expect(params.get('uri')).toBe('new');
    expect(params.get('view_id')).toBe('v2');
  });

  it('returns the empty string when no params remain', () => {
    expect(applyDiagramUrlParams('', null, null)).toBe('');
  });
});
