import { describe, expect, it } from 'vitest';
import type { ElkNode } from 'elkjs';
import {
  __test,
  buildParentMap,
  layoutScene,
  orthogonalElbow,
  snapEdgeToPorts,
  type ElkLike,
} from '../layout';
import { colorsForVisualKind } from '../palette';
import { glyphSizeFor, isCardShape, shapeForVisualKind } from '../shapes';
import { edgeDecor } from '../edges';
import { adjustedEdgePoints, truncateScene } from '../SvgCanvas';
import { formatOverlayValue, isActive, isCompleted, overlayForNode } from '../overlay';
import type { PlacedEdge, PlacedNode } from '../layout';
import type { SimOverlay } from '../viewmodel-types';
import { elementAtOffset } from '@/features/diagram-link/store';
import type { DiagramEdge, DiagramIR, DiagramNode, EdgeStyleToken, Palette, TextMap } from '../viewmodel-types';

function node(id: string, over: Partial<DiagramNode> = {}): DiagramNode {
  return {
    element_id: id,
    visual_kind: 'Part',
    element_kind: null,
    node_kind: 'Usage',
    name: id,
    stereotype: '',
    header_style: 'Normal',
    children: [],
    ports: [],
    buttons: [],
    expanded: null,
    tags: [],
    solver_status: null,
    sequence_layout: null,
    tooltip: null,
    position: null,
    size: null,
    layout: 'VBox',
    diagnostic_severity: null,
    ...over,
  };
}

describe('partitionChildren', () => {
  it('splits child nodes from compartment text and flattens nested compartments', () => {
    const parent = node('P', {
      children: [
        { Node: node('child') },
        { Text: { compartment: 'Attributes', text: 'mass: Real', element_id: 'a1', source: 'Owned' } },
        {
          Compartment: {
            kind: 'Features',
            children: [
              { Text: { compartment: 'Features', text: 'speed', element_id: 'f1', source: 'Inherited' } },
            ],
          },
        },
      ],
    });
    const { childNodes, lines } = __test.partitionChildren(parent.children);
    expect(childNodes.map((c) => c.element_id)).toEqual(['child']);
    expect(lines.map((l) => l.text)).toEqual(['mass: Real', 'speed']);
    expect(lines[1].source).toBe('Inherited');
  });
});

describe('fixed islands keep their pre-solved geometry (D-S1)', () => {
  // Mirrors the real scratch/OverviewView payload: a `Part` host whose expanded
  // island is a SEQUENCE sub-diagram — every top-level node positioned, sized,
  // and its messages already routed by the Rust generator.
  const lifeline = (id: string, x: number) =>
    node(id, {
      visual_kind: 'Lifeline',
      name: `«part» ${id} : Type`,
      header_style: 'Inline',
      position: [x, 10],
      size: [282, 166],
      expanded: true,
      children: [
        { Node: node(`${id}/proxy`, { position: [138, 82], size: [6, 6], visual_kind: 'SqProxy' }) },
      ],
    });
  const island: DiagramIR = {
    view_type: 'Sequence',
    nodes: [lifeline('engine', 0), lifeline('gearbox', 400)],
    edges: [{
      id: 'msg0', source_id: 'engine/proxy', target_id: 'gearbox/proxy',
      kind: { Relationship: 'Connection' }, label: '', source_port_id: null,
      target_port_id: null, precomputed_route: [[141, 85], [513, 85]],
      endpoint_mode: 'AutoSide', label_placement: {}, tags: [], secondary_labels: [],
    }],
    buttons: [],
  };
  const scene: DiagramIR = {
    view_type: 'General',
    nodes: [node('Host', {
      expanded: true,
      children: [{ Node: node('ibdPart') }, { Island: { view_type: 'Sequence', display_name: 'sv', subtree: island, expanded: true } }],
    })],
    edges: [],
    buttons: [],
  };

  /** Places every child at its declared elk width/height, spread horizontally
   *  so sibling origins are distinguishable — enough to prove the spacer was
   *  sized and its interior re-anchored to where elk put the box. */
  const stub: ElkLike = {
    async layout(graph) {
      const place = (n: ElkNode, x: number): ElkNode => ({
        ...n, x, y: 20,
        width: n.width ?? 100, height: n.height ?? 40,
        children: (n.children ?? []).map((c, i) => place(c, 20 + i * 400)),
      });
      return { ...graph, width: 1600, height: 700, children: (graph.children ?? []).map((c) => place(c, 0)) };
    },
  };

  it('sizes the island spacer to the sub-diagram bounding box, not to elk-derived shapes', async () => {
    const res = await layoutScene(scene, stub);
    // The lifeline heads keep the authored 282×166 — flattening them into elk
    // ran the CONTAINER branch (they nest proxies), which ignores `node.size`
    // and collapsed them to ~56px, colliding with the host's IBD part boxes.
    const heads = res.nodes.filter((n) => n.node.visual_kind === 'Lifeline');
    expect(heads).toHaveLength(2);
    for (const h of heads) expect([h.width, h.height]).toEqual([282, 166]);
  });

  it('does not interleave island nodes with the host own children', async () => {
    const res = await layoutScene(scene, stub);
    const ibd = res.nodes.find((n) => n.node.element_id === 'ibdPart')!;
    const xs = res.nodes.filter((n) => n.node.visual_kind === 'Lifeline').map((n) => n.x);
    // Both lifelines land inside one contiguous island block offset by the
    // spacer origin, never bracketing the host's own part box.
    expect(Math.max(...xs) - Math.min(...xs)).toBe(400);
    expect(xs.every((x) => x !== ibd.x)).toBe(true);
  });

  it('keeps the precomputed message route instead of handing it to elk', async () => {
    const res = await layoutScene(scene, stub);
    const msg = res.edges.find((e) => e.edge.id === 'msg0')!;
    // Horizontal ladder rung preserved: elk never saw this edge.
    expect(msg.points).toHaveLength(2);
    expect(msg.points[0].y).toBe(msg.points[1].y);
    expect(msg.points[1].x - msg.points[0].x).toBe(513 - 141);
  });
});

