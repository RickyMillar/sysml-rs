/**
 * Tests for the Layer 1 EP6 PassFailGridViewer (R3.3).
 *
 * Covers the contract that VerifyWorkflow and future aggregating
 * workflows depend on:
 *   1. `accepts` narrows `ResultData` to `PassFailGridData`.
 *   2. All four `VerdictKind` values render with distinct badges.
 *   3. Clicking a cell fires `onVerdictSelect` with the right verdict.
 *   4. Column-header sorting reverses row order (asc ↔ desc).
 *   5. Empty state renders the guidance string.
 *   6. Summary totals tile counts all four verdict kinds.
 */
import { describe, it, expect, afterEach, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { isValidElement } from 'react';
import { passFailGridViewer } from '../PassFailGridViewer';
import type { Verdict } from '../../../engine/types';
import type { PassFailGridData, ResultData } from '../types';

afterEach(() => {
  cleanup();
});

// ── Fixtures ────────────────────────────────────────────────────────

function mkVerdict(
  verdict: Verdict['verdict'],
  caseName: string,
  requirement: string,
  extra?: Partial<Verdict>,
): Verdict {
  return {
    verdict,
    metadata: {
      case_name: caseName,
      requirement_id: requirement,
      ...(extra?.metadata ?? {}),
    },
    ...extra,
  };
}

/**
 * Fixture: 2 cases × 2 requirements, all four verdict kinds represented.
 *
 *                   R1           R2
 *   Alpha  |   pass         inconclusive
 *   Bravo  |   fail         error
 */
function mkFixture(): Verdict[] {
  return [
    mkVerdict('pass', 'Alpha', 'R1', { actual: 42 }),
    mkVerdict('inconclusive', 'Alpha', 'R2', {
      metadata: { case_name: 'Alpha', requirement_id: 'R2', message: 'non-boolean' },
    }),
    mkVerdict('fail', 'Bravo', 'R1', { actual: 10, expected: 100 }),
    mkVerdict('error', 'Bravo', 'R2', {
      metadata: { case_name: 'Bravo', requirement_id: 'R2', error_reason: 'div-by-zero' },
    }),
  ];
}

function mkData(overrides?: Partial<PassFailGridData>): PassFailGridData {
  return {
    kind: 'pass-fail-grid',
    verdicts: overrides?.verdicts ?? mkFixture(),
    onVerdictSelect: overrides?.onVerdictSelect,
  };
}

// ── Viewer identity / accepts contract ─────────────────────────────

describe('passFailGridViewer', () => {
  it('tags itself as a pass-fail-grid viewer with a stable id', () => {
    expect(passFailGridViewer.kind).toBe('pass-fail-grid');
    expect(passFailGridViewer.id).toBe('pass-fail-grid-default');
  });

  describe('accepts', () => {
    it('returns true for pass-fail-grid payloads', () => {
      const data: ResultData = { kind: 'pass-fail-grid' } as PassFailGridData;
      expect(passFailGridViewer.accepts(data)).toBe(true);
    });

    it('returns false for other viewer kinds', () => {
      const cases: ResultData[] = [
        { kind: 'time-series' },
        { kind: 'histogram' },
        { kind: 'timeline' },
        { kind: 'table' },
        { kind: 'heatmap' },
        { kind: 'parallel-coords' },
      ];
      for (const data of cases) {
        expect(passFailGridViewer.accepts(data)).toBe(false);
      }
    });
  });

  describe('render', () => {
    it('returns a React element for a populated payload', () => {
      const node = passFailGridViewer.render(mkData(), { height: 300 });
      expect(isValidElement(node)).toBe(true);
    });
  });
});

// ── Rendering: all four VerdictKind values visible ─────────────────

describe('PassFailGridViewer — rendering', () => {
  it('renders a row per case and a column per requirement', () => {
    render(passFailGridViewer.render(mkData(), {}));
    expect(screen.getByTestId('row-Alpha')).toBeInTheDocument();
    expect(screen.getByTestId('row-Bravo')).toBeInTheDocument();
    expect(screen.getByTestId('header-col-R1')).toBeInTheDocument();
    expect(screen.getByTestId('header-col-R2')).toBeInTheDocument();
  });

  it('renders a VerdictBadge in each cell, one per verdict kind', () => {
    render(passFailGridViewer.render(mkData(), {}));
    // Pass
    expect(
      within(screen.getByTestId('cell-Alpha-R1')).getByTestId('verdict-badge-pass'),
    ).toBeInTheDocument();
    // Inconclusive
    expect(
      within(screen.getByTestId('cell-Alpha-R2')).getByTestId('verdict-badge-inconclusive'),
    ).toBeInTheDocument();
    // Fail
    expect(
      within(screen.getByTestId('cell-Bravo-R1')).getByTestId('verdict-badge-fail'),
    ).toBeInTheDocument();
    // Error
    expect(
      within(screen.getByTestId('cell-Bravo-R2')).getByTestId('verdict-badge-error'),
    ).toBeInTheDocument();
  });

  it('applies the required accessibility label to each cell', () => {
    render(passFailGridViewer.render(mkData(), {}));
    expect(
      screen.getByLabelText('Verdict for Alpha.R1: Pass'),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText('Verdict for Alpha.R2: Inconclusive'),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText('Verdict for Bravo.R1: Fail'),
    ).toBeInTheDocument();
    expect(
      screen.getByLabelText('Verdict for Bravo.R2: Error'),
    ).toBeInTheDocument();
  });
});

// ── Summary bar totals ─────────────────────────────────────────────

describe('PassFailGridViewer — summary bar', () => {
  it('tallies all four verdict kinds', () => {
    render(passFailGridViewer.render(mkData(), {}));
    expect(screen.getByTestId('summary-tile-pass')).toHaveTextContent('1');
    expect(screen.getByTestId('summary-tile-fail')).toHaveTextContent('1');
    expect(screen.getByTestId('summary-tile-inconclusive')).toHaveTextContent('1');
    expect(screen.getByTestId('summary-tile-error')).toHaveTextContent('1');
  });

  it('labels tiles for screen readers', () => {
    render(passFailGridViewer.render(mkData(), {}));
    const group = screen.getByRole('group', { name: 'Verdict totals' });
    expect(group).toBeInTheDocument();
    expect(within(group).getByLabelText('Pass: 1')).toBeInTheDocument();
    expect(within(group).getByLabelText('Fail: 1')).toBeInTheDocument();
    expect(within(group).getByLabelText('Inconclusive: 1')).toBeInTheDocument();
    expect(within(group).getByLabelText('Error: 1')).toBeInTheDocument();
  });
});

// ── Click handler wiring ───────────────────────────────────────────

describe('PassFailGridViewer — onVerdictSelect', () => {
  it('fires onVerdictSelect with the underlying verdict', () => {
    const onVerdictSelect = vi.fn();
    render(passFailGridViewer.render(mkData({ onVerdictSelect }), {}));
    const cell = screen.getByTestId('verdict-cell-Bravo-R1');
    fireEvent.click(cell);
    expect(onVerdictSelect).toHaveBeenCalledTimes(1);
    const [verdict] = onVerdictSelect.mock.calls[0];
    expect(verdict.verdict).toBe('fail');
    expect(verdict.metadata?.case_name).toBe('Bravo');
    expect(verdict.metadata?.requirement_id).toBe('R1');
  });

  it('fires on Enter key (keyboard accessibility)', () => {
    const onVerdictSelect = vi.fn();
    render(passFailGridViewer.render(mkData({ onVerdictSelect }), {}));
    const cell = screen.getByTestId('verdict-cell-Alpha-R1');
    fireEvent.keyDown(cell, { key: 'Enter' });
    expect(onVerdictSelect).toHaveBeenCalledTimes(1);
    expect(onVerdictSelect.mock.calls[0][0].verdict).toBe('pass');
  });

  it('disables the cell button when no callback is wired', () => {
    render(passFailGridViewer.render(mkData({ onVerdictSelect: undefined }), {}));
    const cell = screen.getByTestId('verdict-cell-Alpha-R1');
    expect(cell).toBeDisabled();
  });
});

// ── Sorting ────────────────────────────────────────────────────────

describe('PassFailGridViewer — sorting', () => {
  it('defaults to case-name ascending', () => {
    render(passFailGridViewer.render(mkData(), {}));
    const rows = screen.getAllByRole('row').slice(1); // drop header row
    const first = rows[0] as HTMLElement;
    expect(first.getAttribute('data-testid')).toBe('row-Alpha');
  });

  it('reverses order when the case header is clicked', () => {
    render(passFailGridViewer.render(mkData(), {}));
    const header = screen.getByTestId('header-case');
    fireEvent.click(header);
    const rows = screen.getAllByRole('row').slice(1);
    const first = rows[0] as HTMLElement;
    expect(first.getAttribute('data-testid')).toBe('row-Bravo');
    expect(header).toHaveAttribute('aria-sort', 'descending');
  });

  it('sorts by a column header (pass-count) when the column is clicked', () => {
    // Alpha has 1 pass in R1, Bravo has 0. Asc: Bravo (0) first, Alpha (1) last.
    render(passFailGridViewer.render(mkData(), {}));
    const colHeader = screen.getByTestId('header-col-R1');
    fireEvent.click(colHeader);
    expect(colHeader).toHaveAttribute('aria-sort', 'ascending');
    const rows = screen.getAllByRole('row').slice(1);
    const first = rows[0] as HTMLElement;
    expect(first.getAttribute('data-testid')).toBe('row-Bravo');

    // Click again → desc → Alpha first.
    fireEvent.click(colHeader);
    expect(colHeader).toHaveAttribute('aria-sort', 'descending');
    const rows2 = screen.getAllByRole('row').slice(1);
    expect((rows2[0] as HTMLElement).getAttribute('data-testid')).toBe('row-Alpha');
  });
});

// ── Empty state ────────────────────────────────────────────────────

describe('PassFailGridViewer — empty state', () => {
  it('renders the guidance string when verdicts is empty', () => {
    render(passFailGridViewer.render({ kind: 'pass-fail-grid', verdicts: [] }, {}));
    const empty = screen.getByTestId('pass-fail-grid-empty');
    expect(empty).toBeInTheDocument();
    expect(empty).toHaveTextContent('Run verification to populate matrix');
  });

  it('does not render the summary bar in the empty state', () => {
    render(passFailGridViewer.render({ kind: 'pass-fail-grid', verdicts: [] }, {}));
    expect(screen.queryByTestId('pass-fail-grid-summary')).not.toBeInTheDocument();
  });
});
