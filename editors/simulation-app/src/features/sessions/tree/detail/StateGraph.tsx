/**
 * StateGraph — compact SVG state-machine diagram.
 *
 * Round 2 Task #145. Inputs come straight from the structural build
 * (states + transitions on SmTreeNode); the layout helper
 * (`stateGraphLayout`) places every state on a circle and bows the
 * transition arcs so bidirectional pairs don't overlap. The
 * currently-active state is highlighted; every other state is a
 * muted outline.
 *
 * Interactivity v1: click any state to fire `onSelectState(name)`
 * — today the SmDetail consumer only uses it for a focus outline
 * (force-state is a later backend task).
 */
import type {
  SmStateDescriptor,
  SmTransitionDescriptor,
} from '../types';
import { layoutStateGraph } from './stateGraphLayout';

export interface StateGraphProps {
  states: readonly SmStateDescriptor[];
  transitions: readonly SmTransitionDescriptor[];
  /** Name of the currently-active state (from SmTreeNode.currentState). */
  currentState?: string;
  /** Called when the user clicks a state. */
  onSelectState?: (name: string) => void;
  /** Width override — default 320. */
  width?: number;
  /** Height override — default 240. */
  height?: number;
  testId?: string;
}

export function StateGraph({
  states,
  transitions,
  currentState,
  onSelectState,
  width,
  height,
  testId = 'state-graph',
}: StateGraphProps) {
  if (states.length === 0) {
    return (
      <div
        data-testid={`${testId}-empty`}
        style={{
          fontSize: 11,
          color: 'var(--outline)',
          padding: 8,
          border: '1px dashed var(--outline-variant)',
          borderRadius: 4,
        }}
      >
        No states found on this state machine yet.
      </div>
    );
  }

  const layout = layoutStateGraph(states, transitions, { width, height });
  const currentLower = currentState?.toLowerCase();
  const nodeRadius = 26;

  return (
    <svg
      data-testid={testId}
      data-state-count={states.length}
      data-transition-count={transitions.length}
      width={layout.width}
      height={layout.height}
      viewBox={`0 0 ${layout.width} ${layout.height}`}
      style={{
        display: 'block',
        background: 'var(--surface-container)',
        borderRadius: 4,
      }}
    >
      <defs>
        <marker
          id="sg-arrow"
          viewBox="0 0 8 8"
          refX="7"
          refY="4"
          markerWidth="6"
          markerHeight="6"
          orient="auto-start-reverse"
        >
          <path d="M 0 0 L 8 4 L 0 8 z" fill="var(--on-surface-variant)" />
        </marker>
        <marker
          id="sg-arrow-active"
          viewBox="0 0 8 8"
          refX="7"
          refY="4"
          markerWidth="6"
          markerHeight="6"
          orient="auto-start-reverse"
        >
          <path d="M 0 0 L 8 4 L 0 8 z" fill="var(--primary)" />
        </marker>
      </defs>

      {/* Edges go first so nodes paint over them. */}
      {layout.edges.map((edge) => {
        if (!edge.path) return null;
        const isActiveSource =
          !!currentLower &&
          !!edge.sourceId &&
          layout.nodes.find((n) => n.id === edge.sourceId)?.name.toLowerCase() ===
            currentLower;
        return (
          <g
            key={edge.id}
            data-testid={`${testId}-edge-${edge.id}`}
            data-self-loop={edge.selfLoop || undefined}
            data-active-source={isActiveSource || undefined}
          >
            <path
              d={edge.path}
              fill="none"
              stroke={isActiveSource ? 'var(--primary)' : 'var(--on-surface-variant)'}
              strokeWidth={isActiveSource ? 2 : 1.3}
              opacity={isActiveSource ? 0.95 : 0.7}
              markerEnd={`url(#${isActiveSource ? 'sg-arrow-active' : 'sg-arrow'})`}
            />
            <text
              x={edge.labelX}
              y={edge.labelY}
              textAnchor="middle"
              style={{
                fontSize: 9,
                fill: 'var(--outline)',
                fontFamily: 'var(--font-mono, monospace)',
                pointerEvents: 'none',
              }}
            >
              {edge.label.length > 20 ? edge.label.slice(0, 18) + '…' : edge.label}
            </text>
          </g>
        );
      })}

      {/* Nodes (circles + labels). */}
      {layout.nodes.map((node) => {
        const isActive = node.name.toLowerCase() === currentLower;
        return (
          <g
            key={node.id}
            data-testid={`${testId}-node-${node.id}`}
            data-active={isActive || undefined}
            onClick={
              onSelectState ? () => onSelectState(node.name) : undefined
            }
            style={{ cursor: onSelectState ? 'pointer' : 'default' }}
          >
            <circle
              cx={node.cx}
              cy={node.cy}
              r={nodeRadius}
              fill={
                isActive
                  ? 'color-mix(in srgb, var(--primary) 22%, var(--surface-container))'
                  : 'var(--surface-container-high)'
              }
              stroke={
                isActive ? 'var(--primary)' : 'var(--outline-variant)'
              }
              strokeWidth={isActive ? 2 : 1}
            />
            <text
              x={node.cx}
              y={node.cy + 4}
              textAnchor="middle"
              style={{
                fontSize: 10,
                fill: isActive ? 'var(--on-surface)' : 'var(--on-surface-variant)',
                fontWeight: isActive ? 600 : 500,
                fontFamily: 'var(--font-mono, monospace)',
                pointerEvents: 'none',
              }}
            >
              {node.name.length > 14 ? node.name.slice(0, 12) + '…' : node.name}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