describe('toElkNode container compartment band (#67)', () => {
  const topPad = (n: ElkNode): number => {
    const pad = (n.layoutOptions?.['elk.padding'] as string) ?? '';
    const m = pad.match(/top=(\d+)/);
    return m ? Number(m[1]) : NaN;
  };
  const container = (over: Partial<DiagramNode> = {}) =>
    node('C', { expanded: true, children: [{ Node: node('kid') }, ...(over.children ?? [])] });

  it('reserves vertical space for compartment rows in a container top padding', () => {
    const bare = __test.toElkNode(container(), 0, new Map());
    const withRows = __test.toElkNode(
      container({
        children: [
          { Text: { compartment: 'Attributes', text: 'mass: Real', element_id: 'a1', source: 'Owned' } },
          { Text: { compartment: 'Attributes', text: 'speed: Real', element_id: 'a2', source: 'Owned' } },
          { Text: { compartment: 'Attributes', text: 'power: Real', element_id: 'a3', source: 'Owned' } },
        ],
      }),
      0,
      new Map(),
    );
    // 3 compartment rows at LINE_H=16 + one PAD_Y band gap → children pushed
    // down so the attribute text never overlaps them (corePhysics overflow).
    expect(topPad(withRows)).toBeGreaterThan(topPad(bare));
    expect(topPad(withRows) - topPad(bare)).toBe(3 * 16 + 8);
  });
});

