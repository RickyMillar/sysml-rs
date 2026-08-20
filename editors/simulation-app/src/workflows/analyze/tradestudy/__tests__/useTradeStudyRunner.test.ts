/**
 * Tests for useTradeStudyRunner — verifies the fan-out over the legacy
 * `sysml.trade_study` command, the per-row assembly, the weighted-sum
 * objective combination, and the idle/running/complete state machine.
 *
 * Backend-path note: R5.10 currently uses the legacy `sysml.trade_study`
 * command (AA2's `sysml.batch.create` not yet wired). These tests stub
 * the HTTP poster so they do not depend on either backend being up.
 */

import { describe, it, expect, vi } from 'vitest';
import {
  combineScore,
  overridesToTuples,
  runTradeStudyOnce,
  type TradeStudyRunConfig,
} from '../useTradeStudyRunner';
import type { CriterionConfig } from '../useTradeStudyConfig';

/** Matches the (unexported) `HttpPoster` alias in useTradeStudyRunner. */
type HttpPosterFn = <T>(path: string, body?: unknown) => Promise<T>;

function makeConfig(partial: Partial<TradeStudyRunConfig> = {}): TradeStudyRunConfig {
  const criteria: CriterionConfig[] = partial.criteria ?? [
    { metricId: 'score', objective: 'max' },
  ];
  return {
    studyName: partial.studyName ?? 'StudyX',
    alternatives: partial.alternatives ?? [
      { id: 'alt-1', label: 'A', overrides: { gain: 1 } },
      { id: 'alt-2', label: 'B', overrides: { gain: 2 } },
    ],
    criteria,
    weights: partial.weights ?? new Array(criteria.length).fill(1 / criteria.length),
  };
}

describe('overridesToTuples', () => {
  it('serialises numeric / string / boolean values', () => {
    const tuples = overridesToTuples({ a: 1, b: 'x', c: true });
    expect(tuples).toEqual([
      ['a', '1'],
      ['b', 'x'],
      ['c', 'true'],
    ]);
  });

  it('returns [] for undefined overrides', () => {
    expect(overridesToTuples(undefined)).toEqual([]);
  });
});

describe('combineScore', () => {
  const criteria: CriterionConfig[] = [
    { metricId: 'cost_usd', objective: 'min' },
    { metricId: 'performance_score', objective: 'max' },
  ];

  it('sign-flips Min criteria so higher combined = better', () => {
    // score map: cost=10, perf=3 → (-1 * 10 * 0.5) + (1 * 3 * 0.5) = -3.5
    const s = combineScore(
      { cost_usd: 10, performance_score: 3 },
      criteria,
      [0.5, 0.5],
    );
    expect(s).toBeCloseTo(-3.5, 10);
  });

  it('ignores non-finite score entries', () => {
    const s = combineScore(
      { cost_usd: NaN, performance_score: 2 },
      criteria,
      [0.5, 0.5],
    );
    // Only the performance term contributes.
    expect(s).toBeCloseTo(1, 10);
  });
});

describe('runTradeStudyOnce', () => {
  it('fans out over alternatives + preserves submission order', async () => {
    const calls: any[] = [];
    const posterImpl = async (path: string, body: any): Promise<unknown> => {
      calls.push({ path, body });
      // Return the alternative's own label as the match (simulates the
      // legacy command with per-alternative sub-scores baked in).
      const overrides: [string, string][] = body.params.overrides;
      const gain = Number(overrides.find(([k]) => k === 'gain')?.[1] ?? '0');
      const altName = gain === 1 ? 'A' : 'B';
      return {
        study_name: body.params.study_name,
        alternatives: [{ name: altName, score: gain * 10 }],
        best: altName,
        best_score: gain * 10,
      };
    };
    const poster = vi.fn(posterImpl) as unknown as HttpPosterFn;

    const out = await runTradeStudyOnce(makeConfig(), { poster });
    expect(out.rows).toHaveLength(2);
    expect(out.rows[0].label).toBe('A');
    expect(out.rows[0].score).toBe(10);
    expect(out.rows[1].label).toBe('B');
    expect(out.rows[1].score).toBe(20);
    expect(out.bestLabel).toBe('B');
    expect(out.bestScore).toBe(20);
    expect(out.criteria.map((c) => c.metricId)).toEqual(['score']);
    expect(out.weights).toEqual([1]);
    expect(calls).toHaveLength(2);
    expect(calls[0].path).toBe('/api/command');
    expect(calls[0].body.command).toBe('sysml.trade_study');
  });

  it('records errors per alternative without aborting the whole run', async () => {
    const posterImpl = async (_path: string, body: any): Promise<unknown> => {
      const overrides: [string, string][] = body.params.overrides;
      const gain = Number(overrides.find(([k]) => k === 'gain')?.[1] ?? '0');
      if (gain === 2) throw new Error('backend boom');
      return {
        study_name: body.params.study_name,
        alternatives: [{ name: 'A', score: 5 }],
        best: 'A',
        best_score: 5,
      };
    };
    const poster = vi.fn(posterImpl) as unknown as HttpPosterFn;

    const out = await runTradeStudyOnce(makeConfig(), { poster });
    expect(out.rows[0].error).toBeUndefined();
    expect(out.rows[0].score).toBe(5);
    expect(out.rows[1].error).toMatch(/backend boom/);
    expect(Number.isNaN(out.rows[1].score)).toBe(true);
    expect(out.bestLabel).toBe('A');
  });

  it('progress callback fires once per alternative in order', async () => {
    const posterImpl = async (_path: string, body: any): Promise<unknown> => ({
      study_name: body.params.study_name,
      alternatives: [{ name: 'A', score: 1 }],
      best: 'A',
      best_score: 1,
    });
    const poster = vi.fn(posterImpl) as unknown as HttpPosterFn;
    const events: string[] = [];
    await runTradeStudyOnce(makeConfig(), {
      poster,
      onProgress: (ev) => events.push(`${ev.index}:${ev.label}`),
    });
    expect(events).toEqual(['0:A', '1:B']);
  });

  it('aborts when the provided signal is already aborted', async () => {
    const poster = vi.fn();
    const ctl = new AbortController();
    ctl.abort();
    await expect(
      runTradeStudyOnce(makeConfig(), { poster: poster as any, signal: ctl.signal }),
    ).rejects.toMatchObject({ name: 'AbortError' });
    expect(poster).not.toHaveBeenCalled();
  });
});
