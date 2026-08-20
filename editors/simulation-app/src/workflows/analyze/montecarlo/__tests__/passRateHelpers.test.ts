/**
 * Tests for the pass-rate helpers (R5.8).
 *
 * Covers the contract the dashboard + CSV exporter lean on:
 *   - empty children → zeros and rate=0
 *   - all-pass → rate=1
 *   - AND semantics for overall pass-rate (a single fail ⇒ !allPass)
 *   - non-terminal children are excluded from the denominator so
 *     streaming updates don't drag the rate toward zero during runs
 */
import { describe, it, expect } from 'vitest';
import {
  computeOverallPassRate,
  computePassRate,
  __internals,
  type ChildDescriptor,
} from '../passRateHelpers';
import type { Verdict, VerdictKind } from '../../../../engine/types';

function v(id: string, kind: VerdictKind): Verdict {
  return { verdict: kind, id };
}

function child(
  index: number,
  status: ChildDescriptor['status'],
  verdicts: Verdict[] = [],
): ChildDescriptor {
  return { index, status, verdicts };
}

describe('computePassRate', () => {
  it('returns zeros for an empty child list', () => {
    const r = computePassRate([], 'c1');
    expect(r).toEqual({
      pass: 0,
      fail: 0,
      inconclusive: 0,
      error: 0,
      total: 0,
      passRate: 0,
    });
  });

  it('rate is 1 when every child passes', () => {
    const r = computePassRate(
      [
        child(0, 'complete', [v('c1', 'pass')]),
        child(1, 'complete', [v('c1', 'pass')]),
      ],
      'c1',
    );
    expect(r.pass).toBe(2);
    expect(r.total).toBe(2);
    expect(r.passRate).toBe(1);
  });

  it('mixed pass/fail counts separately', () => {
    const r = computePassRate(
      [
        child(0, 'complete', [v('c1', 'pass')]),
        child(1, 'complete', [v('c1', 'fail')]),
        child(2, 'complete', [v('c1', 'inconclusive')]),
        child(3, 'complete', [v('c1', 'error')]),
      ],
      'c1',
    );
    expect(r.pass).toBe(1);
    expect(r.fail).toBe(1);
    expect(r.inconclusive).toBe(1);
    expect(r.error).toBe(1);
    expect(r.total).toBe(4);
    expect(r.passRate).toBe(0.25);
  });

  it('excludes pending/running children from the denominator', () => {
    const r = computePassRate(
      [
        child(0, 'complete', [v('c1', 'pass')]),
        child(1, 'pending', [v('c1', 'pass')]), // ignored
        child(2, 'running', [v('c1', 'fail')]), // ignored
      ],
      'c1',
    );
    expect(r.total).toBe(1);
    expect(r.passRate).toBe(1);
  });

  it('ignores children with no verdict for the constraint', () => {
    const r = computePassRate(
      [
        child(0, 'complete', [v('other', 'pass')]),
        child(1, 'complete', [v('c1', 'pass')]),
      ],
      'c1',
    );
    expect(r.total).toBe(1);
  });
});

describe('computeOverallPassRate', () => {
  it('empty children + empty constraints → rate 0', () => {
    const r = computeOverallPassRate([], []);
    expect(r.rate).toBe(0);
    expect(r.total).toBe(0);
    expect(r.allPass).toBe(0);
  });

  it('AND semantics — one fail means overall fail', () => {
    const r = computeOverallPassRate(
      [
        child(0, 'complete', [v('c1', 'pass'), v('c2', 'pass')]),
        child(1, 'complete', [v('c1', 'pass'), v('c2', 'fail')]),
      ],
      ['c1', 'c2'],
    );
    expect(r.allPass).toBe(1);
    expect(r.anyFail).toBe(1);
    expect(r.total).toBe(2);
    expect(r.rate).toBe(0.5);
  });

  it('all-pass → rate 1', () => {
    const r = computeOverallPassRate(
      [
        child(0, 'complete', [v('c1', 'pass'), v('c2', 'pass')]),
        child(1, 'complete', [v('c1', 'pass'), v('c2', 'pass')]),
      ],
      ['c1', 'c2'],
    );
    expect(r.rate).toBe(1);
    expect(r.allPass).toBe(2);
    expect(r.anyFail).toBe(0);
  });

  it('inconclusive blocks allPass but does not flag anyFail', () => {
    const r = computeOverallPassRate(
      [child(0, 'complete', [v('c1', 'pass'), v('c2', 'inconclusive')])],
      ['c1', 'c2'],
    );
    expect(r.allPass).toBe(0);
    expect(r.anyFail).toBe(0);
    expect(r.total).toBe(1);
  });

  it('child missing a tracked verdict is counted but not evaluated', () => {
    const r = computeOverallPassRate(
      [child(0, 'complete', [v('c1', 'pass')])],
      ['c1', 'c2'],
    );
    expect(r.total).toBe(1);
    expect(r.evaluated).toBe(0);
    expect(r.allPass).toBe(0);
  });

  it('non-terminal children are excluded from total', () => {
    const r = computeOverallPassRate(
      [
        child(0, 'pending', [v('c1', 'pass')]),
        child(1, 'running', [v('c1', 'pass')]),
      ],
      ['c1'],
    );
    expect(r.total).toBe(0);
    expect(r.rate).toBe(0);
  });

  it('empty constraints just tallies terminal children', () => {
    const r = computeOverallPassRate(
      [
        child(0, 'complete'),
        child(1, 'failed'),
        child(2, 'pending'),
      ],
      [],
    );
    expect(r.total).toBe(2);
    expect(r.allPass).toBe(0);
  });
});

describe('__internals.verdictId', () => {
  it('prefers verdict.id', () => {
    expect(__internals.verdictId({ verdict: 'pass', id: 'foo' }, 0)).toBe('foo');
  });
  it('falls back to metadata.requirement_id', () => {
    expect(
      __internals.verdictId(
        { verdict: 'pass', metadata: { requirement_id: 'req.1' } },
        0,
      ),
    ).toBe('req.1');
  });
  it('falls back to metadata.case_name when no requirement_id', () => {
    expect(
      __internals.verdictId(
        { verdict: 'pass', metadata: { case_name: 'case.a' } },
        0,
      ),
    ).toBe('case.a');
  });
  it('synthesises an id from the index as last resort', () => {
    expect(__internals.verdictId({ verdict: 'pass' }, 7)).toBe('_verdict_7');
  });
});