describe('layoutScene', () => {
  const scene: DiagramIR = {
    view_type: 'General',
    nodes: [node('A'), node('B')],
    edges: [{
      id: 'e1', source_id: 'A', target_id: 'B', kind: { Relationship: 'Specialization' },
      label: 'specializes', source_port_id: null, target_port_id: null,
      precomputed_route: null, endpoint_mode: 'AutoSide', label_placement: {},
      tags: [], secondary_labels: [],
    }],
    buttons: [],
  };

  // Stub ELK: positions A and B, routes the edge with one bend point.
  const stub: ElkLike = {
    async layout(graph) {
      return {
        ...graph,
        width: 200,
        height: 160,
        children: (graph.children ?? []).map((c, i) => ({ ...c, x: 10, y: 10 + i * 80, width: 120, height: 50 })),
        edges: [{
          id: 'e1', sources: ['A'], targets: ['B'],
          sections: [{ id: 's', startPoint: { x: 70, y: 60 }, endPoint: { x: 70, y: 90 }, bendPoints: [{ x: 70, y: 75 }] }],
        }],
      };
    },
  };

  it('resolves absolute node positions and edge route points', async () => {
    const res = await layoutScene(scene, stub);
    expect(res.width).toBe(200);
    expect(res.nodes.map((n) => n.node.element_id).sort()).toEqual(['A', 'B']);
    const b = res.nodes.find((n) => n.node.element_id === 'B')!;
    expect(b.y).toBe(90);
    expect(res.edges).toHaveLength(1);
    expect(res.edges[0].points).toEqual([
      { x: 70, y: 60 },
      { x: 70, y: 75 },
      { x: 70, y: 90 },
    ]);
    expect(res.edges[0].label).toBe('specializes');
  });

  it('accumulates parent offsets for nested (expanded) children', async () => {
    const nested: DiagramIR = {
      view_type: 'General',
      nodes: [node('Pkg', { expanded: true, children: [{ Node: node('Inner') }] })],
      edges: [],
      buttons: [],
    };
    const nestedStub: ElkLike = {
      async layout(graph) {
        const root = graph.children![0];
        return {
          ...graph,
          width: 300,
          height: 200,
          children: [{
            ...root,
            x: 20,
            y: 30,
            width: 160,
            height: 120,
            children: (root.children ?? []).map((c) => ({ ...c, x: 12, y: 40, width: 120, height: 50 })),
          }],
        };
      },
    };
    const res = await layoutScene(nested, nestedStub);
    const inner = res.nodes.find((n) => n.node.element_id === 'Inner')!;
    // absolute = parent (20,30) + child (12,40)
    expect(inner.x).toBe(32);
    expect(inner.y).toBe(70);
  });

  it('owns an EXPANDED container Edge child in its LCA container (D-N2 + brief §3.2)', async () => {
    // Behavior views nest transition/control-flow edges as DiagramChild::Edge
    // inside their container node — they must reach elk and the result. Under
    // the layout-quality brief §3.2 (LCA edge ownership) an edge between two
    // siblings of container SM is placed in SM's `edges[]`, NOT the root list,
    // so SEPARATE_CHILDREN + component packing can act on the rest of the graph
    // and SM sizes to hold its transition labels (brief §5 StateTransition).
    const behaviorEdge: DiagramEdge = {
      id: 't1', source_id: 's1', target_id: 's2', kind: { Transition: { trigger: 'go', guard: null } },
      label: '', source_port_id: null, target_port_id: null,
      precomputed_route: null, endpoint_mode: 'AutoSide', label_placement: {},
      tags: [], secondary_labels: [],
    } as unknown as DiagramEdge;
    const sceneWithNestedEdges: DiagramIR = {
      view_type: 'StateTransition',
      nodes: [
        node('SM', {
          expanded: true,
          children: [{ Node: node('s1') }, { Node: node('s2') }, { Edge: behaviorEdge }],
        }),
      ],
      edges: [],
      buttons: [],
    };
    let rootEdgeIds: string[] = [];
    let smEdgeIds: string[] = [];
    const edgeStub: ElkLike = {
      async layout(graph) {
        rootEdgeIds = (graph.edges ?? []).map((e) => e.id);
        const root = graph.children![0];
        smEdgeIds = (root.edges ?? []).map((e) => e.id);
        return {
          ...graph,
          width: 300,
          height: 200,
          children: [{
            ...root, x: 0, y: 0, width: 200, height: 150,
            children: (root.children ?? []).map((c, i) => ({ ...c, x: 10, y: 10 + i * 60, width: 100, height: 40 })),
            // Container edge coords are relative to SM's origin.
            edges: [{
              id: 't1', sources: ['s1'], targets: ['s2'],
              sections: [{ id: 's', startPoint: { x: 60, y: 50 }, endPoint: { x: 60, y: 70 } }],
            }],
          }],
          edges: [],
        };
      },
    };
    const res = await layoutScene(sceneWithNestedEdges, edgeStub);
    // The transition is owned by SM, not root.
    expect(rootEdgeIds).not.toContain('t1');
    expect(smEdgeIds).toContain('t1');
    // …and it still reaches the result (routed, coords lifted to absolute).
    expect(res.edges.map((e) => e.edge.id)).toContain('t1');
  });

  it('drops Edge children of a COLLAPSED container (endpoints not laid out)', async () => {
    const hiddenEdge: DiagramEdge = {
      id: 'hidden', source_id: 'x1', target_id: 'x2', kind: { Transition: { trigger: null, guard: null } },
      label: '', source_port_id: null, target_port_id: null,
      precomputed_route: null, endpoint_mode: 'AutoSide', label_placement: {},
      tags: [], secondary_labels: [],
    } as unknown as DiagramEdge;
    const sceneCollapsed: DiagramIR = {
      view_type: 'StateTransition',
      nodes: [
        node('SM', {
          expanded: false,
          children: [{ Node: node('x1') }, { Node: node('x2') }, { Edge: hiddenEdge }],
        }),
      ],
      edges: [],
      buttons: [],
    };
    let elkEdgeIds: string[] = ['sentinel'];
    const edgeStub: ElkLike = {
      async layout(graph) {
        elkEdgeIds = (graph.edges ?? []).map((e) => e.id);
        return { ...graph, width: 100, height: 50 };
      },
    };
    await layoutScene(sceneCollapsed, edgeStub);
    // Collapsed host → its child nodes aren't in the elk graph, so the edge
    // must not be either (an edge to a missing shape is a hard elk error).
    expect(elkEdgeIds).not.toContain('hidden');
  });

  it('seeds interactive re-layout with parent-relative positions (post-drop reflow)', async () => {
    const nested: DiagramIR = {
      view_type: 'General',
      nodes: [node('Pkg', { expanded: true, children: [{ Node: node('Inner') }] })],
      edges: [],
      buttons: [],
    };
    let captured: ElkNode | null = null;
    const capStub: ElkLike = {
      async layout(graph) {
        captured = graph;
        return { ...graph, width: 1, height: 1, children: [] };
      },
    };
    await layoutScene(nested, capStub, undefined, {
      interactive: true,
      positions: { Pkg: { x: 100, y: 50 }, Inner: { x: 130, y: 90 } },
    });
    expect(captured!.layoutOptions!['org.eclipse.elk.interactive']).toBe('true');
    expect(captured!.layoutOptions!['elk.layered.layering.strategy']).toBe('INTERACTIVE');
    const pkg = captured!.children![0];
    expect({ x: pkg.x, y: pkg.y }).toEqual({ x: 100, y: 50 }); // top-level: absolute
    const inner = pkg.children![0];
    expect({ x: inner.x, y: inner.y }).toEqual({ x: 30, y: 40 }); // 130-100, 90-50
  });

  it('lays out an expanded island sub-diagram nested inside its host', async () => {
    const islandEdge: DiagramEdge = {
      id: 'se1', source_id: 'S1', target_id: 'S2', kind: { Relationship: 'Succession' },
      label: 't', source_port_id: null, target_port_id: null, precomputed_route: null,
      endpoint_mode: 'AutoSide', label_placement: {}, tags: [], secondary_labels: [],
    };
    const scene: DiagramIR = {
      view_type: 'Interconnection',
      nodes: [
        node('Host', {
          children: [
            { Island: { view_type: 'StateTransition', display_name: 'SM', expanded: true,
                subtree: { view_type: 'StateTransition', nodes: [node('S1'), node('S2')], edges: [islandEdge], buttons: [] } } },
          ],
        }),
      ],
      edges: [],
      buttons: [],
    };
    // Generic stub: place every node, echo every edge with one section.
    let i = 0;
    const place = (n: ElkNode): ElkNode => ({
      ...n, x: 10, y: 10 + i++ * 40, width: 100, height: 30,
      children: (n.children ?? []).map(place),
    });
    const stub: ElkLike = {
      async layout(graph) {
        return {
          ...graph, width: 400, height: 400,
          children: (graph.children ?? []).map(place),
          edges: (graph.edges ?? []).map((e) => ({
            ...e, sections: [{ id: 's', startPoint: { x: 1, y: 2 }, endPoint: { x: 3, y: 4 }, bendPoints: [] }],
          })),
        };
      },
    };
    const res = await layoutScene(scene, stub);
    // The island's subtree nodes are placed alongside the host.
    expect(res.nodes.map((n) => n.node.element_id).sort()).toEqual(['Host', 'S1', 'S2']);
    // The island's internal edge participates in routing.
    expect(res.edges.map((e) => e.edge.id)).toEqual(['se1']);
    // The host is a container (so it renders with a body / header split).
    expect(res.nodes.find((n) => n.node.element_id === 'Host')!.hasChildren).toBe(true);
  });

  it('drops a COLLAPSED island sub-diagram', async () => {
    const scene: DiagramIR = {
      view_type: 'Interconnection',
      nodes: [
        node('Host', {
          children: [
            { Island: { view_type: 'StateTransition', display_name: 'SM', expanded: false,
                subtree: { view_type: 'StateTransition', nodes: [node('S1')], edges: [], buttons: [] } } },
          ],
        }),
      ],
      edges: [],
      buttons: [],
    };
    const stub: ElkLike = {
      async layout(graph) {
        return {
          ...graph, width: 100, height: 100,
          children: (graph.children ?? []).map((c) => ({ ...c, x: 0, y: 0, width: 100, height: 30 })),
        };
      },
    };
    const res = await layoutScene(scene, stub);
    expect(res.nodes.map((n) => n.node.element_id)).toEqual(['Host']);
    expect(res.nodes[0].hasChildren).toBe(false);
  });
});

