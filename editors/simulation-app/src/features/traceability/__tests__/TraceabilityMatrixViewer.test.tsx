/**
 * TraceabilityMatrixViewer — render + interaction tests (R6.2).
 *
 * Covers the acceptance criteria:
 *   - Empty / populated render paths
 *   - Filter bar (search / only-unsatisfied / only-no-coverage)
 *   - Sort toggle on column headers
 *   - Row click → selection store called with (uri, rowId)
 *   - Sticky scroll smoke test — first column + first row both carry
 *     `position: sticky` at render time
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { TraceabilityMatrixViewer } from '../TraceabilityMatrixViewer';
import type { TraceMatrix } from '../types';
import type { VerdictKind } from '@/engine/types';

afterEach(() => {
  cleanup();
});

function mkMatrix(
  links: Array<{ row: string; column: string; verdict?: VerdictKind }>,
  opts: {
    rowLabels?: Record<string, string>;
    columnLabels?: Record<string, string>;
    extraRows?: string[];
    extraColumns?: string[];
  } = {},
): TraceMatrix {
  const rowIds = new Set<string>(opts.extraRows ?? []);
  const colIds = new Set<string>(opts.extraColumns ?? []);
  for (const l of links) {
    rowIds.add(l.row);
    colIds.add(l.column);
  }
  return {
    rows: Array.from(rowIds).map((id) => ({ id, label: opts.rowLabels?.[id] ?? id })),
    columns: Array.from(colIds).map((id) => ({
      id,
      label: opts.columnLabels?.[id] ?? id,
    })),
    links: links.map((l) => ({
      row: l.row,
      column: l.column,
      relationship: `${l.row}->${l.column}`,
      verdict: l.verdict ?? 'inconclusive',
    })),
  };
}

describe('TraceabilityMatrixViewer — empty state', () => {
  it('renders the empty-state when the matrix has no rows', () => {
    render(
      <TraceabilityMatrixViewer
        data={mkMatrix([])}
        testHooks={{ select: vi.fn() }}
      />,
    );
    expect(screen.getByTestId('trace-matrix-empty-total')).toBeDefined();
    expect(screen.queryByTestId('trace-matrix-table')).toBeNull();
  });
});

describe('TraceabilityMatrixViewer — populated render', () => {
  const matrix = mkMatrix(
    [
      { row: 'R1', column: 'P1', verdict: 'pass' },
      { row: 'R1', column: 'P2', verdict: 'pass' },
      { row: 'R2', column: 'P1', verdict: 'fail' },
    ],
    {
      rowLabels: { R1: 'Brake force', R2: 'Trip time' },
      columnLabels: { P1: 'Main', P2: 'Aux' },
    },
  );

  it('renders a row per requirement and a column per target', () => {
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    expect(screen.getByTestId('trace-matrix-row-R1')).toBeDefined();
    expect(screen.getByTestId('trace-matrix-row-R2')).toBeDefined();
    expect(screen.getByTestId('trace-matrix-header-col-P1')).toBeDefined();
    expect(screen.getByTestId('trace-matrix-header-col-P2')).toBeDefined();
  });

  it('renders a VerdictBadge in every populated cell', () => {
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    const cellR1P1 = screen.getByTestId('trace-matrix-cell-R1-P1');
    expect(cellR1P1.getAttribute('data-verdict-kind')).toBe('pass');
    const cellR2P1 = screen.getByTestId('trace-matrix-cell-R2-P1');
    expect(cellR2P1.getAttribute('data-verdict-kind')).toBe('fail');
  });

  it('renders a placeholder dash for an (row, column) pair with no link', () => {
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    // R2.P2 has no link in the fixture.
    const cell = screen.getByTestId('trace-matrix-cell-R2-P2');
    expect(cell.getAttribute('data-verdict-kind')).toBe('empty');
  });
});

describe('TraceabilityMatrixViewer — filter bar', () => {
  const matrix = mkMatrix(
    [
      { row: 'R1', column: 'P1', verdict: 'pass' },
      { row: 'R2', column: 'P1', verdict: 'fail' },
    ],
    {
      rowLabels: { R1: 'Brake force', R2: 'Trip time' },
      extraRows: ['R3'], // zero-coverage row
    },
  );

  it('narrows via the search box (substring, case-insensitive)', () => {
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    const search = screen.getByTestId('trace-matrix-search') as HTMLInputElement;
    fireEvent.change(search, { target: { value: 'TRIP' } });
    expect(screen.queryByTestId('trace-matrix-row-R1')).toBeNull();
    expect(screen.getByTestId('trace-matrix-row-R2')).toBeDefined();
  });

  it('narrows to only-unsatisfied rows', () => {
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    fireEvent.click(screen.getByTestId('trace-matrix-toggle-unsatisfied'));
    // R1 is fully-passing → dropped.
    expect(screen.queryByTestId('trace-matrix-row-R1')).toBeNull();
    expect(screen.getByTestId('trace-matrix-row-R2')).toBeDefined();
    expect(screen.getByTestId('trace-matrix-row-R3')).toBeDefined();
  });

  it('narrows to only-no-coverage rows', () => {
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    fireEvent.click(screen.getByTestId('trace-matrix-toggle-no-coverage'));
    expect(screen.queryByTestId('trace-matrix-row-R1')).toBeNull();
    expect(screen.queryByTestId('trace-matrix-row-R2')).toBeNull();
    expect(screen.getByTestId('trace-matrix-row-R3')).toBeDefined();
  });

  it('shows the filtered empty-state when filters exclude everything', () => {
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    const search = screen.getByTestId('trace-matrix-search') as HTMLInputElement;
    fireEvent.change(search, { target: { value: 'nonexistent-needle' } });
    expect(screen.getByTestId('trace-matrix-empty-filtered')).toBeDefined();
  });

  it('clearing via "Clear" restores every row', () => {
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    fireEvent.click(screen.getByTestId('trace-matrix-toggle-unsatisfied'));
    expect(screen.queryByTestId('trace-matrix-row-R1')).toBeNull();
    const clearBtn = screen.getByTestId('trace-matrix-clear-filters-inline');
    fireEvent.click(clearBtn);
    expect(screen.getByTestId('trace-matrix-row-R1')).toBeDefined();
  });
});

describe('TraceabilityMatrixViewer — sort', () => {
  const matrix = mkMatrix(
    [
      { row: 'R-zebra', column: 'P1', verdict: 'pass' },
      { row: 'R-apple', column: 'P1', verdict: 'fail' },
    ],
    { rowLabels: { 'R-zebra': 'Zebra req', 'R-apple': 'Apple req' } },
  );

  function rowOrder(): string[] {
    return Array.from(document.querySelectorAll('tbody tr'))
      .map((tr) => tr.getAttribute('data-testid') ?? '');
  }

  it('sorts rows alphabetically asc by default (click to toggle desc)', () => {
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    // Default = asc on 'row'.
    expect(rowOrder()).toEqual([
      'trace-matrix-row-R-apple',
      'trace-matrix-row-R-zebra',
    ]);
    fireEvent.click(screen.getByTestId('trace-matrix-header-row'));
    // Now desc.
    expect(rowOrder()).toEqual([
      'trace-matrix-row-R-zebra',
      'trace-matrix-row-R-apple',
    ]);
  });

  it('reflects the aria-sort attribute on the sorted header', () => {
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    const header = screen.getByTestId('trace-matrix-header-row');
    expect(header.getAttribute('aria-sort')).toBe('ascending');
    fireEvent.click(header);
    expect(header.getAttribute('aria-sort')).toBe('descending');
  });
});

describe('TraceabilityMatrixViewer — row click → selection', () => {
  it('invokes the selection store with (workspaceUri, rowId) on row-header click', () => {
    const select = vi.fn();
    const matrix = mkMatrix(
      [{ row: 'R1', column: 'P1', verdict: 'pass' }],
      { rowLabels: { R1: 'Requirement 1' } },
    );
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        workspaceUri="file:///ws/test.sysml"
        testHooks={{ select }}
      />,
    );
    fireEvent.click(screen.getByTestId('trace-matrix-row-header-R1'));
    expect(select).toHaveBeenCalledWith('file:///ws/test.sysml', 'R1');
  });

  it('responds to Enter / Space keyboard activation', () => {
    const select = vi.fn();
    const matrix = mkMatrix(
      [{ row: 'R1', column: 'P1', verdict: 'pass' }],
    );
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        workspaceUri="file:///ws/test.sysml"
        testHooks={{ select }}
      />,
    );
    const header = screen.getByTestId('trace-matrix-row-header-R1');
    fireEvent.keyDown(header, { key: 'Enter' });
    expect(select).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(header, { key: ' ' });
    expect(select).toHaveBeenCalledTimes(2);
  });

  it('passes null workspaceUri through when none was supplied', () => {
    const select = vi.fn();
    const matrix = mkMatrix([{ row: 'R1', column: 'P1', verdict: 'pass' }]);
    render(
      <TraceabilityMatrixViewer data={matrix} testHooks={{ select }} />,
    );
    fireEvent.click(screen.getByTestId('trace-matrix-row-header-R1'));
    expect(select).toHaveBeenCalledWith(null, 'R1');
  });
});

describe('TraceabilityMatrixViewer — sticky scroll smoke test', () => {
  it('applies position: sticky to the corner header and the row header', () => {
    const matrix = mkMatrix(
      [{ row: 'R1', column: 'P1', verdict: 'pass' }],
    );
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    // Column-header row is sticky-top via the corner + col cells.
    const corner = screen.getByTestId('trace-matrix-header-row');
    expect((corner as HTMLElement).style.position).toBe('sticky');
    expect((corner as HTMLElement).style.top).toBe('0px');
    const colHeader = screen.getByTestId('trace-matrix-header-col-P1');
    expect((colHeader as HTMLElement).style.position).toBe('sticky');
    // Row header is sticky-left.
    const rowHeader = screen.getByTestId('trace-matrix-row-header-R1');
    expect((rowHeader as HTMLElement).style.position).toBe('sticky');
    expect((rowHeader as HTMLElement).style.left).toBe('0px');
  });

  it('uses getComputedStyle to verify sticky is present for the sorted corner', () => {
    const matrix = mkMatrix(
      [{ row: 'R1', column: 'P1', verdict: 'pass' }],
    );
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    const corner = screen.getByTestId('trace-matrix-header-row');
    const computed = window.getComputedStyle(corner);
    // jsdom returns the inline value verbatim.
    expect(computed.position).toBe('sticky');
  });
});

describe('TraceabilityMatrixViewer — density', () => {
  it('starts in compact density by default', () => {
    const matrix = mkMatrix(
      [{ row: 'R1', column: 'P1', verdict: 'pass' }],
    );
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    const table = screen.getByTestId('trace-matrix-table');
    expect(table.getAttribute('data-density')).toBe('compact');
  });

  it('toggles to roomy when the user picks the chip', () => {
    const matrix = mkMatrix(
      [{ row: 'R1', column: 'P1', verdict: 'pass' }],
    );
    render(
      <TraceabilityMatrixViewer
        data={matrix}
        testHooks={{ select: vi.fn() }}
      />,
    );
    fireEvent.click(screen.getByTestId('trace-matrix-density-roomy'));
    const table = screen.getByTestId('trace-matrix-table');
    expect(table.getAttribute('data-density')).toBe('roomy');
  });
});
