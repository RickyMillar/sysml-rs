/**
 * Outcome metrics — a sweep can measure what the model declares, not just
 * what its verdicts imply.
 *
 * Before this, the viewer kit knew exactly two metrics, both derived from
 * verdicts (`fail_count`, `margin`). A study could SELECT `temperature` as an
 * outcome — the chip lit up, the store recorded it — and then every result
 * surface ignored it: no Table column, no Tornado entry, nothing to plot. The
 * selection had no consumer.
 *
 * The rules pinned here are the ones that make an outcome trustworthy once it
 * IS plotted: an unreadable outcome must never arrive as a number, and an
 * outcome must never be shadowed by a built-in that happens to share its name.
 */

import { describe, it, expect } from 'vitest';
import {
  collectOutcomeNames,
  extractorFor,
  metricLabelFor,
  metricOptionsFor,
  outcomeMetricId,
  outcomeNameFromMetricId,
  outcomeUnit,
  outcomeValue,
  type ChildDescriptor,
} from '../sweepViewerHelpers';

/** A child as the backend returns it once its run has been archived. */
function child(
  index: number,
  params: Record<string, unknown>,
  outcomes?: ChildDescriptor['outcomes'],
): ChildDescriptor {
  return {
    session_id: `s${index}`,
    index,
    params,
    status: 'complete',
    verdicts: [],
    ...(outcomes ? { outcomes } : {}),
  };
}

/** The measured shape of the five-point `ambientTemp` sweep. */
const SWEPT = [250, 275, 300, 325, 350].map((amb, i) =>
  child(i, { ambientTemp: amb }, {
    temperature: { value: 990.03 + i * 0.03, time_ms: 1000, unit: 'K' },
  }),
);

describe('outcome metric ids', () => {
  it('round-trips a name through its metric id', () => {
    expect(outcomeNameFromMetricId(outcomeMetricId('temperature'))).toBe('temperature');
  });

  it('does not mistake a built-in for an outcome', () => {
    expect(outcomeNameFromMetricId('fail_count')).toBeNull();
    expect(outcomeNameFromMetricId('margin')).toBeNull();
  });

  it('keeps an outcome named `margin` distinct from the built-in margin', () => {
    // Outcome names come from the model, so this collision is reachable by
    // writing `out attribute margin`. Namespacing is what stops the model's
    // own variable from being silently replaced by verdict arithmetic.
    const kids = [child(0, {}, { margin: { value: 7, time_ms: 10 } })];
    const asOutcome = extractorFor(outcomeMetricId('margin'))(kids[0]);
    const asBuiltin = extractorFor('margin')(kids[0]);
    expect(asOutcome).toBe(7);
    expect(asBuiltin).toBeNaN(); // no verdicts → no verdict margin
  });
});

describe('collectOutcomeNames', () => {
  it('finds the outcomes the children reported', () => {
    expect(collectOutcomeNames(SWEPT)).toEqual(['temperature']);
  });

  it('is streaming-safe — children with no outcomes yet contribute nothing', () => {
    const mixed = [
      { ...child(0, { a: 1 }), status: 'running' as const, outcomes: undefined },
      child(1, { a: 2 }, { temperature: { value: 5, time_ms: 1 } }),
    ];
    expect(collectOutcomeNames(mixed)).toEqual(['temperature']);
  });

  it('preserves first-appearance order across children', () => {
    const kids = [
      child(0, {}, { beta: { value: 1 } }),
      child(1, {}, { alpha: { value: 2 }, beta: { value: 3 } }),
    ];
    expect(collectOutcomeNames(kids)).toEqual(['beta', 'alpha']);
  });
});

describe('outcomeValue', () => {
  it('reads the measured value', () => {
    expect(outcomeValue(SWEPT[0], 'temperature')).toBeCloseTo(990.03);
  });

  it('is NaN — never 0 — when the outcome carries an error', () => {
    // This is the whole point. `0` is a plottable number and would put a
    // fabricated point on the chart at the axis origin; NaN is what every
    // helper in this kit already drops.
    const failed = child(0, {}, {
      temperature: { error: "'temperature' was not recorded by this run" },
    });
    expect(outcomeValue(failed, 'temperature')).toBeNaN();
    expect(outcomeValue(failed, 'temperature')).not.toBe(0);
  });

  it('is NaN when the outcome was never requested', () => {
    expect(outcomeValue(child(0, {}), 'temperature')).toBeNaN();
  });

  it('is NaN for a non-finite reading', () => {
    expect(outcomeValue(child(0, {}, { t: { value: Number.NaN } }), 't')).toBeNaN();
    expect(outcomeValue(child(0, {}, { t: { value: Infinity } }), 't')).toBeNaN();
  });

  it('reads a genuine zero as zero', () => {
    // The mirror of the rule above: a run that really settled at 0 must not
    // be rendered as unavailable.
    expect(outcomeValue(child(0, {}, { t: { value: 0, time_ms: 1 } }), 't')).toBe(0);
  });
});

describe('metricOptionsFor', () => {
  it('offers the verdict built-ins plus every measured outcome', () => {
    expect(metricOptionsFor(SWEPT).map((o) => o.value)).toEqual([
      'fail_count',
      'margin',
      'outcome:temperature',
    ]);
  });

  it('labels an outcome with its unit when the model declared one', () => {
    expect(metricOptionsFor(SWEPT).at(-1)?.label).toBe('temperature (K)');
  });

  it('omits the unit rather than inventing one', () => {
    // `temperature : ThermodynamicTemperatureValue` is a type-only ISQ
    // quantity: it has a dimension but no explicit unit symbol, so there is
    // nothing honest to print.
    const unitless = [child(0, {}, { temperature: { value: 990, time_ms: 1 } })];
    expect(outcomeUnit(unitless, 'temperature')).toBeUndefined();
    expect(metricLabelFor('outcome:temperature', unitless)).toBe('temperature');
  });

  it('still offers the built-ins when the batch measured nothing', () => {
    expect(metricOptionsFor([]).map((o) => o.value)).toEqual(['fail_count', 'margin']);
  });
});

describe('extractorFor', () => {
  it('drives the numeric viewers off the outcome across the sweep', () => {
    const extract = extractorFor(outcomeMetricId('temperature'));
    const values = SWEPT.map(extract);
    expect(values).toHaveLength(5);
    expect(values.every((v) => Number.isFinite(v))).toBe(true);
    // The five points genuinely differ — a spread the tornado can rank.
    expect(new Set(values).size).toBe(5);
  });

  it('leaves the built-ins behaving exactly as before', () => {
    const failing = {
      ...child(0, {}),
      verdicts: [
        { verdict: 'fail' as const, margin: 1.5 },
        { verdict: 'pass' as const, margin: 9 },
      ] as ChildDescriptor['verdicts'],
    };
    expect(extractorFor('fail_count')(failing)).toBe(1);
    expect(extractorFor('margin')(failing)).toBe(1.5);
  });
});
