import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { PlotsTab } from '../PlotsTab';
import { useSessionStore } from '@/features/sessions/store';
import { usePlotSelectionStore } from '@/features/results/usePlotSelectionStore';

import { vi } from 'vitest';

vi.mock('@/shared/viewers/TimeSeriesViewer', () => ({
  timeSeriesViewer: {
    render: (data: { series: Array<{ name: string }> }) => (
      <div data-testid="time-series-viewer">{data.series.map((s) => s.name).join(',')}</div>
    ),
  },
}));

vi.mock('@/features/sessions/OverrideEditor', () => ({
  OverrideEditor: () => <div data-testid="override-editor" />,
}));

afterEach(() => {
  cleanup();
  localStorage.clear();
  useSessionStore.setState({ activeSessionId: null });
  usePlotSelectionStore.setState({ selectionsBySession: {} });
});

describe('PlotsTab', () => {
  it('renders selected variables in the default plot', () => {
    useSessionStore.setState({ activeSessionId: 's1' });
    usePlotSelectionStore.getState().setSelected('s1', ['v_bus']);

    const timeSeries = { v_bus: [{ t: 0, v: 1 }, { t: 1, v: 2 }] };
    render(
      <PlotsTab
        running={false}
        expanded
        timeSeries={timeSeries}
        getFullTimeSeries={() => timeSeries}
      />,
    );

    expect(screen.getByTestId('plots-tab')).toBeInTheDocument();
    expect(screen.getByTestId('plot-card-0')).toBeInTheDocument();
    expect(screen.getByTestId('time-series-viewer')).toHaveTextContent('v_bus');
  });

  it('adds and removes additional plots', () => {
    useSessionStore.setState({ activeSessionId: 's2' });

    render(<PlotsTab running={false} expanded timeSeries={{}} getFullTimeSeries={() => ({})} />);

    fireEvent.click(screen.getByTestId('plots-add-plot'));
    expect(screen.getByTestId('plot-card-1')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('plot-remove-1'));
    expect(screen.queryByTestId('plot-card-1')).toBeNull();
  });

  it('renders an XY plot from two selected variables', () => {
    useSessionStore.setState({ activeSessionId: 's3' });
    usePlotSelectionStore.getState().setSelected('s3', ['x', 'y']);

    const timeSeries = {
      x: [{ t: 0, v: 0 }, { t: 1, v: 1 }, { t: 2, v: 2 }],
      y: [{ t: 0, v: 0 }, { t: 1, v: 2 }, { t: 2, v: 4 }],
    };
    render(
      <PlotsTab
        running={false}
        expanded
        timeSeries={timeSeries}
        getFullTimeSeries={() => timeSeries}
      />,
    );

    fireEvent.change(screen.getByTestId('plot-kind-0'), { target: { value: 'xy' } });
    expect(screen.getByTestId('plot-xy-viewer')).toBeInTheDocument();
  });

  it('renders a projected 3D trajectory from three selected variables', () => {
    useSessionStore.setState({ activeSessionId: 's4' });
    usePlotSelectionStore.getState().setSelected('s4', ['x', 'y', 'z']);

    const timeSeries = {
      x: [{ t: 0, v: 0 }, { t: 1, v: 1 }],
      y: [{ t: 0, v: 0 }, { t: 1, v: 1 }],
      z: [{ t: 0, v: 0 }, { t: 1, v: 2 }],
    };
    render(
      <PlotsTab
        running={false}
        expanded
        timeSeries={timeSeries}
        getFullTimeSeries={() => timeSeries}
      />,
    );

    fireEvent.change(screen.getByTestId('plot-kind-0'), { target: { value: 'trajectory-3d' } });
    expect(screen.getByTestId('plot-trajectory-3d-viewer')).toBeInTheDocument();
  });
});
