/**
 * Two-design-at-fixed-scenario mode (R4.3).
 *
 * UX goal: "design A" vs "design B" running the same input scenario
 * (same `overrides`). Different workspace_uri / label each. The UI
 * forces side-by-side layout and shows a third "delta" column
 * plotting A − B.
 *
 * Output: a "which design wins per metric" table — per variable we
 * compute ∫|A − B| dt (or rather ∑|A_i − B_i| · Δt_i, trapezoid would
 * be marginal improvement here) plus the peak delta and the tick at
 * which the peak occurred. The "worse" flag marks which of A/B had
 * the larger integrated contribution at that peak — this is a coarse
 * heuristic but surfaces the divergence the analyst cares about.
 *
 * Overrides-hash check: two-design assumes both sessions ran with the
 * same input scenario. If their override hashes differ we warn but
 * still compute — the analyst may have intentionally tweaked one.
 */
import { createElement } from 'react';
import type { ReactNode } from 'react';
import type { CompareContext, CompareMode } from '../compareMode';
import type { TimePoint } from '../../../features/sessions/types';
import type { Overrides } from '../../../engine/types';
import { isFiniteNumber, zipSeries } from './seriesAccess';

// ── Pure helpers ─────────────────────────────────────────────────────

/** Result of comparing two series for one variable. */
export interface DesignDeltaResult {
  /** ∫|A − B| over the union tick grid (trapezoidal with last-known-value fill). */
  integral: number;
  /** Maximum |A − B|. Zero when both series are identical or empty. */
  peakDelta: number;
  /** Tick at which the peak |A − B| occurred. Null when no evaluable samples. */
  peakTick: number | null;
  /** Count of union-grid ticks where both A and B were finite. */
  evaluatedTicks: number;
}

/**
 * Compute the integrated |A − B|, peak delta, and peak tick over the
 * union tick grid. Uses trapezoidal integration with LKV sampling —
 * good enough for a "which design wins" heuristic.
 *
 * Edge cases:
 *   - Empty series on either side → zeros with `peakTick: null`.
 *   - Non-finite samples at a tick are skipped (don't break the run).
 *   - Single-sample overlap → `integral: 0` (no dt to integrate over).
 */
export function compareDesignDelta(a: TimePoint[], b: TimePoint[]): DesignDeltaResult {
  const zipped = zipSeries(a, b);
  if (zipped.length === 0) {
    return { integral: 0, peakDelta: 0, peakTick: null, evaluatedTicks: 0 };
  }

  let integral = 0;
  let peakDelta = 0;
  let peakTick: number | null = null;
  let evaluatedTicks = 0;
  let prev: { t: number; absDiff: number } | null = null;

  for (const { t, a: av, b: bv } of zipped) {
    if (!isFiniteNumber(av) || !isFiniteNumber(bv)) {
      // Keep `prev` alive — integration skips the gap but resumes afterwards.
      continue;
    }
    evaluatedTicks += 1;
    const absDiff = Math.abs(av - bv);
    if (absDiff > peakDelta) {
      peakDelta = absDiff;
      peakTick = t;
    }
    if (prev != null) {
      const dt = t - prev.t;
      if (dt > 0) integral += 0.5 * (prev.absDiff + absDiff) * dt;
    }
    prev = { t, absDiff };
  }

  return { integral, peakDelta, peakTick, evaluatedTicks };
}

/**
 * Hash an overrides map deterministically. Used by two-design to warn
 * when the two sessions' inputs differ.
 *
 * Keys are sorted, values stringified with JSON. Not cryptographic —
 * just good enough for equality / inequality.
 */
export function hashOverrides(overrides: Overrides | null | undefined): string {
  if (!overrides) return '∅';
  const keys = Object.keys(overrides).sort();
  if (keys.length === 0) return '∅';
  const parts = keys.map((k) => `${k}=${JSON.stringify(overrides[k])}`);
  return parts.join('\u0001');
}

/** A named input to the two-design compare. */
export interface DesignSeries {
  /** Session id, used as id in the winner table. */
  sessionId: string;
  /** Human label ("Design A" / "Design B" or workspace label). */
  label: string;
  /** Variable name → TimePoint[]. */
  variables: Record<string, TimePoint[]>;
  /** Input overrides the session was started with. */
  overrides?: Overrides | null;
}

/** One row of the winner table. */
export interface DesignWinnerRow {
  variable: string;
  integral: number;
  peakDelta: number;
  peakTick: number | null;
  evaluatedTicks: number;
  /**
   * Which side had the larger value at the peak tick. `null` when
   * peak is zero (identical) or no evaluable ticks.
   */
  worseAt: 'A' | 'B' | null;
}