describe('ports (IBD)', () => {
  const port = (id: string) => ({
    element_id: id, name: 'p', direction: null, is_conjugated: false, is_reference: false,
    tags: [], sub_ports: [], is_proxy: false, is_hidden: false, side: null, position: null, size: null,
  });
  const ibdScene: DiagramIR = {
    view_type: 'Interconnection',
    nodes: [node('A', { ports: [port('pa')] }), node('B', { ports: [port('pb')] })],
    edges: [{
      id: 'e', source_id: 'A', target_id: 'B', source_port_id: 'pa', target_port_id: 'pb',
      kind: { Relationship: 'Connection' }, label: '', precomputed_route: null,
      endpoint_mode: 'StrictPort', label_placement: {}, tags: [], secondary_labels: [],
    }],
    buttons: [],
  };

  it('routes edges to port ids and lifts elk port positions to absolute', async () => {
    let captured: ElkNode | null = null;
    const stub: ElkLike = {
      async layout(graph) {
        captured = graph;
        return {
          ...graph, width: 300, height: 100,
          children: (graph.children ?? []).map((c, i) => ({
            ...c, x: i * 120, y: 0, width: 100, height: 40,
            ports: (c.ports ?? []).map((p) => ({ ...p, x: 96, y: 16 })),
          })),
          edges: [{ id: 'e', sources: ['pa'], targets: ['pb'],
            sections: [{ id: 's', startPoint: { x: 96, y: 16 }, endPoint: { x: 216, y: 16 }, bendPoints: [] }] }],
        };
      },
    };
    const res = await layoutScene(ibdScene, stub);
    // collectEdges routed the edge to the *port* ids, not the node ids.
    expect(captured!.edges![0].sources).toEqual(['pa']);
    expect(captured!.edges![0].targets).toEqual(['pb']);
    // elk port coords (relative) are lifted to absolute: A@x0 → 96; B@x120 → 216.
    const a = res.nodes.find((n) => n.node.element_id === 'A')!;
    expect(a.ports).toHaveLength(1);
    expect(a.ports[0].x).toBe(96);
    expect(a.ports[0].port.element_id).toBe('pa');
    const b = res.nodes.find((n) => n.node.element_id === 'B')!;
    expect(b.ports[0].x).toBe(216);
  });

  it('omits hidden ports from layout', async () => {
    const scene: DiagramIR = {
      view_type: 'Interconnection',
      nodes: [node('A', { ports: [{ ...port('vis') }, { ...port('hid'), is_hidden: true }] })],
      edges: [], buttons: [],
    };
    let captured: ElkNode | null = null;
    const stub: ElkLike = {
      async layout(graph) { captured = graph; return { ...graph, width: 1, height: 1, children: [] }; },
    };
    await layoutScene(scene, stub);
    const portIds = (captured!.children![0].ports ?? []).map((p) => p.id);
    expect(portIds).toEqual(['vis']);
  });

  it('pins ports to their authored side (FIXED_SIDE) when the backend assigns one', async () => {
    const scene: DiagramIR = {
      view_type: 'Interconnection',
      nodes: [
        node('A', {
          ports: [
            { ...port('east'), side: 'East' },
            { ...port('west'), side: 'West' },
          ],
        }),
      ],
      edges: [], buttons: [],
    };
    let captured: ElkNode | null = null;
    const stub: ElkLike = {
      async layout(graph) { captured = graph; return { ...graph, width: 1, height: 1, children: [] }; },
    };
    await layoutScene(scene, stub);
    const a = captured!.children![0];
    expect(a.layoutOptions!['elk.portConstraints']).toBe('FIXED_SIDE');
    const byId = Object.fromEntries((a.ports ?? []).map((p) => [p.id, p]));
    expect(byId.east.layoutOptions!['elk.port.side']).toBe('EAST');
    expect(byId.west.layoutOptions!['elk.port.side']).toBe('WEST');
  });

  it('reserves + lifts elk port-label geometry to absolute (brief §2/§6 G8)', async () => {
    const scene: DiagramIR = {
      view_type: 'Interconnection',
      nodes: [node('A', { ports: [{ ...port('pa'), name: 'fuelIn', side: 'West' }] })],
      edges: [],
      buttons: [],
    };
    let captured: ElkNode | null = null;
    const stub: ElkLike = {
      async layout(graph) {
        captured = graph;
        return {
          ...graph, width: 200, height: 100,
          children: (graph.children ?? []).map((c) => ({
            ...c, x: 40, y: 20, width: 100, height: 40,
            // Port at the node's West face; elk returns a label box relative to
            // the port (label sizes in → label coords out).
            ports: (c.ports ?? []).map((p) => ({
              ...p, x: 0, y: 16,
              labels: [{ ...(p.labels ?? [])[0], x: -34, y: 0, width: 30, height: 10 }],
            })),
          })),
        };
      },
    };
    const res = await layoutScene(scene, stub);
    // The port carried a native label into elk for space reservation.
    const inPort = (captured!.children![0].ports ?? [])[0];
    expect(inPort.labels?.[0]).toMatchObject({ text: 'fuelIn', height: 10 });
    // …and the returned label box is lifted to absolute: node(40,20)+port(0,16)+label(-34,0).
    const a = res.nodes.find((n) => n.node.element_id === 'A')!;
    expect(a.ports[0].labelRect).toEqual({ x: 6, y: 36, width: 30, height: 10 });
  });

  it('reserves a wrapped elk edge-label + seeds the resolver anchor (brief §1/§6 G8)', async () => {
    const scene: DiagramIR = {
      view_type: 'StateTransition',
      nodes: [node('s1'), node('s2')],
      edges: [{
        id: 't', source_id: 's1', target_id: 's2',
        kind: { Transition: { trigger: 'when i_drive <= threshold', guard: 'armed' } },
        label: '', source_port_id: null, target_port_id: null, precomputed_route: null,
        endpoint_mode: 'AutoSide', label_placement: {}, tags: [], secondary_labels: [],
      }],
      buttons: [],
    };
    let captured: ElkNode | null = null;
    const stub: ElkLike = {
      async layout(graph) {
        captured = graph;
        return {
          ...graph, width: 200, height: 200,
          children: (graph.children ?? []).map((c, i) => ({ ...c, x: 10, y: 10 + i * 90, width: 100, height: 40 })),
          edges: (graph.edges ?? []).map((e) => ({
            ...e,
            // elk returns the inline-label box (root coords) after spreading.
            labels: [{ ...(e.labels ?? [])[0], x: 40, y: 90 }],
            sections: [{ id: 's', startPoint: { x: 60, y: 50 }, endPoint: { x: 60, y: 100 }, bendPoints: [] }],
          })),
        };
      },
    };
    const res = await layoutScene(scene, stub);
    // elk got a WRAPPED, sized label (long trigger+guard → multi-line chip).
    const elkLabel = (captured!.edges![0].labels ?? [])[0];
    expect(elkLabel.text).toBe('when i_drive <= threshold [armed]');
    expect(elkLabel.height).toBeGreaterThan(14); // wrapped → >1 line
    // The result seeds the FE resolver with elk's inline-label CENTER.
    const e = res.edges.find((x) => x.edge.id === 't')!;
    expect(e.labelAt).toEqual({ x: 40 + elkLabel.width! / 2, y: 90 + elkLabel.height! / 2 });
  });

  it('leaves ports FREE when no side is assigned', async () => {
    const scene: DiagramIR = {
      view_type: 'Interconnection',
      nodes: [node('A', { ports: [port('p')] })], // side: null
      edges: [], buttons: [],
    };
    let captured: ElkNode | null = null;
    const stub: ElkLike = {
      async layout(graph) { captured = graph; return { ...graph, width: 1, height: 1, children: [] }; },
    };
    await layoutScene(scene, stub);
    const a = captured!.children![0];
    expect(a.layoutOptions!['elk.portConstraints']).toBe('FREE');
    expect((a.ports ?? [])[0].layoutOptions).toBeUndefined();
  });
});

