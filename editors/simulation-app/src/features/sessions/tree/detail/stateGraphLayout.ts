/**
 * Pure layout helpers for the state-graph SVG.
 *
 * Input: a flat list of states + transitions (output of
 * `extractSmTopology`).
 * Output: positioned nodes + edge curves that the SVG renderer can
 * consume without doing any geometry.
 *
 * Layout algorithm (v1, simple):
 *   - Arrange states evenly around a circle, starting at 12 o'clock.
 *   - Each edge is a quadratic curve from source to target with a
 *     small perpendicular offset so bidirectional pairs don't
 *     overlap.
 *   - Self-loops render as a small arc above the node.
 *
 * Why a pure helper: we unit-test layout without React, and the
 * layout is deterministic (stable across renders) so React's
 * reconciler gets the cheapest possible diff.
 */
import type {
  SmStateDescriptor,
  SmTransitionDescriptor,
} from '../types';

export interface GraphNode {
  id: string;
  name: string;
  cx: number;
  cy: number;
}

export interface GraphEdge {
  id: string;
  label: string;
  /** Node id — undefined when the transition name didn't parse. */
  sourceId?: string;
  targetId?: string;
  /** Actual drawing coordinates. `null` when we don't have both endpoints. */
  path: string | null;
  /** Midpoint of the drawn path — where the label sits. */
  labelX: number;
  labelY: number;
  /** `true` when source === target. */
  selfLoop: boolean;
}

export interface GraphLayout {
  width: number;
  height: number;
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface LayoutOptions {
  width?: number;
  height?: number;
  nodeRadius?: number;
}

const DEFAULT_WIDTH = 320;
const DEFAULT_HEIGHT = 240;
const DEFAULT_NODE_RADIUS = 26;

export function layoutStateGraph(
  states: readonly SmStateDescriptor[],
  transitions: readonly SmTransitionDescriptor[],
  options: LayoutOptions = {},
): GraphLayout {
  const width = options.width ?? DEFAULT_WIDTH;
  const height = options.height ?? DEFAULT_HEIGHT;
  const nodeRadius = options.nodeRadius ?? DEFAULT_NODE_RADIUS;
  const cx = width / 2;
  const cy = height / 2;
  const outerPadding = nodeRadius + 12;
  const r = Math.max(0, Math.min(cx, cy) - outerPadding);

  const n = states.length;
  const nodes: GraphNode[] = states.map((state, i) => {
    if (n === 1) return { id: state.id, name: state.name, cx, cy };
    // Start at 12 o'clock (angle -π/2), go clockwise.
    const angle = -Math.PI / 2 + (2 * Math.PI * i) / n;
    return {
      id: state.id,
      name: state.name,
      cx: cx + r * Math.cos(angle),
      cy: cy + r * Math.sin(angle),
    };
  });

  const byName = new Map<string, GraphNode>();
  for (const node of nodes) byName.set(node.name.toLowerCase(), node);

  // Pair tracker so bidirectional edges bow in opposite directions.
  const pairDirections = new Map<string, number>();

  const edges: GraphEdge[] = transitions.map((t) => {
    const source = t.source ? byName.get(t.source.toLowerCase()) : undefined;
    const target = t.target ? byName.get(t.target.toLowerCase()) : undefined;
    const labelFallback = t.source && t.target
      ? `${t.source} → ${t.target}`
      : t.name;

    if (!source || !target) {
      return {
        id: t.id,
        label: t.name || labelFallback,
        sourceId: source?.id,
        targetId: target?.id,
        path: null,
        labelX: 0,
        labelY: 0,
        selfLoop: false,
      };
    }

    if (source.id === target.id) {
      // Self-loop: small arc above the node.
      const loopR = nodeRadius * 0.7;
      const lx = source.cx;
      const ly = source.cy - nodeRadius - loopR;
      return {
        id: t.id,
        label: t.name || labelFallback,
        sourceId: source.id,
        targetId: target.id,
        path: `M ${source.cx - 4} ${source.cy - nodeRadius} ` +
              `A ${loopR} ${loopR} 0 1 1 ${source.cx + 4} ${source.cy - nodeRadius}`,
        labelX: lx,
        labelY: ly - 4,
        selfLoop: true,
      };
    }

    // Figure out curve direction for overlap avoidance.
    const pairKey = [source.id, target.id].sort().join('|');
    const dir = pairDirections.get(pairKey) ?? 1;
    pairDirections.set(pairKey, -dir);

    // Compute the normal using a CANONICAL source→target direction
    // (lower id first) so a bidirectional pair's normals don't
    // flip on each other and cancel the bow-sign alternation.
    const [canonA, canonB] =
      source.id < target.id ? [source, target] : [target, source];
    const cdx = canonB.cx - canonA.cx;
    const cdy = canonB.cy - canonA.cy;
    const clen = Math.hypot(cdx, cdy) || 1;
    const nx = -cdy / clen;
    const ny = cdx / clen;
    const bow = 18 * dir;

    const dx = target.cx - source.cx;
    const dy = target.cy - source.cy;
    const len = Math.hypot(dx, dy) || 1;
    const mx = (source.cx + target.cx) / 2 + nx * bow;
    const my = (source.cy + target.cy) / 2 + ny * bow;

    // Shorten start/end to the node boundary so the arrow tip lands
    // right on the circle, not at the centre.
    const startX = source.cx + (dx / len) * nodeRadius;
    const startY = source.cy + (dy / len) * nodeRadius;
    const endX = target.cx - (dx / len) * nodeRadius;
    const endY = target.cy - (dy / len) * nodeRadius;

    return {
      id: t.id,
      label: t.name || labelFallback,
      sourceId: source.id,
      targetId: target.id,
      path: `M ${startX.toFixed(1)} ${startY.toFixed(1)} Q ${mx.toFixed(1)} ${my.toFixed(1)} ${endX.toFixed(1)} ${endY.toFixed(1)}`,
      labelX: mx,
      labelY: my,
      selfLoop: false,
    };
  });

  return { width, height, nodes, edges };
}
