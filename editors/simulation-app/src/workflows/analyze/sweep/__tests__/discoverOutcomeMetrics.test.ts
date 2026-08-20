/**
 * Outcome discovery — the sweep learns what it can measure from the MODEL,
 * on the same terms as it learns what it can vary.
 *
 * Before this, outcomes came only from `metricRegistry`, a client-side
 * push-only catalogue written while a live session rendered in Plots. So the
 * Configure modal told you to "run the model once so its variables register"
 * — in a workflow whose whole job is choosing what to measure BEFORE you run
 * anything.
 *
 * Fixture shape mirrors `examples/radiation-cooling` as the live backend
 * returns it (measured 2026-08-19): five plain attributes and one `out`.
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';

const queryModel = vi.hoisted(() => vi.fn());
vi.mock('@/shared/api/model', () => ({ queryModel }));

import { discoverOutcomeMetrics } from '../discoverOutcomeMetrics';

/** `elements`-projection rows, exactly as the backend sends them. */
const RADIATION_ROWS = [
  { id: 'e1', name: 'emissivity', kind: 'AttributeUsage', props: { isDefault: true, value: 0.9 } },
  {
    id: 'e2',
    name: 'temperature',
    kind: 'AttributeUsage',
    props: { direction: 'out', isDefault: true, value: 1000.0 },
  },
  { id: 'e3', name: 'ambientTemp', kind: 'AttributeUsage', props: { isDefault: true, value: 300.0 } },
  {
    id: 'e4',
    name: 'thermalCapacity',
    kind: 'AttributeUsage',
    props: { isDefault: true, value: 500.0 },
  },
  {
    id: 'e5',
    name: 'stefanBoltzmann',
    kind: 'AttributeUsage',
    props: { isDefault: true, value: 5.67e-8 },
  },
  { id: 'e6', name: 'surfaceArea', kind: 'AttributeUsage', props: { isDefault: true, value: 0.1 } },
];

beforeEach(() => {
  queryModel.mockReset();
});

describe('discoverOutcomeMetrics', () => {
  it('finds the model outcome with no session having been run', async () => {
    queryModel.mockResolvedValue({ rows: RADIATION_ROWS });

    const metrics = await discoverOutcomeMetrics(['file:///RadiationCooling.sysml']);

    expect(metrics.map((m) => m.name)).toEqual(['temperature']);
    expect(metrics[0]).toMatchObject({
      id: 'temperature',
      source: 'variable',
      expression: 'temperature',
    });
  });

  it('leaves plain attributes out — those are knobs, not results', async () => {
    queryModel.mockResolvedValue({ rows: RADIATION_ROWS });
    const names = (await discoverOutcomeMetrics(['file:///x.sysml'])).map((m) => m.name);
    for (const knob of ['emissivity', 'surfaceArea', 'thermalCapacity', 'ambientTemp']) {
      expect(names).not.toContain(knob);
    }
  });

  it('treats `in` as an input and `inout` as an outcome', async () => {
    queryModel.mockResolvedValue({
      rows: [
        { id: 'a', name: 'drive', kind: 'AttributeUsage', props: { direction: 'in' } },
        { id: 'b', name: 'level', kind: 'AttributeUsage', props: { direction: 'inout' } },
      ],
    });
    const names = (await discoverOutcomeMetrics(['file:///x.sysml'])).map((m) => m.name);
    expect(names).toEqual(['level']);
  });

  it('asks per FILE uri, never the library-overlaid workspace graph', async () => {
    // A `__workspace__` query returns thousands of ISQ unit attributes
    // alongside the model's own — measured against the live backend.
    queryModel.mockResolvedValue({ rows: [] });
    await discoverOutcomeMetrics(['file:///a.sysml', 'file:///b.sysml']);

    expect(queryModel).toHaveBeenCalledTimes(2);
    for (const call of queryModel.mock.calls) {
      expect(call[0]).not.toBe('__workspace__');
      // The `summary` projection drops props, and `direction` lives in props.
      expect(call[1]).toMatchObject({ projection: 'elements' });
    }
  });

  it('dedupes by name across files', async () => {
    queryModel.mockResolvedValue({
      rows: [{ id: 'a', name: 'temperature', kind: 'AttributeUsage', props: { direction: 'out' } }],
    });
    const metrics = await discoverOutcomeMetrics(['file:///a.sysml', 'file:///b.sysml']);
    expect(metrics).toHaveLength(1);
  });

  it('keeps going when one uri cannot be queried', async () => {
    queryModel
      .mockRejectedValueOnce(new Error('parse failure'))
      .mockResolvedValueOnce({
        rows: [{ id: 'a', name: 'level', kind: 'AttributeUsage', props: { direction: 'out' } }],
      });
    const metrics = await discoverOutcomeMetrics(['file:///bad.sysml', 'file:///good.sysml']);
    expect(metrics.map((m) => m.name)).toEqual(['level']);
  });

  it('returns nothing for a model that declares no outputs', async () => {
    // Honest empty, not a prompt to go and run something first.
    queryModel.mockResolvedValue({
      rows: [{ id: 'a', name: 'gain', kind: 'AttributeUsage', props: { value: 2 } }],
    });
    expect(await discoverOutcomeMetrics(['file:///x.sysml'])).toEqual([]);
  });

  it('does not query at all with no loaded uris', async () => {
    expect(await discoverOutcomeMetrics([])).toEqual([]);
    expect(queryModel).not.toHaveBeenCalled();
  });
});