/** Whole-table output of the two-design analysis. */
export interface DesignWinnerTable {
  rows: DesignWinnerRow[];
  /** True when `hashOverrides` differs between A and B. */
  overridesMismatch: boolean;
  overridesHashA: string;
  overridesHashB: string;
}

/**
 * Find which side is "worse" at the peak-delta tick. Returns `'A'` when
 * A was larger (farther from the common story), `'B'` when B was
 * larger, `null` when both were equal or either sample wasn't finite.
 */
function worseSideAt(
  a: TimePoint[],
  b: TimePoint[],
  tick: number | null,
): 'A' | 'B' | null {
  if (tick == null) return null;
  const sampleA = sampleAtOrBefore(a, tick);
  const sampleB = sampleAtOrBefore(b, tick);
  if (!isFiniteNumber(sampleA) || !isFiniteNumber(sampleB)) return null;
  if (sampleA > sampleB) return 'A';
  if (sampleB > sampleA) return 'B';
  return null;
}

/** Small local LKV sampler — avoids a cross-module dep just for this one call. */
function sampleAtOrBefore(points: TimePoint[], tick: number): number | null {
  if (points.length === 0) return null;
  if (points[0].t > tick) return null;
  let lo = 0;
  let hi = points.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >>> 1;
    if (points[mid].t <= tick) lo = mid;
    else hi = mid - 1;
  }
  return points[lo].v;
}

/**
 * Build the "which design wins" table for the listed variables.
 *
 * `overridesMismatch` is informational — the analyst may have
 * intentionally altered one side. The shell surfaces the warning.
 */
export function buildDesignWinnerTable(
  a: DesignSeries,
  b: DesignSeries,
  variables: string[],
): DesignWinnerTable {
  const rows: DesignWinnerRow[] = [];
  for (const variable of variables) {
    const av = a.variables[variable] ?? [];
    const bv = b.variables[variable] ?? [];
    const delta = compareDesignDelta(av, bv);
    rows.push({
      variable,
      integral: delta.integral,
      peakDelta: delta.peakDelta,
      peakTick: delta.peakTick,
      evaluatedTicks: delta.evaluatedTicks,
      worseAt: worseSideAt(av, bv, delta.peakTick),
    });
  }
  // Sort descending by integral — biggest divergence first.
  rows.sort((x, y) => y.integral - x.integral);
  const hashA = hashOverrides(a.overrides);
  const hashB = hashOverrides(b.overrides);
  return {
    rows,
    overridesMismatch: hashA !== hashB,
    overridesHashA: hashA,
    overridesHashB: hashB,
  };
}

// ── React surface ────────────────────────────────────────────────────

function TwoDesignConfigPanel(ctx: CompareContext): ReactNode {
  const exactlyTwo = ctx.pickedSessionIds.length === 2;
  return createElement(
    'div',
    {
      'data-testid': 'two-design-config',
      style: {
        padding: 12,
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        fontSize: 12,
        color: 'var(--on-surface)',
      },
    },
    createElement('div', { style: { fontWeight: 600 } }, 'Two-design compare'),
    createElement(
      'div',
      { style: { opacity: 0.8 } },
      'Pick exactly two sessions (design A and design B). Same input scenario recommended.',
    ),
    createElement(
      'div',
      {
        'data-testid': 'two-design-selection-status',
        style: {
          marginTop: 4,
          padding: '4px 8px',
          borderRadius: 4,
          background: exactlyTwo
            ? 'color-mix(in srgb, var(--verdict-pass) 12%, transparent)'
            : 'color-mix(in srgb, var(--accent) 12%, transparent)',
          border: `1px solid color-mix(in srgb, ${exactlyTwo ? 'var(--verdict-pass)' : 'var(--accent)'} 35%, transparent)`,
        },
      },
      exactlyTwo
        ? 'Ready — 2 sessions selected'
        : `Selection: ${ctx.pickedSessionIds.length} / 2 sessions`,
    ),
    createElement(
      'div',
      { style: { opacity: 0.7, fontStyle: 'italic', marginTop: 6 } },
      'Layout is forced to side-by-side; a third "Δ" column plots A − B.',
    ),
  );
}

/**
 * The two-design mode. `mainRender` is omitted — the shell is
 * expected to switch its `layout` to `side-by-side` when this mode is
 * active and render a third column consuming `compareDesignDelta` +
 * `buildDesignWinnerTable` output.
 */
export const twoDesignMode: CompareMode = {
  id: 'two-design',
  label: 'Two-design',
  description: 'Design A vs Design B at a fixed input scenario — side-by-side + Δ column.',
  configRender: TwoDesignConfigPanel,
};