describe('snapEdgeToPorts (§2 port-center anchoring)', () => {
  const mk = (over: Partial<DiagramEdge>): PlacedEdge => ({
    edge: {
      id: 'e', source_id: 'A', target_id: 'B', source_port_id: 'pa', target_port_id: 'pb',
      kind: { Relationship: 'Connection' }, label: '', precomputed_route: null,
      endpoint_mode: 'StrictPort', label_placement: {}, tags: [], secondary_labels: [], ...over,
    },
    points: [{ x: 95, y: 18 }, { x: 150, y: 18 }, { x: 150, y: 60 }, { x: 205, y: 62 }],
    label: '', labelAt: null,
  });

  it('pins first/last points to port centers and re-squares the adjacent bend', () => {
    const pe = mk({});
    const centers = new Map([
      ['pa', { x: 96, y: 16 }],
      ['pb', { x: 208, y: 64 }],
    ]);
    snapEdgeToPorts(pe, centers);
    expect(pe.points[0]).toEqual({ x: 96, y: 16 });
    expect(pe.points[pe.points.length - 1]).toEqual({ x: 208, y: 64 });
    // First segment is horizontal (bend.y realigned to the port center y).
    expect(pe.points[1].y).toBe(16);
    // Last segment is horizontal (bend.y realigned to the target port y).
    expect(pe.points[2].y).toBe(64);
  });

  it('leaves non-ported endpoints untouched', () => {
    const pe = mk({ source_port_id: null, target_port_id: null });
    const before = pe.points.map((p) => ({ ...p }));
    snapEdgeToPorts(pe, new Map());
    expect(pe.points).toEqual(before);
  });
});

describe('fixed layout (sequence/grid/geometry)', () => {
  // Two lifelines, each owning two occurrence proxies (parent-relative coords),
  // plus a forward + a return message with precomputed routes. Mirrors the
  // backend SequenceView output shape (proxy y omits the lifeline y-offset).
  const seqScene: DiagramIR = {
    view_type: 'Sequence',
    nodes: [
      node('lifeline:server', {
        visual_kind: 'Lifeline', name: 'server', header_style: 'Inline',
        position: [0, 10], size: [80, 166],
        children: [
          { Node: node('proxy:0:src', { visual_kind: 'SqProxy', name: '', header_style: 'None', position: [37, 82], size: [6, 6] }) },
          { Node: node('proxy:1:tgt', { visual_kind: 'SqProxy', name: '', header_style: 'None', position: [37, 124], size: [6, 6] }) },
        ],
      }),
      node('lifeline:client', {
        visual_kind: 'Lifeline', name: 'client', header_style: 'Inline',
        position: [160, 10], size: [80, 166],
        children: [
          { Node: node('proxy:0:tgt', { visual_kind: 'SqProxy', name: '', header_style: 'None', position: [37, 82], size: [6, 6] }) },
          { Node: node('proxy:1:src', { visual_kind: 'SqProxy', name: '', header_style: 'None', position: [37, 124], size: [6, 6] }) },
        ],
      }),
    ],
    edges: [
      {
        id: 'message:0', source_id: 'proxy:0:src', target_id: 'proxy:0:tgt',
        kind: { Message: { payload: 'reqOut', is_succession: false, is_move: true, is_push: true } },
        label: '', source_port_id: null, target_port_id: null,
        precomputed_route: [[40, 85], [200, 85]], endpoint_mode: 'AutoSide', label_placement: {},
        tags: [], secondary_labels: [],
      },
      {
        id: 'message:1', source_id: 'proxy:1:src', target_id: 'proxy:1:tgt',
        kind: { Message: { payload: 'respOut', is_succession: false, is_move: true, is_push: true } },
        label: '', source_port_id: null, target_port_id: null,
        precomputed_route: [[200, 127], [40, 127]], endpoint_mode: 'AutoSide', label_placement: {},
        tags: ['Return'], secondary_labels: [],
      },
    ],
    buttons: [],
  };

  it('honors precomputed positions without elk (throwing stub never called)', async () => {
    const explode: ElkLike = { async layout() { throw new Error('elk must not run for a fixed scene'); } };
    const res = await layoutScene(seqScene, explode);
    // 2 lifelines + 4 proxies, all placed.
    expect(res.nodes).toHaveLength(6);
    const server = res.nodes.find((n) => n.node.element_id === 'lifeline:server')!;
    expect([server.x, server.y, server.width, server.height]).toEqual([0, 10, 80, 166]);
  });

  it('resolves child proxy positions to absolute (parent-relative origin)', async () => {
    const res = await layoutScene(seqScene);
    // client lifeline at (160,10); proxy:0:tgt local (37,82) → absolute (197,92).
    const tgt = res.nodes.find((n) => n.node.element_id === 'proxy:0:tgt')!;
    expect([tgt.x, tgt.y]).toEqual([197, 92]);
  });

  it('re-anchors message routes onto the true source-proxy center (fixes the lifeline y-offset)', async () => {
    const res = await layoutScene(seqScene);
    const m0 = res.edges.find((e) => e.edge.id === 'message:0')!;
    // source proxy:0:src absolute center: (0+37+3, 10+82+3) = (40,95). The route
    // y (85) is shifted by +10 so both endpoints land on the proxy centers.
    expect(m0.points).toEqual([{ x: 40, y: 95 }, { x: 200, y: 95 }]);
  });
});

