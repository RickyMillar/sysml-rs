import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { KpisTab } from '../KpisTab';
import { useSessionStore } from '@/features/sessions/store';

import { vi } from 'vitest';

vi.mock('@/shared/export', () => ({
  exportCSV: vi.fn(),
}));

afterEach(() => {
  cleanup();
  localStorage.clear();
  useSessionStore.setState({ activeSessionId: null });
});

describe('KpisTab', () => {
  it('adds a user-defined KPI and evaluates final value', () => {
    useSessionStore.setState({ activeSessionId: 's1' });

    render(
      <KpisTab
        expanded
        clockTime={10}
        timeSeries={{ temp: [{ t: 0, v: 290 }, { t: 1, v: 310 }] }}
      />,
    );

    fireEvent.click(screen.getByTestId('kpis-add'));
    const row = screen.getByTestId('kpi-row');
    expect(row).toBeInTheDocument();
    expect(within(row).getByText('310')).toBeInTheDocument();
  });

  it('marks threshold verdicts as pass or fail', () => {
    useSessionStore.setState({ activeSessionId: 's2' });

    render(
      <KpisTab
        expanded
        clockTime={10}
        timeSeries={{ current: [{ t: 0, v: 2 }, { t: 1, v: 5 }] }}
      />,
    );

    fireEvent.click(screen.getByTestId('kpis-add'));
    const row = screen.getByTestId('kpi-row');
    const selects = within(row).getAllByRole('combobox');
    fireEvent.change(selects[2]!, { target: { value: '<=' } });
    fireEvent.change(within(row).getByLabelText('KPI threshold'), { target: { value: '10' } });
    expect(within(row).getByText('pass')).toBeInTheDocument();

    fireEvent.change(within(row).getByLabelText('KPI threshold'), { target: { value: '1' } });
    expect(within(row).getByText('fail')).toBeInTheDocument();
  });

  it('promotes auto suggestions into editable KPI rows', () => {
    useSessionStore.setState({ activeSessionId: 's3' });

    render(
      <KpisTab
        expanded
        clockTime={10}
        timeSeries={{ loadCurrent: [{ t: 0, v: 2 }, { t: 1, v: 5 }] }}
      />,
    );

    fireEvent.click(screen.getByText('Peak loadCurrent'));
    expect(screen.getByTestId('kpi-row')).toBeInTheDocument();
  });
});
