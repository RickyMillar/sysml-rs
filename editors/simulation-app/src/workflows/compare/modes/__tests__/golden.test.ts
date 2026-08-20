/**
 * Tests for golden-baseline pure helpers.
 */
import { describe, it, expect } from 'vitest';
import type { TimePoint } from '../../../../features/sessions/types';
import {
  buildForkAtTickIntent,
  buildGoldenGridVerdicts,
  computeGoldenVerdict,
  goldenMode,
  resolveWindow,
  rollupPerSession,
  withinTolerance,
  type GoldenCandidate,
  type GoldenReference,
} from '../golden';

// ── Basic tolerance ──────────────────────────────────────────────────

describe('withinTolerance', () => {
  it('passes when equal', () => {
    expect(withinTolerance(10, 10, { kind: 'relative', value: 0.05 })).toBe(true);
  });

  it('passes within relative band', () => {
    expect(withinTolerance(100, 104, { kind: 'relative', value: 0.05 })).toBe(true);
  });

  it('fails outside relative band', () => {
    expect(withinTolerance(100, 106, { kind: 'relative', value: 0.05 })).toBe(false);
  });

  it('degrades to absolute when golden is ~0', () => {
    // Relative of 0.05 on a golden of 0 would be 0 → never pass.
    // We treat the 0.05 as an absolute fallback.
    expect(withinTolerance(0, 0.04, { kind: 'relative', value: 0.05 })).toBe(true);
    expect(withinTolerance(0, 0.06, { kind: 'relative', value: 0.05 })).toBe(false);
  });

  it('handles absolute tolerance', () => {
    expect(withinTolerance(100, 102, { kind: 'absolute', value: 3 })).toBe(true);
    expect(withinTolerance(100, 104, { kind: 'absolute', value: 3 })).toBe(false);
  });

  it('fails on non-finite inputs', () => {
    expect(withinTolerance(null, 10, { kind: 'absolute', value: 1 })).toBe(false);
    expect(withinTolerance(10, Number.NaN, { kind: 'absolute', value: 1 })).toBe(false);
  });
});

// ── Window resolution ────────────────────────────────────────────────

describe('resolveWindow', () => {
  const series: TimePoint[] = [
    { t: 0, v: 0 },
    { t: 10, v: 1 },
    { t: 20, v: 2 },
  ];

  it('returns null for empty series', () => {
    expect(resolveWindow([], { kind: 'whole-run' })).toBeNull();
  });

  it('resolves whole-run to [first, last]', () => {
    expect(resolveWindow(series, { kind: 'whole-run' })).toEqual({
      startTick: 0,
      endTick: 20,
    });
  });

  it('resolves last-fraction', () => {
    expect(resolveWindow(series, { kind: 'last-fraction', fraction: 0.5 })).toEqual({
      startTick: 10,
      endTick: 20,
    });
  });

  it('clamps last-fraction to [0, 1]', () => {
    expect(resolveWindow(series, { kind: 'last-fraction', fraction: -1 })).toEqual({
      startTick: 20,
      endTick: 20,
    });
    expect(resolveWindow(series, { kind: 'last-fraction', fraction: 99 })).toEqual({
      startTick: 0,
      endTick: 20,
    });
  });

  it('resolves explicit range, clipping to series bounds', () => {
    expect(resolveWindow(series, { kind: 'range', startTick: 5, endTick: 15 })).toEqual({
      startTick: 5,
      endTick: 15,
    });
    expect(resolveWindow(series, { kind: 'range', startTick: -10, endTick: 30 })).toEqual({
      startTick: 0,
      endTick: 20,
    });
  });
});

// ── Core verdict logic ───────────────────────────────────────────────

