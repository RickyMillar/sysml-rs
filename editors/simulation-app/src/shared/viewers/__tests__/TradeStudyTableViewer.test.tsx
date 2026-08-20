/**
 * Tests for the R5.11 TradeStudyTableViewer.
 *
 * Focus is on the things *only* the component decides (sort state,
 * Pareto/best-row markup, pending skeletons, selection → promote
 * toolbar). Scoring math is covered in `tradeHelpers.test.ts`; we assert
 * here only that the component surfaces its outputs correctly.
 */
import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import { tradeStudyTableViewer } from '../TradeStudyTableViewer';
import type { TradeStudyTableData } from '../TradeStudyTableViewer';
import type { TradeCriterion } from '../tradeHelpers';
import { useCompareStore } from '@/workflows/compare/useCompareStore';

afterEach(() => {
  cleanup();
  useCompareStore.getState().setPickedSessionIds([]);
});

const criteria: TradeCriterion[] = [
  { key: 'cost', label: 'Cost', objective: 'min', weight: 0.5, unit: '$' },
  { key: 'thr', label: 'Throughput', objective: 'max', weight: 0.5 },
];

/**
 * 3-row fixture:
 *   A — cheap / slow  (dominated by B on throughput at same cost? no, same cost)
 *   B — cheap / fast  (Pareto, should be rank 1)
 *   C — expensive / slow (dominated)
 */
const data: TradeStudyTableData = {
  kind: 'trade-table',
  alternatives: [
    { id: 'A', session_id: 'sid-a', label: 'Alpha', status: 'completed', metrics: { cost: 10, thr: 10 } },
    { id: 'B', session_id: 'sid-b', label: 'Bravo', status: 'completed', metrics: { cost: 10, thr: 20 } },
    { id: 'C', session_id: 'sid-c', label: 'Charlie', status: 'completed', metrics: { cost: 20, thr: 5 } },
  ],
  criteria,
};

function renderViewer(d: TradeStudyTableData = data) {
  return render(
    <MemoryRouter initialEntries={['/trade']}>
      {tradeStudyTableViewer.render(d, { height: 400 })}
    </MemoryRouter>,
  );
}

describe('tradeStudyTableViewer.accepts', () => {
  it('narrows to TradeStudyTableData', () => {
    expect(tradeStudyTableViewer.accepts(data)).toBe(true);
    expect(tradeStudyTableViewer.accepts({ kind: 'time-series' } as never)).toBe(false);
  });
});

describe('<TradeStudyTableViewer> — rendering', () => {
  it('renders one row per alternative', () => {
    renderViewer();
    expect(screen.getByTestId('row-A')).toBeInTheDocument();
    expect(screen.getByTestId('row-B')).toBeInTheDocument();
    expect(screen.getByTestId('row-C')).toBeInTheDocument();
  });

  it('marks the best row (rank 1) with data-best="true"', () => {
    renderViewer();
    // B is best: cheapest & fastest of the three.
    expect(screen.getByTestId('row-B').getAttribute('data-best')).toBe('true');
    expect(screen.getByTestId('row-A').getAttribute('data-best')).toBe('false');
  });

  it('renders a Pareto chip for undominated rows only', () => {
    renderViewer();
    // B and A share cost=10; B strictly better on throughput → B dominates A.
    // C is dominated by B on both → not Pareto.
    expect(screen.getByTestId('row-B').getAttribute('data-pareto')).toBe('true');
    expect(screen.getByTestId('row-A').getAttribute('data-pareto')).toBe('false');
    expect(screen.getByTestId('row-C').getAttribute('data-pareto')).toBe('false');
    // Chip visible only inside Pareto rows.
    expect(within(screen.getByTestId('pareto-B')).queryByTestId('pareto-chip'))
      .toBeInTheDocument();
  });

  it('shows skeletons for pending rows and blanks their score/rank', () => {
    renderViewer({
      ...data,
      alternatives: [
        ...data.alternatives,
        { id: 'D', session_id: null, status: 'pending', label: 'Delta' },
      ],
    });
    const pendingRow = screen.getByTestId('row-D');
    expect(pendingRow.getAttribute('data-pending')).toBe('true');
    expect(within(pendingRow).getAllByTestId('skeleton-cell').length).toBeGreaterThan(0);
  });

  it('renders the empty state when no alternatives provided', () => {
    renderViewer({ ...data, alternatives: [] });
    expect(screen.getByTestId('trade-study-table-empty')).toBeInTheDocument();
  });
});

describe('<TradeStudyTableViewer> — interaction', () => {
  it('clicking a header toggles sort direction', async () => {
    const user = userEvent.setup();
    renderViewer();
    const header = screen.getByTestId('header-label');
    // First click: asc. Header reflects aria-sort.
    await user.click(header);
    expect(header.getAttribute('aria-sort')).toBe('ascending');
    await user.click(header);
    expect(header.getAttribute('aria-sort')).toBe('descending');
  });

  it('row checkbox selection feeds the promote toolbar', async () => {
    const user = userEvent.setup();
    renderViewer();
    const btn = screen.getByTestId('promote-to-compare-button');
    // Starts disabled (0 selected).
    expect(btn).toBeDisabled();
    await user.click(screen.getByTestId('select-A'));
    await user.click(screen.getByTestId('select-B'));
    // With 2 selected and both carrying session_id → enabled.
    expect(screen.getByTestId('promote-to-compare-button')).not.toBeDisabled();
    expect(screen.getByTestId('trade-study-selected-count').textContent).toMatch(/2/);
  });
});
