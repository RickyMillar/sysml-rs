/**
 * BreakpointRow — pure helpers.
 *
 * The row is a presentational component; rendering behaviour is covered
 * at the Playwright layer. Here we pin the `buildTooltip` contract so
 * the native title text stays stable as new extension fields land.
 */

import { describe, expect, it } from 'vitest';
import { buildTooltip } from '../BreakpointRow';

describe('buildTooltip', () => {
  it('covers element-kind breakpoints with kind + target', () => {
    const tip = buildTooltip({
      id: 'bp-1',
      breakpoint: { kind: 'state-entry', target: 'Engine.On' },
    });
    expect(tip).toContain('Kind: state-entry');
    expect(tip).toContain('Target: Engine.On');
  });

  it('covers threshold breakpoints with variable / threshold / direction', () => {
    const tip = buildTooltip({
      id: 'bp-2',
      breakpoint: {
        kind: 'threshold-crossing',
        target: 'v',
        variable: 'v',
        threshold: 5,
        direction: 'rising',
      },
    });
    expect(tip).toContain('Kind: threshold-crossing');
    expect(tip).toContain('Variable: v');
    expect(tip).toContain('Threshold: 5');
    expect(tip).toContain('Direction: rising');
  });

  it('defaults direction to "either" when absent', () => {
    const tip = buildTooltip({
      id: 'bp-3',
      breakpoint: {
        kind: 'threshold-crossing',
        target: 'x',
        variable: 'x',
        threshold: 1,
      },
    });
    expect(tip).toContain('Direction: either');
  });

  it('emits Round-4 extension fields when set', () => {
    const tip = buildTooltip({
      id: 'bp-4',
      breakpoint: { kind: 'state-entry', target: 'A' },
      condition: 'x > 5',
      hitCount: 3,
      logMessage: 'Trip',
    });
    expect(tip).toContain('Condition: x > 5');
    expect(tip).toContain('Hit count: 3');
    expect(tip).toContain('Log message: Trip');
  });

  it('flags soft-disabled rows', () => {
    const tip = buildTooltip({
      id: 'bp-5',
      breakpoint: { kind: 'state-entry', target: 'A' },
      enabled: false,
    });
    expect(tip).toContain('Disabled');
  });

  it('never surfaces extension fields when unset', () => {
    const tip = buildTooltip({
      id: 'bp-6',
      breakpoint: { kind: 'state-entry', target: 'A' },
    });
    expect(tip).not.toContain('Condition');
    expect(tip).not.toContain('Hit count');
    expect(tip).not.toContain('Log message');
    expect(tip).not.toContain('Disabled');
  });

  it('describes conditional breakpoints with target + op + value', () => {
    const tip = buildTooltip({
      id: 'bp-cond',
      breakpoint: {
        kind: 'conditional',
        target: 'circuit1',
        variable: 'voltage',
        op: 'gt',
        value: 12.0,
      },
    });
    expect(tip).toContain('Kind: conditional');
    expect(tip).toContain('Target: circuit1');
    expect(tip).toContain('Variable: voltage');
    expect(tip).toContain('Compare: gt 12');
  });

  it('surfaces debounce window on threshold-crossing when set', () => {
    const tip = buildTooltip({
      id: 'bp-debounce',
      breakpoint: {
        kind: 'threshold-crossing',
        target: 'i_total',
        variable: 'i_total',
        threshold: 32,
        direction: 'rising',
        debounce_ticks: 3,
      },
    });
    expect(tip).toContain('Debounce: 3 ticks');
  });

  it('omits the Debounce row when debounce_ticks is 0 / missing', () => {
    expect(
      buildTooltip({
        id: 'bp-no-debounce',
        breakpoint: {
          kind: 'threshold-crossing',
          target: 'v',
          variable: 'v',
          threshold: 5,
        },
      }),
    ).not.toContain('Debounce');
  });
});