describe('computeGoldenVerdict', () => {
  const golden: TimePoint[] = [
    { t: 0, v: 100 },
    { t: 10, v: 100 },
    { t: 20, v: 100 },
  ];

  it('passes when every tick is within tolerance', () => {
    const cand: TimePoint[] = [
      { t: 0, v: 102 },
      { t: 10, v: 98 },
      { t: 20, v: 103 },
    ];
    const out = computeGoldenVerdict(cand, cand, { kind: 'relative', value: 0.05 });
    expect(out.verdict).toBe('pass');
    // Same series → zero delta on its own.
    expect(out.maxDelta).toBe(0);
    // compare cand vs golden:
    const out2 = computeGoldenVerdict(golden, cand, { kind: 'relative', value: 0.05 });
    expect(out2.verdict).toBe('pass');
    expect(out2.maxDelta).toBeCloseTo(3, 6);
  });

  it('fails when any tick is outside tolerance', () => {
    const cand: TimePoint[] = [
      { t: 0, v: 100 },
      { t: 10, v: 200 },
      { t: 20, v: 100 },
    ];
    const out = computeGoldenVerdict(golden, cand, { kind: 'relative', value: 0.05 });
    expect(out.verdict).toBe('fail');
    expect(out.maxDelta).toBe(100);
    expect(out.firstFailTick).toBe(10);
  });

  it('returns inconclusive when no overlap', () => {
    const out = computeGoldenVerdict(golden, [], { kind: 'relative', value: 0.05 });
    expect(out.verdict).toBe('inconclusive');
    expect(out.evaluatedTicks).toBe(0);
  });

  it('honours a compare window', () => {
    const cand: TimePoint[] = [
      { t: 0, v: 200 }, // outside tol, but also outside window
      { t: 10, v: 100 },
      { t: 20, v: 100 },
    ];
    const out = computeGoldenVerdict(
      golden,
      cand,
      { kind: 'absolute', value: 1 },
      { kind: 'range', startTick: 10, endTick: 20 },
    );
    expect(out.verdict).toBe('pass');
  });

  it('ignores non-finite samples', () => {
    const cand: TimePoint[] = [
      { t: 0, v: 100 },
      { t: 10, v: Number.NaN },
      { t: 20, v: 100 },
    ];
    const out = computeGoldenVerdict(golden, cand, { kind: 'absolute', value: 1 });
    expect(out.verdict).toBe('pass');
    expect(out.evaluatedTicks).toBe(2);
  });

  it('passes at the boundary (|Δ| == tolerance)', () => {
    const cand: TimePoint[] = [
      { t: 0, v: 101 },
      { t: 10, v: 101 },
      { t: 20, v: 101 },
    ];
    const out = computeGoldenVerdict(golden, cand, { kind: 'absolute', value: 1 });
    expect(out.verdict).toBe('pass');
  });

  it('fails just past the boundary', () => {
    const cand: TimePoint[] = [
      { t: 0, v: 101.0001 },
      { t: 10, v: 101.0001 },
      { t: 20, v: 101.0001 },
    ];
    const out = computeGoldenVerdict(golden, cand, { kind: 'absolute', value: 1 });
    expect(out.verdict).toBe('fail');
  });

  it("returns 'error' on negative tolerance (config error, not data fail)", () => {
    const cand: TimePoint[] = [
      { t: 0, v: 100 },
      { t: 10, v: 100 },
    ];
    const out = computeGoldenVerdict(golden, cand, { kind: 'absolute', value: -1 });
    expect(out.verdict).toBe('error');
    expect(out.errorReason).toBeTruthy();
  });

  it("returns 'error' on non-finite tolerance value", () => {
    const cand: TimePoint[] = [{ t: 0, v: 100 }];
    const out = computeGoldenVerdict(golden, cand, {
      kind: 'relative',
      value: Number.NaN,
    });
    expect(out.verdict).toBe('error');
  });

  it('tolerance-value = 0 is a strict-equality check (not an error)', () => {
    const cand: TimePoint[] = [
      { t: 0, v: 100 },
      { t: 10, v: 100 },
      { t: 20, v: 100 },
    ];
    const out = computeGoldenVerdict(golden, cand, { kind: 'absolute', value: 0 });
    expect(out.verdict).toBe('pass');
  });
});

// ── Grid-builder + rollup ────────────────────────────────────────────