describe('palette mapping', () => {
  const palette = {
    block: { fill: 'oklch(94% 0.04 155)', stroke: 'oklch(55% 0.14 155)', header: null },
    state: { fill: 'oklch(94% 0.05 55)', stroke: 'oklch(60% 0.14 55)', header: null },
    node_fallback: { fill: 'oklch(94% 0.03 230)', stroke: 'oklch(58% 0.10 230)', header: null },
  } as unknown as Palette;

  // categories map is emitted from Rust (F3); the FE just looks it up.
  const categories: Record<string, string> = {
    Part: 'block',
    Connection: 'block',
    State: 'state',
    InitialNode: 'node_fallback',
  };

  it('resolves VisualKind colors via the Rust-emitted categories map', () => {
    expect(colorsForVisualKind(palette, categories, 'Part').fill).toBe('oklch(94% 0.04 155)');
    expect(colorsForVisualKind(palette, categories, 'Connection').fill).toBe('oklch(94% 0.04 155)');
    expect(colorsForVisualKind(palette, categories, 'State').stroke).toBe('oklch(60% 0.14 55)');
    expect(colorsForVisualKind(palette, categories, 'InitialNode').fill).toBe('oklch(94% 0.03 230)');
    // unmapped kind → node_fallback
    expect(colorsForVisualKind(palette, categories, 'TotallyUnknown').fill).toBe('oklch(94% 0.03 230)');
  });

});

describe('shape dispatch (Rust-emitted shapes map)', () => {
  // The map is emitted from Rust (`DesignTokens.shapes`); the FE just looks up.
  const shapes: Record<string, string> = {
    Part: 'Rect',
    State: 'RoundedRect',
    UseCase: 'Ellipse',
    DecisionNode: 'Diamond',
    ForkNode: 'HBar',
    InitialNode: 'FilledCircle',
    FinalNode: 'BullseyeCircle',
  };

  it('resolves shapes from the emitted map, falling back to Rect', () => {
    expect(shapeForVisualKind(shapes, 'State')).toBe('RoundedRect');
    expect(shapeForVisualKind(shapes, 'DecisionNode')).toBe('Diamond');
    expect(shapeForVisualKind(shapes, 'TotallyUnknown')).toBe('Rect');
  });

  it('classifies card vs glyph shapes', () => {
    expect(isCardShape('Rect')).toBe(true);
    expect(isCardShape('RoundedRect')).toBe(true);
    expect(isCardShape('NoteRect')).toBe(true);
    expect(isCardShape('Diamond')).toBe(false);
    expect(isCardShape('Ellipse')).toBe(false);
  });

  it('gives control glyphs a fixed box, cards none', () => {
    expect(glyphSizeFor('Diamond')).toEqual({ w: 40, h: 32 });
    expect(glyphSizeFor('HBar')).toEqual({ w: 60, h: 10 });
    expect(glyphSizeFor('FilledCircle')).toEqual({ w: 22, h: 22 });
    // Cards and named glyphs keep their text-derived box.
    expect(glyphSizeFor('Rect')).toBeNull();
    expect(glyphSizeFor('Ellipse')).toBeNull();
  });
});

describe('edgeDecor (edge markers/dash/label from emitted edge_styles)', () => {
  // Keys are the camelCase WIRE names (`RelationshipKind::wire_name()`) — the
  // exact strings the backend serializes into `edge.kind.Relationship`. Using
  // PascalCase here would re-encode the very contract drift that left the real
  // table unreachable from the renderer.
  const edgeStyles: Record<string, EdgeStyleToken> = {
    specialize: { arrowhead: 'Hollow', line_style: 'Solid', label: null },
    satisfy: { arrowhead: 'Open', line_style: 'Dashed', label: '«satisfy»' },
    subsetting: { arrowhead: 'Open', line_style: 'Dotted', label: '«subsets»' },
    // Symmetric connectors (Table 11): plain line, NO arrowhead (R5).
    connection: { arrowhead: 'None', line_style: 'Solid', label: null },
    binding: { arrowhead: 'None', line_style: 'Solid', label: null },
  };
  function edge(over: Partial<DiagramEdge>): DiagramEdge {
    return {
      id: 'e', source_id: 'a', target_id: 'b', kind: { Relationship: 'specialize' },
      label: '', source_port_id: null, target_port_id: null, precomputed_route: null,
      endpoint_mode: 'AutoSide', label_placement: {}, tags: [], secondary_labels: [], ...over,
    };
  }

  it('maps arrowhead + line style from the emitted table', () => {
    const spec = edgeDecor(edge({ kind: { Relationship: 'specialize' } }), edgeStyles);
    expect(spec.markerEnd).toBe('url(#svgc-tri-hollow)');
    expect(spec.dash).toBeUndefined();
    const sat = edgeDecor(edge({ kind: { Relationship: 'satisfy' } }), edgeStyles);
    expect(sat.markerEnd).toBe('url(#svgc-arrow)');
    expect(sat.dash).toBe('6 4');
    expect(sat.label).toBe('«satisfy»'); // falls back to the stereotype keyword
    expect(edgeDecor(edge({ kind: { Relationship: 'subsetting' } }), edgeStyles).dash).toBe('2 3');
  });

  it('renders symmetric connectors with no arrowhead (R5, Table 11)', () => {
    const conn = edgeDecor(edge({ kind: { Relationship: 'connection' } }), edgeStyles);
    expect(conn.markerEnd).toBeUndefined();
    expect(conn.markerStart).toBeUndefined();
    expect(conn.dash).toBeUndefined();
    const bind = edgeDecor(edge({ kind: { Relationship: 'binding' } }), edgeStyles);
    expect(bind.markerEnd).toBeUndefined();
  });

  it('draws §F-8 aggregation diamonds at the source end', () => {
    const comp = edgeDecor(edge({ kind: { Relationship: 'composition' } }), edgeStyles);
    expect(comp.markerStart).toBe('url(#svgc-diamond-filled)');
    expect(comp.markerEnd).toBeUndefined();
    const shared = edgeDecor(edge({ kind: { Relationship: 'featureMembership' } }), edgeStyles);
    expect(shared.markerStart).toBe('url(#svgc-diamond-open)');
  });

  it('ignores PascalCase kinds — the wire never sends them (key-drift guard)', () => {
    // If someone re-introduces Debug-name keys, these must NOT quietly work.
    const comp = edgeDecor(edge({ kind: { Relationship: 'Composition' } }), edgeStyles);
    expect(comp.markerStart).toBeUndefined();
    const conn = edgeDecor(edge({ kind: { Relationship: 'Connection' } }), edgeStyles);
    expect(conn.markerEnd).toBe('url(#svgc-arrow)'); // fell through to the default
  });

  it('draws §F-9 n-ary segments plain (no markers)', () => {
    const seg = edgeDecor(edge({ tags: ['NarySegment'], kind: { Relationship: 'composition' } }), edgeStyles);
    expect(seg.markerStart).toBeUndefined();
    expect(seg.markerEnd).toBeUndefined();
  });

  it('composes transition labels from trigger + guard', () => {
    const t = edgeDecor(edge({ kind: { Transition: { trigger: 'tick', guard: 'x>0' } } }), edgeStyles);
    expect(t.markerEnd).toBe('url(#svgc-arrow)');
    expect(t.label).toBe('tick [x>0]');
    const cf = edgeDecor(edge({ kind: { ControlFlow: { guard: 'done' } } }), edgeStyles);
    expect(cf.label).toBe('[done]');
  });

  it('prefers an explicit edge label over the composed one', () => {
    const t = edgeDecor(edge({ label: 'go', kind: { Transition: { trigger: 'tick', guard: null } } }), edgeStyles);
    expect(t.label).toBe('go');
  });

  it('labels a sequence message from its payload + dashes return messages', () => {
    const fwd = edgeDecor(
      edge({ kind: { Message: { payload: 'reqOut', is_succession: false, is_move: true, is_push: true } } }),
      edgeStyles,
    );
    expect(fwd.markerEnd).toBe('url(#svgc-tri-filled)');
    expect(fwd.label).toBe('reqOut');
    expect(fwd.dash).toBeUndefined();
    const ret = edgeDecor(
      edge({ tags: ['Return'], kind: { Message: { payload: 'respOut', is_succession: false, is_move: true, is_push: true } } }),
      edgeStyles,
    );
    expect(ret.label).toBe('respOut');
    expect(ret.dash).toBe('5 3');
  });
});

