/**
 * `selectConstraintResults` — the non-live path feeding ConstraintCard.
 *
 * These pin the four-valued verdict surviving the projection. The card
 * resolves a row's verdict as `c.verdict ?? (c.pass ? 'pass' : 'fail')`, so a
 * producer that drops `verdict` silently renders every undecided constraint
 * as a violation — which is exactly what the backend change removed from the
 * wire, reintroduced one layer up. On the espresso workspace that showed as
 * "3 pass / 8 fail" where the truth is 3 pass / 1 fail / 7 inconclusive.
 */
import { describe, it, expect } from 'vitest';
import { selectConstraintResults } from '../selectors';
import type { SessionDetail } from '@/features/sessions/types';

function detailWith(rows: unknown[]): SessionDetail {
  return { latest_snapshot: { constraint_results: rows } } as unknown as SessionDetail;
}

describe('selectConstraintResults — verdict projection', () => {
  it('forwards all four verdicts, lowercased', () => {
    const out = selectConstraintResults(
      detailWith([
        { name: 'a', expression: 'x > 0', verdict: 'Pass' },
        { name: 'b', expression: 'x > 9', verdict: 'Fail' },
        { name: 'c', expression: 'y > 0', verdict: 'Inconclusive' },
        { name: 'd', expression: 'z > 0', verdict: 'Error' },
      ]),
    );
    expect(out.map((r) => r.verdict)).toEqual([
      'pass',
      'fail',
      'inconclusive',
      'error',
    ]);
  });

  it('does NOT collapse an undecided constraint into a failure', () => {
    const [row] = selectConstraintResults(
      detailWith([{ name: 'unbound', expression: 'q <= s', verdict: 'Inconclusive' }]),
    );
    // The load-bearing assertion: `pass: false` is correct (it is not a pass),
    // but the row must ALSO carry `inconclusive` so the consumer can tell
    // "not decided" from "decided against".
    expect(row.pass).toBe(false);
    expect(row.verdict).toBe('inconclusive');
    expect(row.verdict).not.toBe('fail');
  });

  it('leaves verdict undefined — and does not throw — on a verdict-less row', () => {
    // Backend/frontend version skew: an older backend still emitting the
    // removed `satisfied` shape. Degrade to the `pass` bool rather than
    // blowing up inside a render.
    const out = selectConstraintResults(
      detailWith([{ name: 'legacy', expression: 'x > 0', satisfied: true }]),
    );
    expect(out[0].verdict).toBeUndefined();
    expect(out[0].pass).toBe(false);
  });

  it('leaves verdict undefined on an unrecognised verdict string', () => {
    const out = selectConstraintResults(
      detailWith([{ name: 'weird', expression: 'x > 0', verdict: 'Bogus' }]),
    );
    expect(out[0].verdict).toBeUndefined();
  });

  it('returns an empty list when there is no snapshot', () => {
    expect(selectConstraintResults(undefined)).toEqual([]);
  });
});
