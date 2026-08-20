/**
 * Pure-function tests for `formatCausationEvent` (R7.1). Exercises every
 * `CausationKind` variant so regressions in the label formatter surface
 * without a full panel mount.
 */

import { describe, expect, it } from 'vitest';
import type { CausationEvent } from '@/engine/types';
import {
  CAUSATION_KIND_ICON,
  CAUSATION_KIND_LABEL,
  formatCausationEvent,
  formatCausationEventPrefix,
} from '../formatCausationEvent';

function base(overrides: Partial<CausationEvent>): CausationEvent {
  // Defaults: a variable_write shape; tests override `kind` as needed.
  return {
    id: 'ev-1-0',
    tick: 1,
    actor: 'sm1',
    target: 'speed',
    detail: '',
    caused_by: [],
    kind: 'variable_write',
    var: 'speed',
    old_value: 0,
    new_value: 100,
    ...overrides,
  } as CausationEvent;
}

describe('formatCausationEvent', () => {
  it('renders variable_write with integer values', () => {
    const ev = base({ kind: 'variable_write', var: 'speed', old_value: 0, new_value: 100 });
    expect(formatCausationEvent(ev)).toBe('speed = 100 (was 0)');
  });

  it('renders variable_write with float values', () => {
    const ev = base({
      kind: 'variable_write',
      var: 'temp',
      old_value: 1.23456,
      new_value: 2.34567,
    });
    expect(formatCausationEvent(ev)).toBe('temp = 2.3457 (was 1.2346)');
  });

  it('renders variable_write with scientific notation for very small numbers', () => {
    const ev = base({
      kind: 'variable_write',
      var: 'x',
      old_value: 0,
      new_value: 1e-8,
    });
    expect(formatCausationEvent(ev)).toMatch(/x = 1\.000e-8/);
  });

  it('renders variable_write with boolean values', () => {
    const ev = base({
      kind: 'variable_write',
      var: 'active',
      old_value: true,
      new_value: false,
    });
    expect(formatCausationEvent(ev)).toBe('active = false (was true)');
  });

  it('renders variable_write with null values', () => {
    const ev = base({
      kind: 'variable_write',
      var: 'x',
      old_value: null,
      new_value: 1,
    });
    expect(formatCausationEvent(ev)).toBe('x = 1 (was null)');
  });

  it('renders transition_fire with triggering event', () => {
    const ev = base({
      kind: 'transition_fire',
      from: 'A',
      to: 'B',
      event: 'go',
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('sm1: A → B on `go`');
  });

  it('renders transition_fire without triggering event', () => {
    const ev = base({
      kind: 'transition_fire',
      from: 'A',
      to: 'B',
      event: null,
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('sm1: A → B');
  });

  it('renders action_invoke with args', () => {
    const ev = base({
      kind: 'action_invoke',
      action: 'resetCounter',
      args: ['0', 'true'],
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('resetCounter(0, true)');
  });

  it('renders action_invoke without args', () => {
    const ev = base({
      kind: 'action_invoke',
      action: 'doWork',
      args: [],
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('doWork');
  });

  it('renders constraint_evaluated pass', () => {
    const ev = base({
      kind: 'constraint_evaluated',
      constraint: 'speedLimit',
      verdict: true,
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('speedLimit: pass');
  });

  it('renders constraint_evaluated fail', () => {
    const ev = base({
      kind: 'constraint_evaluated',
      constraint: 'speedLimit',
      verdict: false,
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('speedLimit: FAIL');
  });

  it('renders event_injected with target', () => {
    const ev = base({
      kind: 'event_injected',
      event: 'go',
      target: 'go',
      actor: 'sm1',
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('inject `go` → go');
  });

  it('renders event_injected without target', () => {
    const ev = base({
      kind: 'event_injected',
      event: 'go',
      target: null,
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('inject `go`');
  });

  it('renders ode_step with few changed vars', () => {
    const ev = base({
      kind: 'ode_step',
      dt: 0.01,
      changed_vars: ['x', 'v'],
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('dt=0.0100s · x, v');
  });

  it('renders ode_step with many changed vars (truncated)', () => {
    const ev = base({
      kind: 'ode_step',
      dt: 0.005,
      changed_vars: ['a', 'b', 'c', 'd', 'e'],
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('dt=0.0050s · a, b, c, +2 more');
  });

  it('renders ode_step with no changed vars', () => {
    const ev = base({
      kind: 'ode_step',
      dt: 0.02,
      changed_vars: [],
    } as Partial<CausationEvent>);
    expect(formatCausationEvent(ev)).toBe('dt=0.0200s · no changes');
  });

  it('renders a tick·actor prefix', () => {
    expect(formatCausationEventPrefix(base({ tick: 42, actor: 'sm1' }))).toBe(
      't=42 · sm1',
    );
  });

  it('renders prefix with em-dash when actor is missing', () => {
    expect(formatCausationEventPrefix(base({ tick: 5, actor: '' }))).toBe(
      't=5 · —',
    );
  });

  it('provides a label + icon for every CausationKind', () => {
    const kinds = [
      'variable_write',
      'transition_fire',
      'action_invoke',
      'constraint_evaluated',
      'event_injected',
      'ode_step',
    ] as const;
    for (const k of kinds) {
      expect(CAUSATION_KIND_LABEL[k]).toBeTypeOf('string');
      expect(CAUSATION_KIND_ICON[k]).toBeTypeOf('string');
      expect(CAUSATION_KIND_LABEL[k]).not.toBe('');
      expect(CAUSATION_KIND_ICON[k]).not.toBe('');
    }
  });
});
