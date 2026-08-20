/**
 * AttributeDetail — component render tests. Exercises the three
 * sample regimes (no samples / 1-2 samples / 3+ samples) + the
 * draft-override pill.
 *
 * uPlot can't mount under jsdom (no real canvas), so we mock the
 * `timeSeriesViewer.render` call to return a plain div. The viewer
 * itself has its own tests elsewhere (TimeSeriesViewer.test.tsx).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';

// Capture the axes config each time the chart renders so tests can
// assert on the markers list the component forwarded. The mock
// returns a plain div (uPlot can't mount under jsdom).
const renderCalls: Array<Parameters<typeof renderStub>> = [];
const renderStub = (_data: unknown, axes: unknown) => {
  renderCalls.push([_data, axes] as never);
  return <div data-testid="uplot-mock" />;
};

vi.mock('@/shared/viewers/TimeSeriesViewer', () => ({
  timeSeriesViewer: {
    id: 'time-series-uplot',
    kind: 'time-series',
    accepts: () => true,
    render: (d: unknown, a: unknown) => renderStub(d, a),
  },
}));

import { AttributeDetail } from '../detail/AttributeDetail';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { useSessionStore } from '@/features/sessions/store';
import type { AttributeTreeNode } from '../types';

function attrNode(overrides: Partial<AttributeTreeNode> = {}): AttributeTreeNode {
  return {
    id: 'a1',
    uri: 'file:///w.sysml',
    name: 'temperature',
    rawKind: 'AttributeUsage',
    kind: 'attribute',
    depth: 2,
    ownerPath: 'ProductionCell.Station1',
    children: [],
    unit: 'V',
    ...overrides,
  } as AttributeTreeNode;
}

function pushSample(name: string, t: number, v: number) {
  useTimeSeriesStore.getState().pushPoint(t, { [name]: v });
}

beforeEach(() => {
  useTimeSeriesStore.getState().reset();
  useSessionStore.getState().clearDraftOverrides();
  renderCalls.length = 0;
});

afterEach(() => {
  cleanup();
});

describe('AttributeDetail — render', () => {
  it('empty state when no samples are recorded yet', () => {
    render(
      <AttributeDetail node={attrNode({ value: 12.5 })} testIdPrefix="d" />,
    );
    expect(screen.getByTestId('d-attribute-value')).toHaveTextContent('12.5');
    expect(screen.getByTestId('d-attribute-value')).toHaveTextContent('V');
    expect(
      screen.getByTestId('d-attribute-chart-empty'),
    ).toBeInTheDocument();
    // Stats strip suppressed when there are no samples.
    expect(screen.queryByTestId('d-attribute-stats')).toBeNull();
  });

  it('sparkline fallback at 1-2 samples', () => {
    pushSample('ProductionCell.Station1.temperature', 0, 10);
    pushSample('ProductionCell.Station1.temperature', 1, 12);
    render(<AttributeDetail node={attrNode({ value: 12 })} testIdPrefix="d" />);
    expect(
      screen.getByTestId('d-attribute-chart-fallback'),
    ).toBeInTheDocument();
    // Stats computed even from 2 samples.
    expect(screen.getByTestId('d-attribute-stats')).toBeInTheDocument();
  });

  it('full chart at 3+ samples with stats strip', () => {
    pushSample('ProductionCell.Station1.temperature', 0, 10);
    pushSample('ProductionCell.Station1.temperature', 1, 12);
    pushSample('ProductionCell.Station1.temperature', 2, 14);
    render(<AttributeDetail node={attrNode({ value: 14 })} testIdPrefix="d" />);
    expect(
      screen.getByTestId('d-attribute-chart'),
    ).toBeInTheDocument();
    expect(screen.getByTestId('d-attribute-stats')).toHaveTextContent('min');
    expect(screen.getByTestId('d-attribute-stats')).toHaveTextContent('max');
    expect(screen.getByTestId('d-attribute-stats')).toHaveTextContent('mean');
  });

  it('draft override pill appears when setDraftOverride has been called for this variable', () => {
    useSessionStore
      .getState()
      .setDraftOverride('ProductionCell.Station1.temperature', '13.0');
    render(<AttributeDetail node={attrNode({ value: 12.5 })} testIdPrefix="d" />);
    expect(screen.getByTestId('d-attribute-draft')).toHaveTextContent('13.0');
  });

  it('draft override pill absent when nothing is queued', () => {
    render(<AttributeDetail node={attrNode({ value: 12.5 })} testIdPrefix="d" />);
    expect(screen.queryByTestId('d-attribute-draft')).toBeNull();
  });

  it('falls back to bare name lookup when ownerPath-scoped series is absent', () => {
    // Some models emit scalar_vars keyed by bare name.
    pushSample('temperature', 0, 5);
    pushSample('temperature', 1, 6);
    pushSample('temperature', 2, 7);
    render(<AttributeDetail node={attrNode({ value: 7 })} testIdPrefix="d" />);
    expect(screen.getByTestId('d-attribute-chart')).toBeInTheDocument();
  });

  it('shows em-dash for value when nothing observed', () => {
    render(<AttributeDetail node={attrNode()} testIdPrefix="d" />);
    expect(screen.getByTestId('d-attribute-value')).toHaveTextContent('—');
  });
});

describe('AttributeDetail — bounds overlay (Task #155, R3.3)', () => {
  /**
   * R3.3: bounds are now pre-computed by the backend
   * (`bounds.rs::extract_bounds_by_attribute`) and rendered onto the
   * AttributeTreeNode by the build pipeline. The component just reads
   * `node.bounds` — no FE AST walker, no `useExpressionAst` mock.
   * These tests inject `bounds` directly on the test fixture.
   */
  const upperAt = (y: number, name = 'voltageCap') =>
    ({
      y,
      kind: 'upper' as const,
      operator: '<' as const,
      constraintName: name,
    });

  it('forwards bound markers into the chart axes config', () => {
    pushSample('ProductionCell.Station1.temperature', 0, 10);
    pushSample('ProductionCell.Station1.temperature', 1, 11);
    pushSample('ProductionCell.Station1.temperature', 2, 12);
    render(
      <AttributeDetail
        node={attrNode({ value: 12, bounds: [upperAt(24)] })}
        testIdPrefix="d"
      />,
    );
    // Chart mounts (3+ samples path).
    expect(screen.getByTestId('d-attribute-chart')).toBeInTheDocument();
    // Our viewer mock captured the axes — the markers list should
    // include exactly one upper-bound at 24.
    const lastCall = renderCalls[renderCalls.length - 1];
    const axes = lastCall[1] as { markers?: Array<{ y: number; dash?: unknown }> };
    expect(axes.markers).toHaveLength(1);
    expect(axes.markers?.[0].y).toBe(24);
    // Non-target bounds render dashed.
    expect(axes.markers?.[0].dash).toBeTruthy();
  });

  it('renders a legend chip per discovered bound under the chart', () => {
    pushSample('ProductionCell.Station1.temperature', 0, 10);
    pushSample('ProductionCell.Station1.temperature', 1, 11);
    pushSample('ProductionCell.Station1.temperature', 2, 12);
    render(
      <AttributeDetail
        node={attrNode({ value: 12, bounds: [upperAt(24)] })}
        testIdPrefix="d"
      />,
    );
    expect(screen.getByTestId('d-attribute-bounds')).toBeInTheDocument();
    expect(
      screen.getByTestId('d-attribute-bound-upper-24'),
    ).toBeInTheDocument();
  });

  it('omits the legend when the backend ships no bounds for this attribute', () => {
    pushSample('ProductionCell.Station1.temperature', 0, 10);
    pushSample('ProductionCell.Station1.temperature', 1, 11);
    pushSample('ProductionCell.Station1.temperature', 2, 12);
    // Empty bounds (or absent) — no overlay should render. The
    // backend resolves attribution by ElementId so a bound on a
    // *different* AttributeUsage will simply never make it onto
    // this node.
    render(<AttributeDetail node={attrNode({ value: 12 })} testIdPrefix="d" />);
    expect(screen.queryByTestId('d-attribute-bounds')).toBeNull();
  });
});
