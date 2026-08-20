/**
 * Tests for the MetricRegistry primitive (R1.7).
 *
 * Focus: the registry is the single seam for metric-aware consumers
 * (PlotsTab today; Sweep / Monte Carlo / KPI / verification later).
 * These tests pin the contract so producers and consumers can rely on
 * stable behaviour across rounds.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { MetricRegistry, metricRegistry, syncVariableMetrics } from '../registry';
import type { MetricDescriptor } from '../types';

function variableMetric(id: string, extras: Partial<MetricDescriptor> = {}): MetricDescriptor {
  return {
    id,
    name: id,
    source: 'variable',
    expression: id,
    ...extras,
  };
}

describe('MetricRegistry', () => {
  let reg: MetricRegistry;
  beforeEach(() => {
    reg = new MetricRegistry();
  });

  describe('register / list / get / unregister', () => {
    it('registers a descriptor and surfaces it via list() / get()', () => {
      const m = variableMetric('trip_time', { unit: 's', domain: 'protection' });
      reg.register(m);
      expect(reg.list()).toEqual([m]);
      expect(reg.get('trip_time')).toEqual(m);
      expect(reg.size).toBe(1);
    });

    it('register is idempotent by id — re-registering replaces the descriptor', () => {
      reg.register(variableMetric('T_busbar'));
      reg.register(variableMetric('T_busbar', { unit: 'K', domain: 'thermal' }));
      expect(reg.size).toBe(1);
      expect(reg.get('T_busbar')?.unit).toBe('K');
      expect(reg.get('T_busbar')?.domain).toBe('thermal');
    });

    it('list() preserves insertion order', () => {
      reg.register(variableMetric('c'));
      reg.register(variableMetric('a'));
      reg.register(variableMetric('b'));
      expect(reg.list().map((m) => m.id)).toEqual(['c', 'a', 'b']);
    });

    it('unregister removes a descriptor and is a no-op for unknown ids', () => {
      reg.register(variableMetric('a'));
      reg.register(variableMetric('b'));
      reg.unregister('a');
      reg.unregister('missing');
      expect(reg.list().map((m) => m.id)).toEqual(['b']);
    });

    it('get returns undefined when the id is absent', () => {
      expect(reg.get('nope')).toBeUndefined();
    });

    it('clear purges the registry', () => {
      reg.register(variableMetric('a'));
      reg.register(variableMetric('b'));
      reg.clear();
      expect(reg.size).toBe(0);
      expect(reg.list()).toEqual([]);
    });
  });

  describe('filter', () => {
    beforeEach(() => {
      reg.register(variableMetric('v1', { domain: 'electrical' }));
      reg.register(variableMetric('T1', { domain: 'thermal' }));
      reg.register({
        id: 'max_trip',
        name: 'max_trip',
        source: 'expression',
        expression: 'max(trip_time)',
        aggregator: 'max',
      });
    });

    it('returns only matching descriptors in insertion order', () => {
      expect(reg.filter((m) => m.source === 'variable').map((m) => m.id)).toEqual([
        'v1',
        'T1',
      ]);
      expect(reg.filter((m) => m.domain === 'thermal').map((m) => m.id)).toEqual([
        'T1',
      ]);
      expect(reg.filter((m) => m.source === 'expression').map((m) => m.id)).toEqual([
        'max_trip',
      ]);
    });

    it('returns an empty array when no metric matches', () => {
      expect(reg.filter((m) => m.domain === 'hydraulic')).toEqual([]);
    });
  });

  describe('syncVariableMetrics', () => {
    it('adds new variable metrics with source: variable', () => {
      syncVariableMetrics(reg, ['a', 'b']);
      expect(reg.list()).toEqual([
        variableMetric('a'),
        variableMetric('b'),
      ]);
    });

    it('applies the classifier when provided', () => {
      const classify = (n: string) => (n.startsWith('T_') ? 'thermal' : 'electrical');
      syncVariableMetrics(reg, ['T_1', 'V_1'], classify);
      expect(reg.get('T_1')?.domain).toBe('thermal');
      expect(reg.get('V_1')?.domain).toBe('electrical');
    });

    it('preserves existing variable-sourced metadata on re-sync', () => {
      reg.register(variableMetric('T_1', { unit: 'K', domain: 'thermal' }));
      syncVariableMetrics(reg, ['T_1']); // no classifier
      // Existing unit + domain should survive the sync (no-op for
      // already-registered variable sources).
      expect(reg.get('T_1')?.unit).toBe('K');
      expect(reg.get('T_1')?.domain).toBe('thermal');
    });

    it('evicts variable-sourced metrics that disappeared from the snapshot', () => {
      syncVariableMetrics(reg, ['a', 'b', 'c']);
      syncVariableMetrics(reg, ['a', 'c']);
      expect(reg.list().map((m) => m.id)).toEqual(['a', 'c']);
    });

    it('leaves expression / constraint sources untouched across syncs', () => {
      reg.register({
        id: 'expr1',
        name: 'expr1',
        source: 'expression',
        expression: 'max(a)',
      });
      syncVariableMetrics(reg, ['a', 'b']);
      // Both syncs — variables added, expression preserved.
      expect(reg.list().map((m) => m.id).sort()).toEqual(['a', 'b', 'expr1']);

      syncVariableMetrics(reg, []); // all variables gone
      expect(reg.list().map((m) => m.id)).toEqual(['expr1']);
    });
  });

  describe('module-level metricRegistry singleton', () => {
    it('is a MetricRegistry instance', () => {
      expect(metricRegistry).toBeInstanceOf(MetricRegistry);
    });

    it('starts empty by default', () => {
      // Defensive reset in case a sibling test leaked state.
      metricRegistry.clear();
      expect(metricRegistry.size).toBe(0);
    });
  });
});
