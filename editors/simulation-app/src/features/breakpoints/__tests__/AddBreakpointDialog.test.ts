/**
 * AddBreakpointDialog — pure validation / form-builder tests.
 *
 * Keeps the test surface node-only (no jsdom) by targeting the exported
 * pure helpers: `validateForm`, `defaultFormValues`. The interactive
 * aspects (Esc to close, focus trap, etc.) are covered by the Playwright
 * suite; this file pins the contract that the form correctly builds a
 * `Breakpoint` + `AdvancedFields` tuple for every kind.
 */

import { describe, expect, it } from 'vitest';
import {
  defaultFormValues,
  validateForm,
  COMPARE_OPS,
} from '../AddBreakpointDialog';

describe('defaultFormValues', () => {
  it('returns a blank form with the default kind', () => {
    const v = defaultFormValues();
    expect(v.kind).toBe('state-entry');
    expect(v.target).toBe('');
    expect(v.variable).toBe('');
    expect(v.threshold).toBe('');
    expect(v.direction).toBe('either');
    expect(v.condition).toBe('');
    expect(v.hitCount).toBe('');
    expect(v.logMessage).toBe('');
  });

  it('honours an explicit kind', () => {
    expect(defaultFormValues('threshold-crossing').kind).toBe('threshold-crossing');
  });

  it('seeds conditional defaults so the form renders without manual init', () => {
    const v = defaultFormValues('conditional');
    expect(v.kind).toBe('conditional');
    expect(v.compareOp).toBe('gt');
    expect(v.conditionalValue).toBe('');
    expect(v.debounceTicks).toBe('');
  });
});

describe('COMPARE_OPS dropdown entries', () => {
  it('covers the full 6-operator CompareOp enum', () => {
    expect(COMPARE_OPS.map((o) => o.value)).toEqual(['lt', 'le', 'gt', 'ge', 'eq', 'ne']);
  });
});

describe('validateForm — element kinds', () => {
  it('requires a non-blank target', () => {
    const result = validateForm({ ...defaultFormValues('state-entry'), target: '   ' });
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/target/i);
  });

  it('builds a state-entry breakpoint from a filled form', () => {
    const result = validateForm({
      ...defaultFormValues('state-entry'),
      target: 'Engine.Running',
    });
    expect(result.ok).toBe(true);
    expect(result.breakpoint).toEqual({
      kind: 'state-entry',
      target: 'Engine.Running',
    });
    expect(result.advanced).toEqual({});
  });

  it('builds a transition-fire breakpoint', () => {
    const result = validateForm({
      ...defaultFormValues('transition-fire'),
      target: 'T1',
    });
    expect(result.ok).toBe(true);
    expect(result.breakpoint?.kind).toBe('transition-fire');
  });

  it('builds an action-invoke breakpoint', () => {
    const result = validateForm({
      ...defaultFormValues('action-invoke'),
      target: 'Boot',
    });
    expect(result.ok).toBe(true);
    expect(result.breakpoint?.kind).toBe('action-invoke');
  });

  it('builds a constraint-violation breakpoint', () => {
    const result = validateForm({
      ...defaultFormValues('constraint-violation'),
      target: 'C1',
    });
    expect(result.ok).toBe(true);
    expect(result.breakpoint?.kind).toBe('constraint-violation');
  });
});

describe('validateForm — threshold-crossing', () => {
  it('requires a variable name', () => {
    const result = validateForm({
      ...defaultFormValues('threshold-crossing'),
      threshold: '5',
    });
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/variable/i);
  });

  it('rejects a non-numeric threshold', () => {
    const result = validateForm({
      ...defaultFormValues('threshold-crossing'),
      variable: 'I_total',
      threshold: 'abc',
    });
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/threshold/i);
  });

  it('builds a rising threshold breakpoint', () => {
    const result = validateForm({
      ...defaultFormValues('threshold-crossing'),
      variable: 'I_total',
      threshold: '32',
      direction: 'rising',
    });
    expect(result.ok).toBe(true);
    expect(result.breakpoint).toEqual({
      kind: 'threshold-crossing',
      target: 'I_total',
      variable: 'I_total',
      threshold: 32,
      direction: 'rising',
    });
  });

  it('defaults direction to "either" when left alone', () => {
    const result = validateForm({
      ...defaultFormValues('threshold-crossing'),
      variable: 'v',
      threshold: '0',
    });
    expect(result.ok).toBe(true);
    expect(result.breakpoint).toMatchObject({ direction: 'either' });
  });

  it('captures debounce_ticks when a positive integer is provided', () => {
    const result = validateForm({
      ...defaultFormValues('threshold-crossing'),
      variable: 'i_total',
      threshold: '32',
      direction: 'rising',
      debounceTicks: '3',
    });
    expect(result.ok).toBe(true);
    expect(result.breakpoint).toEqual({
      kind: 'threshold-crossing',
      target: 'i_total',
      variable: 'i_total',
      threshold: 32,
      direction: 'rising',
      debounce_ticks: 3,
    });
  });

  it('omits debounce_ticks when blank (preserves pre-R4.4 behaviour)', () => {
    const result = validateForm({
      ...defaultFormValues('threshold-crossing'),
      variable: 'x',
      threshold: '1',
    });
    expect(result.ok).toBe(true);
    expect(result.breakpoint).not.toHaveProperty('debounce_ticks');
  });

  it('rejects non-integer / negative debounce_ticks', () => {
    const base = {
      ...defaultFormValues('threshold-crossing'),
      variable: 'x',
      threshold: '1',
    };
    expect(validateForm({ ...base, debounceTicks: '2.5' }).ok).toBe(false);
    expect(validateForm({ ...base, debounceTicks: '-1' }).ok).toBe(false);
    expect(validateForm({ ...base, debounceTicks: 'abc' }).ok).toBe(false);
  });
});

