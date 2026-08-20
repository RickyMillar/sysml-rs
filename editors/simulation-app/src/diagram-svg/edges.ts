/**
 * Edge decoration for the SvgCanvas renderer: resolves SVG markers, dash
 * pattern, and the display label for an edge from its `kind`/`tags` and the
 * Rust-emitted `edge_styles` table (`DesignTokens.edge_styles`).
 *
 * §F-8 aggregation diamonds are keyed off the RelationshipKind directly
 * (`Composition` → filled, `FeatureMembership` → open) per the IR's single-
 * source rule; §F-9 n-ary branch segments (tag `NarySegment`) draw plain.
 */

import type { DiagramEdge, EdgeStyleToken } from './viewmodel-types';

export interface EdgeDecor {
  /** Marker URL for the source end (aggregation diamonds), if any. */
  markerStart?: string;
  /** Marker URL for the target end (arrowheads), if any. */
  markerEnd?: string;
  /** SVG `stroke-dasharray`, or undefined for a solid line. */
  dash?: string;
  /** Composed display label (may be empty). */
  label: string;
}

const DASH_FOR: Record<string, string | undefined> = {
  Solid: undefined,
  Dashed: '6 4',
  Dotted: '2 3',
};

const ARROW_MARKER: Record<string, string | undefined> = {
  Filled: 'url(#svgc-tri-filled)',
  Hollow: 'url(#svgc-tri-hollow)',
  Open: 'url(#svgc-arrow)',
  None: undefined,
};

export function edgeDecor(edge: DiagramEdge, edgeStyles: Record<string, EdgeStyleToken>): EdgeDecor {
  // §F-9: n-ary branch segment — plain line radiating from the central dot.
  if ((edge.tags ?? []).includes('NarySegment')) {
    return { label: edge.label };
  }
  const kind = edge.kind;
  if ('Relationship' in kind) {
    // `rel` is the SERIALIZED RelationshipKind, which is camelCase
    // (`#[serde(rename_all = "camelCase")]`) — e.g. 'connection', 'typeOf',
    // 'featureMembership'. Every comparison and every `edgeStyles` lookup here
    // MUST use that form: the Rust map is keyed by `RelationshipKind::wire_name()`.
    // (Historically both sides used PascalCase names that the wire never sends,
    // so the whole table silently missed and every edge got the fallback head.)
    const rel = kind.Relationship;
    // §F-8 aggregation: diamond at the whole (source) end, no target head.
    if (rel === 'composition') return { markerStart: 'url(#svgc-diamond-filled)', label: edge.label };
    if (rel === 'featureMembership') return { markerStart: 'url(#svgc-diamond-open)', label: edge.label };
    const style = edgeStyles[rel];
    if (import.meta.env.DEV && !style && Object.keys(edgeStyles).length > 0) {
      // Loud in dev: a miss here means the Rust/TS key contract drifted again.
      // Rendering still degrades gracefully, but it must not do so silently.
      console.warn(
        `[diagram-svg] no edge_styles entry for relationship kind '${rel}' — ` +
          `falling back to Open/Solid. Rust and TS key contracts have drifted ` +
          `(expected a camelCase wire name from RelationshipKind::wire_name()).`,
      );
    }
    return {
      markerEnd: ARROW_MARKER[style?.arrowhead ?? 'Open'],
      dash: DASH_FOR[style?.line_style ?? 'Solid'],
      label: edge.label || style?.label || '',
    };
  }
  if ('Transition' in kind) {
    const { trigger, guard } = kind.Transition;
    return {
      markerEnd: 'url(#svgc-arrow)',
      label: edge.label || [trigger, guard ? `[${guard}]` : ''].filter(Boolean).join(' '),
    };
  }
  if ('ControlFlow' in kind) {
    const { guard } = kind.ControlFlow;
    return { markerEnd: 'url(#svgc-arrow)', label: edge.label || (guard ? `[${guard}]` : '') };
  }
  // Message (sequence): filled arrowhead; return messages dashed. The label
  // (the message payload/signal) lives in the Message kind, not `edge.label`.
  const isReturn = (edge.tags ?? []).includes('Return');
  const label = 'Message' in kind ? edge.label || kind.Message.payload || '' : edge.label;
  return { markerEnd: 'url(#svgc-tri-filled)', dash: isReturn ? '5 3' : undefined, label };
}
