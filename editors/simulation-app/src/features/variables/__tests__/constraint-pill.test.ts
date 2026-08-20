/**
 * Constraint pill label/aria metadata — kept as a stable export for the
 * context menu + external consumers that need to reason about verdict
 * labels without instantiating the full `<VerdictBadge>`.
 *
 * The visual rendering of each verdict (colours, glyph shape, tooltip
 * wiring) lives in the shared `VerdictBadge` (R2.5) and is covered by
 * `src/components/__tests__/VerdictBadge.test.tsx`. The tests here only
 * pin the label + ariaLabel contract so context-menu / screen-reader
 * copy stays stable across re-renders.
 */

import { describe, it, expect } from 'vitest';
import { CONSTRAINT_PILL, pillForVerdict } from '../VariableRow';

describe('CONSTRAINT_PILL dictionary', () => {
  it('defines entries for every verdict kind', () => {
    expect(Object.keys(CONSTRAINT_PILL).sort()).toEqual([
      'error',
      'fail',
      'inconclusive',
      'pass',
    ]);
  });

  it('uses the canonical single-character glyphs (P, F, I, E)', () => {
    expect(CONSTRAINT_PILL.pass.label).toBe('P');
    expect(CONSTRAINT_PILL.fail.label).toBe('F');
    expect(CONSTRAINT_PILL.inconclusive.label).toBe('I');
    expect(CONSTRAINT_PILL.error.label).toBe('E');
  });

  it('exposes screen-reader labels that mention "constraint"', () => {
    for (const kind of ['pass', 'fail', 'inconclusive', 'error'] as const) {
      expect(CONSTRAINT_PILL[kind].ariaLabel.toLowerCase()).toContain('constraint');
    }
  });
});

describe('pillForVerdict', () => {
  it('returns null for absent constraint', () => {
    expect(pillForVerdict(undefined)).toBeNull();
  });

  it('returns the matching dictionary entry', () => {
    expect(pillForVerdict('pass')).toBe(CONSTRAINT_PILL.pass);
    expect(pillForVerdict('fail')).toBe(CONSTRAINT_PILL.fail);
  });
});
