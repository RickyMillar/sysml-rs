/**
 * Tests for the R5.11 pure trade-study helpers.
 *
 * Covers the four exported scoring primitives:
 *   - `isPareto` — dominance check vs a set
 *   - `computeParetoFront` — indices of non-dominated alternatives
 *   - `computeWeightedScore` — linear weighted sum with Min-negation
 *   - `rankAlternatives` — ordering (best first) with stable tie-break
 *
 * Plus the two convenience helpers (`buildAlternativeScores`,
 * `objectivesOf`) that the viewer relies on.
 */
import { describe, it, expect } from 'vitest';
import {
  buildAlternativeScores,
  computeParetoFront,
  computeWeightedScore,
  isPareto,
  objectivesOf,
  rankAlternatives,
  sumWeights,
  type AlternativeScore,
  type ChildDescriptorLike,
  type TradeCriterion,
  type TradeObjective,
} from '../tradeHelpers';

// ── Fixtures ────────────────────────────────────────────────────────

function alt(id: string, values: number[]): AlternativeScore {
  return { id, values };
}

/** cost ↓, throughput ↑ — canonical 2-objective trade study. */
const twoObjectives: TradeObjective[] = ['min', 'max'];

const twoCriteria: TradeCriterion[] = [
  { key: 'cost', objective: 'min', weight: 0.5 },
  { key: 'throughput', objective: 'max', weight: 0.5 },
];

// ── isPareto ────────────────────────────────────────────────────────

describe('isPareto', () => {
  it('marks a dominated alternative as non-Pareto', () => {
    const a = alt('A', [100, 50]); // cost=100, thr=50
    const b = alt('B', [80, 60]);  // cheaper AND faster → dominates A
    expect(isPareto(a, [a, b], twoObjectives)).toBe(false);
    expect(isPareto(b, [a, b], twoObjectives)).toBe(true);
  });

  it('treats two equal alternatives as mutually Pareto-optimal', () => {
    const a = alt('A', [100, 50]);
    const b = alt('B', [100, 50]);
    // Neither strictly dominates the other → both Pareto.
    expect(isPareto(a, [a, b], twoObjectives)).toBe(true);
    expect(isPareto(b, [a, b], twoObjectives)).toBe(true);
  });

  it('strict dominance requires strictly-better on at least one criterion', () => {
    // Equal cost, B strictly faster → B dominates A.
    const a = alt('A', [100, 50]);
    const b = alt('B', [100, 60]);
    expect(isPareto(a, [a, b], twoObjectives)).toBe(false);
    expect(isPareto(b, [a, b], twoObjectives)).toBe(true);
  });

  it('returns true for a solitary alternative (trivially optimal)', () => {
    const a = alt('A', [100, 50]);
    expect(isPareto(a, [a], twoObjectives)).toBe(true);
  });

  it('handles a 5-alternative mixed case', () => {
    // Draw the cost-throughput skyline:
    //   A (10, 10)  — dominated by C
    //   B (20, 40)  — Pareto
    //   C (5, 15)   — Pareto (cheap)
    //   D (30, 50)  — Pareto (fast)
    //   E (25, 40)  — dominated by B (same thr, costlier)
    const alts = [
      alt('A', [10, 10]),
      alt('B', [20, 40]),
      alt('C', [5, 15]),
      alt('D', [30, 50]),
      alt('E', [25, 40]),
    ];
    expect(isPareto(alts[0], alts, twoObjectives)).toBe(false); // A
    expect(isPareto(alts[1], alts, twoObjectives)).toBe(true);  // B
    expect(isPareto(alts[2], alts, twoObjectives)).toBe(true);  // C
    expect(isPareto(alts[3], alts, twoObjectives)).toBe(true);  // D
    expect(isPareto(alts[4], alts, twoObjectives)).toBe(false); // E
  });

  it('ignores criteria with non-finite values on either side', () => {
    const a = alt('A', [Number.NaN, 50]);
    const b = alt('B', [80, 60]);
    // B cannot dominate A on cost (NaN) — but B > A on throughput,
    // and the cost criterion is skipped → B dominates via throughput.
    expect(isPareto(a, [a, b], twoObjectives)).toBe(false);
  });
});

// ── computeParetoFront ─────────────────────────────────────────────

describe('computeParetoFront', () => {
  it('returns expected indices for a 3-criterion mixed case', () => {
    // Criteria: cost (min), throughput (max), weight (min)
    const objectives: TradeObjective[] = ['min', 'max', 'min'];
    const alts = [
      alt('0', [10, 100, 5]),  // Pareto (cheap, medium, light)
      alt('1', [20, 200, 10]), // Pareto (fast)
      alt('2', [15, 150, 8]),  // Pareto (balanced)
      alt('3', [20, 100, 10]), // dominated by 0 (costlier, same thr, heavier)
      alt('4', [30, 80, 12]),  // dominated by 0 on every criterion
    ];
    const front = computeParetoFront(alts, objectives);
    expect(front).toEqual([0, 1, 2]);
  });

  it('returns [0] for a single alternative', () => {
    expect(
      computeParetoFront([alt('only', [1, 2])], twoObjectives),
    ).toEqual([0]);
  });

  it('returns every index when all alternatives are mutually non-dominated', () => {
    const alts = [
      alt('A', [10, 10]),
      alt('B', [5, 5]),
      alt('C', [1, 1]),
    ];
    // cost ↓, thr ↑: cheaper rows are slower. All three on the frontier.
    expect(computeParetoFront(alts, twoObjectives)).toEqual([0, 1, 2]);
  });
});