describe('adjustedEdgePoints (drag-override edge preview)', () => {
  const placed = (id: string, x: number, y: number): PlacedNode => ({
    node: node(id), x, y, width: 100, height: 40, depth: 0, headerHeight: 22, lines: [], hasChildren: false, ports: [],
  });
  const placedById = new Map<string, PlacedNode>([
    ['a', placed('a', 0, 0)],
    ['b', placed('b', 200, 200)],
  ]);
  const pe: PlacedEdge = {
    edge: {
      id: 'e', source_id: 'a', target_id: 'b', kind: { Relationship: 'Specialize' }, label: '',
      source_port_id: null, target_port_id: null, precomputed_route: null, endpoint_mode: 'AutoSide',
      label_placement: {}, tags: [], secondary_labels: [],
    },
    points: [{ x: 5, y: 5 }, { x: 50, y: 100 }, { x: 250, y: 220 }],
    label: '', labelAt: { x: 50, y: 100 },
  };

  it('returns the elkjs route by reference when neither endpoint moved', () => {
    expect(adjustedEdgePoints(pe, placedById, () => ({ dx: 0, dy: 0 }))).toBe(pe.points);
  });

  it('rigidly translates the elkjs route when both endpoints move equally', () => {
    // e.g. dragging a container shifts both ends of an internal edge by the same
    // delta — keep the orthogonal route, just offset it.
    const deltaFor = () => ({ dx: 7, dy: -3 });
    const pts = adjustedEdgePoints(pe, placedById, deltaFor);
    expect(pts).toEqual([
      { x: 12, y: 2 },
      { x: 57, y: 97 },
      { x: 257, y: 217 },
    ]);
  });

  it('re-snaps a ported endpoint onto the moved port center after a drag (§2/§6 G4)', () => {
    // Source node 'a' owns port 'pa' on its right face; drag 'a' → the edge's
    // first point must land on the port glyph center (which moved with the node),
    // not the node face.
    const withPort: PlacedNode = {
      node: node('a'),
      x: 0, y: 0, width: 100, height: 40, depth: 0, headerHeight: 22, lines: [], hasChildren: false,
      ports: [
        {
          port: {
            element_id: 'pa', name: 'pa', direction: null, is_conjugated: false, is_reference: false,
            tags: [], sub_ports: [], is_proxy: false, is_hidden: false, side: 'East', position: null, size: null,
          },
          x: 96, y: 16, width: 10, height: 10, labelRect: null,
        },
      ],
    };
    const map = new Map<string, PlacedNode>([
      ['a', withPort],
      ['b', placed('b', 300, 200)],
    ]);
    const ported: PlacedEdge = {
      edge: { ...pe.edge, source_port_id: 'pa', target_port_id: null },
      points: pe.points, label: '', labelAt: null,
    };
    const deltaFor = (id: string) => (id === 'a' ? { dx: 20, dy: 10 } : { dx: 0, dy: 0 });
    const pts = adjustedEdgePoints(ported, map, deltaFor);
    // port center = (96+10/2, 16+10/2) + delta(20,10) = (121, 31).
    expect(pts[0]).toEqual({ x: 121, y: 31 });
  });

  it('re-routes orthogonally (Manhattan elbow) when one endpoint is dragged', () => {
    const deltaFor = (id: string) => (id === 'b' ? { dx: 30, dy: 10 } : { dx: 0, dy: 0 });
    const pts = adjustedEdgePoints(pe, placedById, deltaFor);
    // a (0,0,100,40) center (50,20); b moved to (230,210,100,40) center (280,230).
    // Horizontal-dominant → attach right face of a, left face of b, jog at mid-x 165.
    expect(pts).toEqual([
      { x: 100, y: 20 },
      { x: 165, y: 20 },
      { x: 165, y: 230 },
      { x: 230, y: 230 },
    ]);
    // Every segment is axis-aligned.
    for (let i = 1; i < pts.length; i++) {
      expect(pts[i].x === pts[i - 1].x || pts[i].y === pts[i - 1].y).toBe(true);
    }
  });
});

