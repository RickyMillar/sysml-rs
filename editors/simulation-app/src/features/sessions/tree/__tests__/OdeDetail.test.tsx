/**
 * OdeDetail — render tests covering the three surfaces added by
 * GAP-ODE-002: current readout (value + dy/dt), dy/dt history
 * sparkline, and the sibling-paired phase portrait.
 *
 * Mocks `useSessionLiveStore` + `useTimeSeriesStore` at the module
 * boundary so tests stay free of the zustand stores' singleton
 * state and don't need a React Query provider.
 */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';

type SnapshotStub = {
  tick: number;
  time_ms: number;
  scalar_vars: Record<string, number>;
  derivatives?: Record<string, number>;
};

const liveState: { snapshot: SnapshotStub | null } = { snapshot: null };
const tsState = {
  revision: 0,
  series: {} as Record<string, Array<{ t: number; v: number }>>,
};

vi.mock('../../sessionLiveStore', () => ({
  useSessionLiveStore: <T,>(selector: (s: { snapshot: SnapshotStub | null }) => T): T =>
    selector(liveState),
}));

vi.mock('@/shared/data/useTimeSeriesStore', () => ({
  useTimeSeriesStore: Object.assign(
    <T,>(selector: (s: { revision: number }) => T): T =>
      selector({ revision: tsState.revision }),
    {
      getState: () => ({ getTimeSeries: () => tsState.series }),
    },
  ),
}));

import { OdeDetail } from '../detail/OdeDetail';
import type { OdeTreeNode } from '../types';

function odeNode(overrides: Partial<OdeTreeNode> = {}): OdeTreeNode {
  return {
    id: 'o1',
    uri: 'file:///w.sysml',
    name: 'temperature',
    rawKind: 'CalculationUsage',
    kind: 'ode',
    depth: 2,
    ownerPath: 'ProductionCell.GroupHead',
    children: [],
    ...overrides,
  } as OdeTreeNode;
}

afterEach(() => {
  cleanup();
  liveState.snapshot = null;
  tsState.revision = 0;
  tsState.series = {};
});

describe('OdeDetail — current readout', () => {
  it('renders em-dash placeholders before the first snapshot', () => {
    liveState.snapshot = null;
    render(<OdeDetail node={odeNode()} testIdPrefix="d" />);
    const readout = screen.getByTestId('d-ode-readout');
    // Value placeholder.
    expect(readout.textContent).toContain('—');
    // dy/dt placeholder.
    expect(screen.getByTestId('d-ode-dydt').textContent).toContain('—');
  });

  it('renders the fully-qualified scalar value + dy/dt when the snapshot carries both', () => {
    liveState.snapshot = {
      tick: 5,
      time_ms: 500,
      scalar_vars: { 'ProductionCell.GroupHead.temperature': 321.5 },
      derivatives: { 'ProductionCell.GroupHead.temperature': 0.125 },
    };
    render(<OdeDetail node={odeNode()} testIdPrefix="d" />);
    const readout = screen.getByTestId('d-ode-readout');
    expect(readout.textContent).toContain('321.5');
    const dydt = screen.getByTestId('d-ode-dydt').textContent!;
    expect(dydt).toContain('0.125');
    expect(dydt).toContain('/s');
  });

  it('falls back to the node.value baked by the workspace tree when scalar_vars has no entry', () => {
    liveState.snapshot = {
      tick: 0,
      time_ms: 0,
      scalar_vars: {},
    };
    render(
      <OdeDetail
        node={odeNode({ value: 20, unit: 'K' })}
        testIdPrefix="d"
      />,
    );
    expect(screen.getByTestId('d-ode-readout').textContent).toContain('20');
  });
});

describe('OdeDetail — dy/dt history sparkline', () => {
  it('shows the "gathering" placeholder until at least 3 samples have been observed', () => {
    liveState.snapshot = {
      tick: 1,
      time_ms: 100,
      scalar_vars: { 'ProductionCell.GroupHead.temperature': 100 },
      derivatives: { 'ProductionCell.GroupHead.temperature': 0.1 },
    };
    render(<OdeDetail node={odeNode()} testIdPrefix="d" />);
    expect(screen.getByTestId('d-ode-deriv-spark-empty')).toBeInTheDocument();
  });
});

describe('OdeDetail — phase portrait pairing', () => {
  it('does not render a portrait when no sibling ODE state exists', () => {
    liveState.snapshot = {
      tick: 1,
      time_ms: 10,
      scalar_vars: { 'ProductionCell.GroupHead.temperature': 300 },
      derivatives: { 'ProductionCell.GroupHead.temperature': 0.1 },
    };
    render(<OdeDetail node={odeNode()} testIdPrefix="d" />);
    expect(screen.queryByTestId('d-ode-phase')).toBeNull();
  });

  it('renders a portrait when a sibling ODE state lives under the same parent and the time-series has ≥2 paired samples', () => {
    liveState.snapshot = {
      tick: 10,
      time_ms: 1000,
      scalar_vars: {
        'ProductionCell.GroupHead.temperature': 320,
        'ProductionCell.GroupHead.charge': 0.5,
      },
      derivatives: {
        'ProductionCell.GroupHead.temperature': 0.1,
        'ProductionCell.GroupHead.charge': -0.02,
      },
    };
    tsState.series = {
      'ProductionCell.GroupHead.temperature': [
        { t: 0, v: 300 },
        { t: 1, v: 310 },
        { t: 2, v: 320 },
      ],
      'ProductionCell.GroupHead.charge': [
        { t: 0, v: 1 },
        { t: 1, v: 0.7 },
        { t: 2, v: 0.5 },
      ],
    };
    render(<OdeDetail node={odeNode()} testIdPrefix="d" />);
    expect(screen.getByTestId('d-ode-phase')).toBeInTheDocument();
    // Trailing head marker at the most recent point.
    expect(screen.getByTestId('d-ode-phase-head')).toBeInTheDocument();
  });

  it('ignores sibling entries under nested sub-parts (deeper dot-paths)', () => {
    liveState.snapshot = {
      tick: 5,
      time_ms: 500,
      scalar_vars: { 'ProductionCell.GroupHead.temperature': 320 },
      // Nested derivative — belongs to a sub-part, not to GroupHead.
      derivatives: {
        'ProductionCell.GroupHead.temperature': 0.1,
        'ProductionCell.GroupHead.relay.coil_current': 2.0,
      },
    };
    tsState.series = {
      'ProductionCell.GroupHead.temperature': [
        { t: 0, v: 310 },
        { t: 1, v: 320 },
      ],
      'ProductionCell.GroupHead.relay.coil_current': [
        { t: 0, v: 1 },
        { t: 1, v: 2 },
      ],
    };
    render(<OdeDetail node={odeNode()} testIdPrefix="d" />);
    expect(screen.queryByTestId('d-ode-phase')).toBeNull();
  });
});