// ── computeWeightedScore ───────────────────────────────────────────

describe('computeWeightedScore', () => {
  it('inverts Min objectives via negation', () => {
    // cost=100, weight=0.5, min → contribution = -50
    // throughput=200, weight=0.5, max → contribution = +100
    // total = +50
    const a = alt('A', [100, 200]);
    const score = computeWeightedScore(a, twoCriteria, twoObjectives);
    expect(score).toBeCloseTo(50);
  });

  it('leaves Max objectives unchanged (identity scaling)', () => {
    const maxOnlyCriteria: TradeCriterion[] = [
      { key: 'x', objective: 'max', weight: 1 },
    ];
    const s = computeWeightedScore(
      alt('A', [42]),
      maxOnlyCriteria,
      ['max'],
    );
    expect(s).toBe(42);
  });

  it('respects supplied weights (no auto-renormalisation)', () => {
    // Two Max criteria, weights 2 and 1. Raw [10, 5] → 2*10 + 1*5 = 25.
    const crit: TradeCriterion[] = [
      { key: 'a', objective: 'max', weight: 2 },
      { key: 'b', objective: 'max', weight: 1 },
    ];
    expect(
      computeWeightedScore(alt('x', [10, 5]), crit, ['max', 'max']),
    ).toBe(25);
  });

  it('returns 0 for a zero-criterion input', () => {
    expect(computeWeightedScore(alt('A', []), [], [])).toBe(0);
  });

  it('skips non-finite metrics (pending rows contribute 0)', () => {
    // throughput missing → contribution only from cost.
    const a = alt('A', [100, Number.NaN]);
    const score = computeWeightedScore(a, twoCriteria, twoObjectives);
    // Only cost*weight: -100*0.5 = -50.
    expect(score).toBeCloseTo(-50);
  });

  it('skips criteria with zero weight', () => {
    const crit: TradeCriterion[] = [
      { key: 'a', objective: 'max', weight: 0 },
      { key: 'b', objective: 'max', weight: 1 },
    ];
    expect(computeWeightedScore(alt('x', [999, 5]), crit, ['max', 'max'])).toBe(5);
  });
});

// ── rankAlternatives ───────────────────────────────────────────────

describe('rankAlternatives', () => {
  it('returns indices sorted best-first by weighted score', () => {
    // cost ↓, thr ↑, weights 0.5/0.5
    //   A score = -10*0.5 + 10*0.5 = 0
    //   B score = -5*0.5  + 20*0.5 = 7.5
    //   C score = -15*0.5 + 5*0.5  = -5
    const alts = [
      alt('A', [10, 10]),
      alt('B', [5, 20]),
      alt('C', [15, 5]),
    ];
    expect(
      rankAlternatives(alts, twoCriteria, twoObjectives),
    ).toEqual([1, 0, 2]);
  });

  it('breaks ties with a stable fallback to original index', () => {
    const alts = [
      alt('A', [10, 10]),
      alt('B', [10, 10]), // identical to A
      alt('C', [10, 10]), // identical to A
    ];
    expect(
      rankAlternatives(alts, twoCriteria, twoObjectives),
    ).toEqual([0, 1, 2]);
  });

  it('produces a permutation of input indices', () => {
    const alts = [
      alt('A', [1, 1]),
      alt('B', [2, 2]),
      alt('C', [3, 3]),
      alt('D', [4, 4]),
    ];
    const ranks = rankAlternatives(alts, twoCriteria, twoObjectives);
    expect(ranks.slice().sort((a, b) => a - b)).toEqual([0, 1, 2, 3]);
  });
});

// ── Convenience helpers ────────────────────────────────────────────

describe('buildAlternativeScores', () => {
  it('projects metrics in criterion order, NaN-filling missing keys', () => {
    const rows: ChildDescriptorLike[] = [
      {
        id: 'a',
        session_id: null,
        metrics: { cost: 100, throughput: 200 },
      },
      {
        id: 'b',
        session_id: 'sid-b',
        metrics: { cost: 80 }, // throughput missing → NaN
      },
    ];
    const out = buildAlternativeScores(rows, twoCriteria);
    expect(out[0].values).toEqual([100, 200]);
    expect(out[1].values[0]).toBe(80);
    expect(Number.isNaN(out[1].values[1])).toBe(true);
  });

  it('projects NaN for rows with no metrics object', () => {
    const rows: ChildDescriptorLike[] = [
      { id: 'a', session_id: null },
    ];
    const [only] = buildAlternativeScores(rows, twoCriteria);
    expect(only.values.length).toBe(2);
    expect(Number.isNaN(only.values[0])).toBe(true);
    expect(Number.isNaN(only.values[1])).toBe(true);
  });
});

describe('objectivesOf', () => {
  it('mirrors criteria array order', () => {
    expect(objectivesOf(twoCriteria)).toEqual(['min', 'max']);
  });
});

describe('sumWeights', () => {
  it('sums non-negative finite weights, skipping the rest', () => {
    const crit: TradeCriterion[] = [
      { key: 'a', objective: 'max', weight: 0.5 },
      { key: 'b', objective: 'max', weight: 0.5 },
      { key: 'c', objective: 'max', weight: Number.NaN },
      { key: 'd', objective: 'max', weight: -1 },
    ];
    expect(sumWeights(crit)).toBeCloseTo(1);
  });
});
