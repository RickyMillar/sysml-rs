/**
 * ConstraintChip rollup + presentation — punch-list finding 39.
 *
 * The chip is the surface that displayed "48 failing · 4/52", so it is the
 * one place where collapsing the four-valued verdict is most visible. Its
 * predecessor branched on a single boolean named `allPass` that actually
 * meant "nothing failed", which produced two opposite errors:
 *
 *   - 0 fail + N undecided rendered as the calm neutral chip, reporting
 *     constraints the run never evaluated as though all were well;
 *   - the moment anything failed, the undecided count disappeared from the
 *     label and the popover listed only failures.
 *
 * These pin both directions, plus the flattening the selector does.
 */
import { describe, it, expect } from 'vitest';
import { constraintChipView, constraintSummary, parseSummary } from '../KpiMeterRow';
import type { ConstraintVerdictKind } from '@/features/sessions/sessionLiveStore';


type Row = [string, ConstraintVerdictKind] | [string, ConstraintVerdictKind, string];

/** `[name, verdict, expression?]`. The name is deliberately allowed to be
 *  empty — that is the common case on real payloads, where only
 *  `assert constraint` usages are named. */
function snap(rows: Row[]) {
  return {
    snapshot: {
      constraint_results: rows.map(([name, verdict, expression]) => ({
        name,
        verdict,
        expression: expression ?? null,
      })),
    },
  };
}

describe('constraintSummary — rollup', () => {
  it('counts only decided failures as failing, and carries undecided names', () => {
    // The espresso shape: 3 pass, 1 fail, 7 inconclusive.
    const out = constraintSummary(
      snap([
        ['pressureEnvelope', 'Pass'],
        ['sourceEnvelope', 'Pass'],
        ['marginC >= 15.0', 'Pass'],
        ['temp band', 'Fail'],
        ...Array.from({ length: 7 }, (_, i): [string, ConstraintVerdictKind] => [
          `unbound${i}`,
          'Inconclusive',
        ]),
      ]),
    );
    const parsed = parseSummary(out)!;
    expect(parsed.pass).toBe(3);
    expect(parsed.total).toBe(11);
    expect(parsed.failing).toHaveLength(1);
    expect(parsed.undecided).toHaveLength(7);
  });

  it('groups Error with failing, not with undecided', () => {
    const parsed = parseSummary(
      constraintSummary(snap([['boom', 'Error'], ['maybe', 'Inconclusive']])),
    )!;
    expect(parsed.failing).toEqual(['boom']);
    expect(parsed.undecided).toEqual(['maybe']);
  });

  it('keeps names containing commas intact — expressions contain them', () => {
    const parsed = parseSummary(
      constraintSummary(
        snap([['equals(a, b)', 'Inconclusive'], ['size(x, y) > 0', 'Inconclusive']]),
      ),
    )!;
    expect(parsed.undecided).toEqual(['equals(a, b)', 'size(x, y) > 0']);
  });

  it('returns an empty summary when there are no rows', () => {
    expect(constraintSummary({ snapshot: { constraint_results: [] } })).toBe('');
    expect(constraintSummary({ snapshot: null })).toBe('');
  });

  // Regression: on the espresso workspace 9 of 11 constraints arrive with an
  // EMPTY name (only `assert constraint` usages are named; the
  // `require constraint` usages are anonymous and carry their text in
  // `expression`). A flattened list of empty names is indistinguishable from
  // no list at all, so the single unnamed FAILING row was silently dropped —
  // the chip read "3/11 · 7 undecided", 3 + 7 = 10, and the eleventh row
  // vanished. Counts must never be inferred from the name list.
  it('counts an unnamed failing row — an empty name is not an absent row', () => {
    const parsed = parseSummary(
      constraintSummary(
        snap([
          ['', 'Fail'],
          ['', 'Inconclusive'],
          ['named', 'Pass'],
        ]),
      ),
    )!;
    expect(parsed.failing).toHaveLength(1);
    expect(parsed.undecided).toHaveLength(1);
    expect(parsed.pass).toBe(1);
    expect(parsed.total).toBe(3);
  });

  it('falls back to the expression when a constraint has no name', () => {
    const parsed = parseSummary(
      constraintSummary(snap([['', 'Fail', 'temp >= 88.0 and temp <= 96.0']])),
    )!;
    expect(parsed.failing).toEqual(['temp >= 88.0 and temp <= 96.0']);
  });

  it('accepts either wire spelling — VerdictKind has two in this system', () => {
    // serde emits PascalCase on snapshots; `Display` emits lowercase on the
    // archive and CLI (finding 37). Three sibling consumers already fold; the
    // chip was the only one comparing exact PascalCase, so a lowercase row
    // would have matched no bucket and disappeared.
    const parsed = parseSummary(
      constraintSummary(
        snap([
          ['a', 'pass' as ConstraintVerdictKind],
          ['b', 'fail' as ConstraintVerdictKind],
          ['c', 'inconclusive' as ConstraintVerdictKind],
        ]),
      ),
    )!;
    expect(parsed.pass).toBe(1);
    expect(parsed.failing).toEqual(['b']);
    expect(parsed.undecided).toEqual(['c']);
  });

  it('buckets an unrecognised verdict as undecided rather than dropping it', () => {
    // Claiming nothing is the honest fallback, and it keeps the row counted.
    const parsed = parseSummary(
      constraintSummary(snap([['weird', 'Bogus' as ConstraintVerdictKind], ['ok', 'Pass']])),
    )!;
    expect(parsed.total).toBe(2);
    expect(parsed.undecided).toEqual(['weird']);
    expect(parsed.pass + parsed.failing.length + parsed.undecided.length).toBe(2);
  });

  it('every row lands in exactly one bucket — pass + failing + undecided === total', () => {
    const parsed = parseSummary(
      constraintSummary(
        snap([
          ['', 'Fail'],
          ['', 'Pass'],
          ['', 'Inconclusive'],
          ['', 'Error'],
          ['x', 'Pass'],
        ]),
      ),
    )!;
    expect(parsed.pass + parsed.failing.length + parsed.undecided.length).toBe(parsed.total);
  });
});