describe('validateForm — conditional', () => {
  it('requires a target element', () => {
    const result = validateForm({
      ...defaultFormValues('conditional'),
      variable: 'voltage',
      conditionalValue: '12',
    });
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/element target/i);
  });

  it('requires a variable', () => {
    const result = validateForm({
      ...defaultFormValues('conditional'),
      target: 'circuit1',
      conditionalValue: '12',
    });
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/variable/i);
  });

  it('rejects non-numeric comparison values', () => {
    const result = validateForm({
      ...defaultFormValues('conditional'),
      target: 'circuit1',
      variable: 'voltage',
      conditionalValue: 'not-a-number',
    });
    expect(result.ok).toBe(false);
    expect(result.error).toMatch(/number/i);
  });

  it('produces a kebab-case `conditional` payload matching the Rust enum', () => {
    const result = validateForm({
      ...defaultFormValues('conditional'),
      target: 'circuit1',
      variable: 'voltage',
      compareOp: 'gt',
      conditionalValue: '12.0',
    });
    expect(result.ok).toBe(true);
    expect(result.breakpoint).toEqual({
      kind: 'conditional',
      target: 'circuit1',
      variable: 'voltage',
      op: 'gt',
      value: 12.0,
    });
  });

  it('accepts negative and zero comparison values', () => {
    expect(
      validateForm({
        ...defaultFormValues('conditional'),
        target: 'p',
        variable: 'x',
        compareOp: 'lt',
        conditionalValue: '-3.5',
      }),
    ).toMatchObject({ ok: true, breakpoint: { op: 'lt', value: -3.5 } });
    expect(
      validateForm({
        ...defaultFormValues('conditional'),
        target: 'p',
        variable: 'x',
        compareOp: 'eq',
        conditionalValue: '0',
      }),
    ).toMatchObject({ ok: true, breakpoint: { op: 'eq', value: 0 } });
  });
});

describe('validateForm — advanced fields', () => {
  it('omits advanced fields when every input is blank', () => {
    const result = validateForm({
      ...defaultFormValues('state-entry'),
      target: 'A',
    });
    expect(result.advanced).toEqual({});
  });

  it('captures condition / logMessage (trimmed) + hitCount (parsed)', () => {
    const result = validateForm({
      ...defaultFormValues('state-entry'),
      target: 'A',
      condition: '  x > 5 ',
      hitCount: '3',
      logMessage: '  hit  ',
    });
    expect(result.ok).toBe(true);
    expect(result.advanced).toEqual({
      condition: 'x > 5',
      hitCount: 3,
      logMessage: 'hit',
    });
  });

  it('rejects non-integer / non-positive hit counts', () => {
    expect(
      validateForm({
        ...defaultFormValues('state-entry'),
        target: 'A',
        hitCount: '0',
      }).ok,
    ).toBe(false);
    expect(
      validateForm({
        ...defaultFormValues('state-entry'),
        target: 'A',
        hitCount: '-1',
      }).ok,
    ).toBe(false);
    expect(
      validateForm({
        ...defaultFormValues('state-entry'),
        target: 'A',
        hitCount: '2.5',
      }).ok,
    ).toBe(false);
    expect(
      validateForm({
        ...defaultFormValues('state-entry'),
        target: 'A',
        hitCount: 'abc',
      }).ok,
    ).toBe(false);
  });
});
