/**
 * PartDetail.Connections — `collectPartPorts` tests.
 *
 * The helper is pure: given a PartTreeNode and a snapshot's
 * port_values map, it returns the rows that belong to the focused
 * part, matching both fully-qualified (`ownerPath.name.port`) and
 * bare (`name.port`) key forms. Keeps the Connections rendering
 * predictable regardless of how the runtime labels port owners.
 */
import { describe, expect, it } from 'vitest';
import { collectPartPorts } from '../detail/PartDetail';
import type { PartTreeNode } from '../types';

function part(name: string, ownerPath = ''): PartTreeNode {
  return {
    id: `p-${name}`,
    elementId: `p-${name}`,
    uri: 'file:///w.sysml',
    name,
    rawKind: 'PartUsage',
    kind: 'part',
    depth: ownerPath ? ownerPath.split('.').length : 0,
    ownerPath,
    children: [],
  } as PartTreeNode;
}

describe('collectPartPorts', () => {
  it('returns [] when port_values is undefined', () => {
    expect(collectPartPorts(part('tank'), undefined)).toEqual([]);
  });

  it('picks ports matching bare instance name for root-level parts', () => {
    const rows = collectPartPorts(part('tank'), {
      'tank.waterOut': { flowRate: 1 },
      'tank.waterIn': { flowRate: 0.5 },
      'pump.inlet': { flowRate: 9 },
    });
    expect(rows.map((r) => r.portName)).toEqual(['waterIn', 'waterOut']);
  });

  it('picks ports matching fully-qualified ownerPath.name prefix', () => {
    const rows = collectPartPorts(part('GroupHead', 'ProductionCell.Station1'), {
      'ProductionCell.Station1.GroupHead.trip': { signal: 1 },
      'ProductionCell.Station2.GroupHead.trip': { signal: 0 },
    });
    expect(rows).toHaveLength(1);
    expect(rows[0].portName).toBe('trip');
    expect(rows[0].features).toEqual({ signal: 1 });
  });

  it('ignores keys belonging to a nested part (deeper dotting)', () => {
    // A port like "tank.subunit.leak" has one extra dot after the
    // match point — that's a nested part's port, not ours.
    const rows = collectPartPorts(part('tank'), {
      'tank.waterOut': { flowRate: 1 },
      'tank.subunit.leak': { flowRate: 0.01 },
    });
    expect(rows.map((r) => r.portName)).toEqual(['waterOut']);
  });

  it('sorts rows alphabetically by port name for stable rendering', () => {
    const rows = collectPartPorts(part('tank'), {
      'tank.zOut': { v: 1 },
      'tank.aOut': { v: 2 },
      'tank.mOut': { v: 3 },
    });
    expect(rows.map((r) => r.portName)).toEqual(['aOut', 'mOut', 'zOut']);
  });
});