describe('buildGoldenGridVerdicts', () => {
  const golden: GoldenReference = {
    sessionId: 'gold-1',
    variables: {
      V: [
        { t: 0, v: 10 },
        { t: 1, v: 10 },
      ],
      I: [
        { t: 0, v: 1 },
        { t: 1, v: 1 },
      ],
    },
  };

  const candidates: GoldenCandidate[] = [
    {
      sessionId: 'c-good',
      label: 'Candidate good',
      variables: {
        V: [
          { t: 0, v: 10 },
          { t: 1, v: 10.2 },
        ],
        I: [
          { t: 0, v: 1 },
          { t: 1, v: 1 },
        ],
      },
    },
    {
      sessionId: 'c-bad',
      label: 'Candidate bad',
      variables: {
        V: [
          { t: 0, v: 10 },
          { t: 1, v: 20 },
        ],
        I: [
          { t: 0, v: 1 },
          { t: 1, v: 1 },
        ],
      },
    },
  ];

  it('emits one verdict per (candidate, variable)', () => {
    const verdicts = buildGoldenGridVerdicts(
      golden,
      candidates,
      ['V', 'I'],
      { kind: 'relative', value: 0.05 },
    );
    expect(verdicts).toHaveLength(4);
    // Verdict metadata keys for PassFailGridViewer:
    for (const v of verdicts) {
      expect(v.metadata?.case_name).toBeDefined();
      expect(v.metadata?.requirement_id).toBeDefined();
    }
  });

  it('gets pass/fail right per case', () => {
    const verdicts = buildGoldenGridVerdicts(
      golden,
      candidates,
      ['V', 'I'],
      { kind: 'relative', value: 0.05 },
    );
    const good = verdicts.filter((v) => v.metadata?.case_name === 'Candidate good');
    const bad = verdicts.filter((v) => v.metadata?.case_name === 'Candidate bad');
    expect(good.every((v) => v.verdict === 'pass')).toBe(true);
    expect(bad.find((v) => v.metadata?.requirement_id === 'V')?.verdict).toBe('fail');
    expect(bad.find((v) => v.metadata?.requirement_id === 'I')?.verdict).toBe('pass');
  });

  it('supports per-variable tolerance overrides', () => {
    const verdicts = buildGoldenGridVerdicts(
      golden,
      candidates,
      ['V'],
      { kind: 'relative', value: 0.05 },
      { V: { kind: 'absolute', value: 50 } }, // giant tolerance
    );
    // Everyone passes with the per-variable override.
    expect(verdicts.every((v) => v.verdict === 'pass')).toBe(true);
  });
});

describe('rollupPerSession', () => {
  it('any fail → session fails', () => {
    const rollup = rollupPerSession([
      { verdict: 'pass', metadata: { case_name: 's1' } },
      { verdict: 'fail', metadata: { case_name: 's1' } },
      { verdict: 'pass', metadata: { case_name: 's2' } },
    ]);
    expect(rollup.get('s1')).toBe('fail');
    expect(rollup.get('s2')).toBe('pass');
  });

  it('any inconclusive in an otherwise-pass session → inconclusive', () => {
    const rollup = rollupPerSession([
      { verdict: 'pass', metadata: { case_name: 's1' } },
      { verdict: 'inconclusive', metadata: { case_name: 's1' } },
    ]);
    expect(rollup.get('s1')).toBe('inconclusive');
  });

  it('error beats everything (worst-wins precedence)', () => {
    const rollup = rollupPerSession([
      { verdict: 'pass', metadata: { case_name: 's1' } },
      { verdict: 'fail', metadata: { case_name: 's1' } },
      { verdict: 'error', metadata: { case_name: 's1' } },
    ]);
    expect(rollup.get('s1')).toBe('error');
  });
});

// ── Fork-at-tick gating ──────────────────────────────────────────────

describe('buildForkAtTickIntent', () => {
  it('emits fork-at-tick when backend supports it', () => {
    const intent = buildForkAtTickIntent('g1', 42, { has_fork_at_tick: true });
    expect(intent.kind).toBe('fork-at-tick');
    expect(intent.goldenSessionId).toBe('g1');
    expect(intent.tick).toBe(42);
  });

  it('falls back to snapshot when capability missing', () => {
    const intent = buildForkAtTickIntent('g1', 42, { has_fork_at_tick: false });
    expect(intent.kind).toBe('show-snapshot');
  });
});

describe('goldenMode registration', () => {
  it('has the stable id', () => {
    expect(goldenMode.id).toBe('golden');
  });
});
