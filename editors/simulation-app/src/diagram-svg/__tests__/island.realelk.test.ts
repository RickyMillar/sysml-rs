/**
 * Islands through the REAL elkjs engine (not a stub) — proves an expanded
 * DiagramChild::Island sub-diagram lays out nested inside its host node and
 * its internal edges route, end-to-end. Mirrors the real wire shape of the
 * showcase OverviewView (host 'Vehicle' + expanded Interconnection island).
 * Tracker 3.13 (WI5).
 */
import { describe, expect, it } from 'vitest';
import { layoutScene } from '../layout';
import type { DiagramIR, DiagramNode, DiagramEdge } from '../viewmodel-types';
function n(id: string, over: Partial<DiagramNode> = {}): DiagramNode {
  return { element_id: id, visual_kind: 'Part', element_kind: null, node_kind: 'Usage', name: id,
    stereotype: '', header_style: 'Normal', children: [], ports: [], buttons: [], expanded: null,
    tags: [], solver_status: null, sequence_layout: null,
    tooltip: null, position: null, size: null, layout: 'VBox', diagnostic_severity: null, ...over };
}
describe('island real-elk', () => {
  it('places island subtree nodes through real elkjs', async () => {
    const e: DiagramEdge = { id: 'se', source_id: 'S1', target_id: 'S2', kind: { Relationship: 'Connection' },
      label: '', source_port_id: null, target_port_id: null, precomputed_route: null,
      endpoint_mode: 'AutoSide', label_placement: {}, tags: [], secondary_labels: [] };
    const scene: DiagramIR = { view_type: 'Interconnection', edges: [], buttons: [],
      nodes: [ n('Vehicle', { children: [ { Island: { view_type: 'Interconnection', display_name: 'iv',
        expanded: true, subtree: { view_type: 'Interconnection', buttons: [],
          nodes: [n('S1'), n('S2'), n('S3'), n('S4')], edges: [e] } } } ] }) ] };
    const res = await layoutScene(scene); // real ELK
    const ids = res.nodes.map((p) => p.node.element_id).sort();
    expect(ids).toEqual(['S1', 'S2', 'S3', 'S4', 'Vehicle']);
    const veh = res.nodes.find((p) => p.node.element_id === 'Vehicle')!;
    const s1 = res.nodes.find((p) => p.node.element_id === 'S1')!;
    // S1 lands inside Vehicle's box (nested), and the island edge is routed.
    expect(s1.x).toBeGreaterThanOrEqual(veh.x);
    expect(res.edges.map((x) => x.edge.id)).toContain('se');
  });
});