describe('orthogonalElbow', () => {
  it('attaches top/bottom faces when vertically dominant', () => {
    const s = { x: 0, y: 0, width: 100, height: 40 };
    const t = { x: 10, y: 300, width: 100, height: 40 };
    const pts = orthogonalElbow(s, t);
    // cs (50,20) → ct (60,320); |dy|>|dx| → vertical. attach s bottom y=40, t top y=300.
    expect(pts).toEqual([
      { x: 50, y: 40 },
      { x: 50, y: 170 },
      { x: 60, y: 170 },
      { x: 60, y: 300 },
    ]);
  });
});

describe('buildParentMap (container drag nesting)', () => {
  it('maps nested nodes to their container, skipping compartment wrappers', () => {
    const scene: DiagramIR = {
      view_type: 'General',
      nodes: [
        node('Pkg', {
          children: [
            { Node: node('Block', { children: [{ Node: node('Leaf') }] }) },
            {
              Compartment: {
                kind: 'Attributes',
                children: [{ Node: node('Attr') }],
              },
            },
          ],
        }),
        node('Solo'),
      ],
      edges: [],
      buttons: [],
    };
    const pm = buildParentMap(scene);
    expect(pm).toEqual({ Block: 'Pkg', Leaf: 'Block', Attr: 'Pkg' });
    expect(pm.Solo).toBeUndefined(); // top-level node has no parent
  });
});

describe('sim overlay helpers', () => {
  const overlay: SimOverlay = {
    tick: 5,
    time_ms: 12.5,
    elements: {
      sm: { activity: 'active', value: null },
      done: { activity: 'completed', value: null },
      sensor: { activity: null, value: { value: 1.23456, unit: 'mA' } },
      counter: { activity: null, value: { value: 4, unit: null } },
    },
    channels: [{ channel: 'B', element_id: 'sensor', latest: 0.7, unit: 'T' }],
  };

  it('joins an overlay to a node by id', () => {
    expect(overlayForNode(overlay, 'sm')?.activity).toBe('active');
    expect(overlayForNode(overlay, 'missing')).toBeNull();
    expect(overlayForNode(null, 'sm')).toBeNull();
  });

  it('classifies activity', () => {
    expect(isActive(overlay.elements.sm)).toBe(true);
    expect(isActive(overlay.elements.done)).toBe(false);
    expect(isCompleted(overlay.elements.done)).toBe(true);
    expect(isActive(null)).toBe(false);
  });

  it('formats value badges compactly with units', () => {
    expect(formatOverlayValue({ value: 1.23456, unit: 'mA' })).toBe('1.23 mA');
    expect(formatOverlayValue({ value: 4, unit: null })).toBe('4');
    expect(formatOverlayValue({ value: 0.5, unit: 'T' })).toBe('0.5 T');
  });
});

describe('elementAtOffset (text→diagram reverse lookup)', () => {
  const tm: TextMap = {
    spans: {
      outer: { file: 'f.sysml', start: 0, end: 100, line: 1, col: 1 },
      inner: { file: 'f.sysml', start: 10, end: 30, line: 2, col: 3 },
      other: { file: 'g.sysml', start: 0, end: 50, line: 1, col: 1 },
    },
  };

  it('returns the innermost (smallest) span containing the offset', () => {
    expect(elementAtOffset(tm, 'f.sysml', 20)).toBe('inner');
    expect(elementAtOffset(tm, 'f.sysml', 5)).toBe('outer');
  });

  it('respects the file filter and end-exclusivity', () => {
    expect(elementAtOffset(tm, 'g.sysml', 20)).toBe('other');
    expect(elementAtOffset(tm, 'f.sysml', 100)).toBeNull(); // end exclusive
    expect(elementAtOffset(null, 'f.sysml', 5)).toBeNull();
  });
});

describe('truncateScene (3.11 partial-render cap)', () => {
  const mkScene = (nNodes: number, nEdges: number): DiagramIR => ({
    view_type: 'General',
    nodes: Array.from({ length: nNodes }, (_, i) => node(`n${i}`)),
    edges: Array.from({ length: nEdges }, (_, i) => ({
      id: `e${i}`, source_id: `n${i}`, target_id: `n${(i + 1) % nNodes}`,
      kind: { Relationship: 'Connection' }, label: '', source_port_id: null, target_port_id: null,
      precomputed_route: null, endpoint_mode: 'AutoSide', label_placement: {}, tags: [], secondary_labels: [],
    })),
    buttons: [],
  });

  it('returns the scene unchanged + null truncation when within caps', () => {
    const s = mkScene(10, 5);
    const { scene, truncation } = truncateScene(s, 250, 600);
    expect(scene).toBe(s);
    expect(truncation).toBeNull();
  });

  it('caps nodes and reports the original totals', () => {
    const { scene, truncation } = truncateScene(mkScene(300, 10), 250, 600);
    expect(scene.nodes).toHaveLength(250);
    expect(truncation).toEqual({ nodes: 250, edges: expect.any(Number), totalNodes: 300, totalEdges: 10 });
  });

  it('drops edges whose endpoints fall outside the kept node set', () => {
    // 300 nodes capped to 5; only edges among n0..n4 survive.
    const { scene, truncation } = truncateScene(mkScene(300, 300), 5, 600);
    expect(scene.nodes).toHaveLength(5);
    const kept = new Set(scene.nodes.map((n) => n.element_id));
    expect(scene.edges.every((e) => kept.has(e.source_id) && kept.has(e.target_id))).toBe(true);
    expect(truncation!.totalNodes).toBe(300);
    expect(truncation!.totalEdges).toBe(300);
  });

  it('caps edges independently when only the edge count is over', () => {
    const { scene, truncation } = truncateScene(mkScene(50, 800), 250, 600);
    expect(scene.nodes).toHaveLength(50);
    expect(scene.edges.length).toBeLessThanOrEqual(600);
    expect(truncation!.totalEdges).toBe(800);
  });
});