describe('constraintChipView — three states, never two', () => {
  it('keeps the undecided count in the label when something is ALSO failing', () => {
    // The regression: the label used to read "1 failing · 3/11" and drop the
    // seven undecided entirely — exactly when the user most needs the full
    // picture.
    const v = constraintChipView({
      pass: 3,
      total: 11,
      failing: ['temp band'],
      undecided: Array.from({ length: 7 }, (_, i) => `u${i}`),
    });
    expect(v.label).toBe('1 failing · 7 undecided · 3/11');
    expect(v.hasFailures).toBe(true);
    expect(v.hasUndecided).toBe(true);
    expect(v.accent).toBe('var(--verdict-fail)');
  });

  it('does NOT paint the neutral "all good" chip when rows are merely undecided', () => {
    // The opposite and more dangerous error: nothing failed, so the old flag
    // said `allPass` and the chip went neutral — silently reporting
    // unevaluated constraints as fine.
    const v = constraintChipView({ pass: 3, total: 10, failing: [], undecided: ['a', 'b'] });
    expect(v.accent).toBe('var(--verdict-inconclusive)');
    expect(v.border).toBe('var(--verdict-inconclusive)');
    expect(v.accent).not.toBe('var(--text-secondary)');
    expect(v.label).toBe('2 undecided · 3/10');
  });

  it('is neutral only when every constraint actually passed', () => {
    const v = constraintChipView({ pass: 4, total: 4, failing: [], undecided: [] });
    expect(v.accent).toBe('var(--text-secondary)');
    expect(v.border).toBe('var(--border-default)');
    expect(v.label).toBe('4/4 constraints');
    expect(v.hasFailures).toBe(false);
    expect(v.hasUndecided).toBe(false);
  });

  it('failure colour dominates undecided', () => {
    const v = constraintChipView({ pass: 0, total: 2, failing: ['x'], undecided: ['y'] });
    expect(v.accent).toBe('var(--verdict-fail)');
  });
});
