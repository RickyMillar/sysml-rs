/**
 * TableView (R2-10) — the tabular renderer for `TableModel` payloads
 * (traceability matrix): legend rendering, content-sized layout (no
 * full-viewport stretch for near-empty tables), and the honest
 * empty/coverageless states.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import type { TableModel } from '@/shared/api/model';

afterEach(cleanup);

let tableModel: (TableModel & { legend?: { symbol: string; label: string }[] }) | null = null;

vi.mock('@/store/workspace', () => ({
  useWorkspaceStore: (sel: (s: Record<string, unknown>) => unknown) => sel({ tableModel }),
}));

import { TableView } from '../TableView';

/** A matrix like the backend now emits: requirement-def rows, verification
 * cases as columns, V marks, and a legend for the emitted symbols. */
const complianceMatrix = (): TableModel & {
  legend: { symbol: string; label: string }[];
} => ({
  title: 'Traceability Matrix',
  kind: 'traceability_matrix',
  columns: [
    { id: '__requirement__', label: 'Requirement', kind: 'text' },
    { id: 'case-1', label: 'NoTripCase', kind: 'symbol' },
    { id: 'case-2', label: 'FastTripCase', kind: 'symbol' },
  ],
  rows: [
    {
      id: 'req-1',
      cells: [
        { display: 'NoTrip', cssClasses: ['table-row-header'], elementId: 'req-1' },
        { display: 'V', cssClasses: ['table-cell', 'cell-v'], elementId: 'case-1' },
        { display: '' },
      ],
    },
    {
      id: 'req-2',
      cells: [
        { display: 'FastTrip', cssClasses: ['table-row-header'], elementId: 'req-2' },
        { display: '' },
        { display: 'V', cssClasses: ['table-cell', 'cell-v'], elementId: 'case-2' },
      ],
    },
  ],
  legend: [{ symbol: 'V', label: 'Verified by' }],
});

beforeEach(() => {
  tableModel = null;
});

describe('TableView', () => {
  it('renders matrix rows, columns, and symbol cells', () => {
    tableModel = complianceMatrix();
    render(<TableView />);

    expect(screen.getByText('Requirement')).toBeTruthy();
    expect(screen.getByText('NoTripCase')).toBeTruthy();
    expect(screen.getByText('FastTripCase')).toBeTruthy();
    expect(screen.getByText('NoTrip')).toBeTruthy();
    expect(screen.getByText('FastTrip')).toBeTruthy();
    // Two V marks, one per verified requirement row (the legend's V carries
    // cell-v styling too, so match the table-cell class).
    const marks = screen
      .getAllByText('V')
      .filter((el) => el.className.includes('table-cell'));
    expect(marks).toHaveLength(2);
  });

  it('renders the legend for emitted cell symbols', () => {
    tableModel = complianceMatrix();
    render(<TableView />);

    const legend = screen.getByTestId('table-view-legend');
    expect(legend.textContent).toContain('V = Verified by');
  });

  it('omits the legend when the model carries none', () => {
    const model = complianceMatrix();
    delete (model as { legend?: unknown }).legend;
    tableModel = model;
    render(<TableView />);

    expect(screen.queryByTestId('table-view-legend')).toBeNull();
  });

  it('sizes the table to its content instead of stretching full-width', () => {
    tableModel = complianceMatrix();
    render(<TableView />);

    const table = screen.getByRole('table');
    expect(table.style.width).toBe('auto');
    // The scroll container still owns overflow for genuinely wide tables.
    expect(screen.getByTestId('table-view-root').style.overflow).toBe('auto');
  });

  it('flags a coverageless single-column matrix honestly', () => {
    tableModel = {
      title: 'Traceability Matrix',
      kind: 'traceability_matrix',
      columns: [{ id: '__requirement__', label: 'Requirement', kind: 'text' }],
      rows: [
        {
          id: 'req-1',
          cells: [{ display: 'LonelyReq', cssClasses: ['table-row-header'] }],
        },
      ],
    };
    render(<TableView />);

    const note = screen.getByTestId('table-view-no-coverage');
    expect(note.textContent).toContain('1 requirement');
    expect(screen.queryByTestId('table-view-legend')).toBeNull();
  });

  it('explains WHY a rowless matrix is empty instead of saying "no rows"', () => {
    tableModel = {
      title: 'Traceability Matrix',
      kind: 'traceability_matrix',
      columns: [{ id: '__requirement__', label: 'Requirement', kind: 'text' }],
      rows: [],
    };
    render(<TableView />);

    const empty = screen.getByTestId('table-view-empty');
    // Names the real cause (row set is scoped to the exposed subtree), not a
    // bare "No rows in this table."
    expect(empty.textContent).toContain('No requirements in the exposed scope');
    expect(empty.textContent).toContain('expose');
    expect(empty.textContent).not.toContain('No rows in this table');
  });

  it('shows the empty state when no table payload is present', () => {
    tableModel = null;
    render(<TableView />);

    expect(screen.getByTestId('table-view-empty').textContent).toContain(
      'No tabular data',
    );
  });
});
