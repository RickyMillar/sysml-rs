/**
 * Tests for SweepTableViewer (R5.3).
 *
 * Covers the streaming-settle contract:
 *   1. `accepts` narrows `ResultData` to `SweepTableData`.
 *   2. Rows render one-per-ChildDescriptor with every swept parameter surfaced.
 *   3. `pending` / `running` rows display the skeleton loader (not the metric).
 *   4. `failed` rows display the red failure reason, not the skeleton / metric.
 *   5. `complete` rows display the numeric metric and the rolled-up verdict.
 *   6. Row-click fires `onChildSelect` with the underlying ChildDescriptor.
 *   7. Header clicks toggle the sort direction.
 *   8. Empty input renders the guidance empty state.
 */
import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { sweepTableViewer, type SweepTableData } from '../SweepTableViewer';
import type { Verdict } from '../../../engine/types';
import type { ChildDescriptor } from '../sweepViewerHelpers';
import type { ResultData } from '../types';

afterEach(() => {
  cleanup();
});

// ── Fixtures ───────────────────────────────────────────────────────

function mkVerdict(v: Verdict['verdict'], margin?: number | null): Verdict {
  return { verdict: v, margin: margin ?? null };
}

function mkChild(
  index: number,
  params: Record<string, unknown>,
  verdicts: Verdict[] = [],
  status: ChildDescriptor['status'] = 'complete',
  reason?: string | null,
): ChildDescriptor {
  return {
    session_id: `s${index}`,
    index,
    params,
    status,
    verdicts,
    reason: reason ?? null,
  };
}

// ── accepts ────────────────────────────────────────────────────────

describe('sweepTableViewer.accepts', () => {
  it('narrows to sweep-table payload', () => {
    const payload: ResultData = { kind: 'sweep-table' } as ResultData;
    expect(sweepTableViewer.accepts(payload)).toBe(true);
  });
  it('rejects other kinds', () => {
    expect(sweepTableViewer.accepts({ kind: 'time-series' } as ResultData)).toBe(false);
    expect(sweepTableViewer.accepts({ kind: 'pass-fail-grid' } as ResultData)).toBe(false);
  });
});

// ── Renders rows + exposes parameter columns ───────────────────────

describe('sweepTableViewer render', () => {
  it('renders one row per ChildDescriptor with every swept parameter', () => {
    const data: SweepTableData = {
      kind: 'sweep-table',
      children: [
        mkChild(0, { gain: 1, delay: 5 }, [mkVerdict('pass', 2)]),
        mkChild(1, { gain: 2, delay: 5 }, [mkVerdict('fail', -1)]),
      ],
      config: { metric: 'margin' },
    };
    render(<>{sweepTableViewer.render(data, {})}</>);

    // Both rows present.
    expect(screen.getByTestId('sweep-row-0')).toBeTruthy();
    expect(screen.getByTestId('sweep-row-1')).toBeTruthy();

    // Param headers rendered in insertion order.
    expect(screen.getByTestId('header-param-gain')).toBeTruthy();
    expect(screen.getByTestId('header-param-delay')).toBeTruthy();

    // Metric values rendered for complete rows.
    expect(screen.getByTestId('sweep-metric-0').textContent).toBe('2');
    expect(screen.getByTestId('sweep-metric-1').textContent).toBe('-1');
  });

  it('pending rows show the skeleton loader', () => {
    const data: SweepTableData = {
      kind: 'sweep-table',
      children: [mkChild(0, { gain: 1 }, [], 'pending')],
    };
    render(<>{sweepTableViewer.render(data, {})}</>);

    expect(screen.getByTestId('sweep-skeleton-0')).toBeTruthy();
    // No finite metric rendered while pending.
    expect(screen.queryByTestId('sweep-metric-0')).toBeNull();
  });

  it('running rows also show the skeleton loader (running variant)', () => {
    const data: SweepTableData = {
      kind: 'sweep-table',
      children: [mkChild(0, { gain: 1 }, [], 'running')],
    };
    render(<>{sweepTableViewer.render(data, {})}</>);

    const skel = screen.getByTestId('sweep-skeleton-0');
    expect(skel.className).toContain('running');
  });

  it('failed rows render the failure reason', () => {
    const data: SweepTableData = {
      kind: 'sweep-table',
      children: [mkChild(0, { gain: 1 }, [], 'failed', 'solver diverged')],
    };
    render(<>{sweepTableViewer.render(data, {})}</>);

    const fail = screen.getByTestId('sweep-failure-0');
    expect(fail.textContent).toBe('solver diverged');
  });

  it('rolled-up verdict is shown when present', () => {
    const data: SweepTableData = {
      kind: 'sweep-table',
      children: [mkChild(0, { p: 1 }, [mkVerdict('pass'), mkVerdict('fail')])],
    };
    render(<>{sweepTableViewer.render(data, {})}</>);
    const row = screen.getByTestId('sweep-row-0');
    // The fail rollup dominates pass — rendered as a coloured span.
    const verdict = row.querySelector('[data-verdict-kind]');
    expect(verdict?.getAttribute('data-verdict-kind')).toBe('fail');
  });

  it('fires onChildSelect on row click with the right descriptor', () => {
    const seen: string[] = [];
    const data: SweepTableData = {
      kind: 'sweep-table',
      children: [mkChild(7, { p: 1 }, [mkVerdict('pass', 1)])],
      config: {
        onChildSelect: (c) => seen.push(c.session_id),
      },
    };
    render(<>{sweepTableViewer.render(data, {})}</>);
    fireEvent.click(screen.getByTestId('sweep-row-7'));
    expect(seen).toEqual(['s7']);
  });

  it('clicking a header toggles the sort direction', () => {
    const data: SweepTableData = {
      kind: 'sweep-table',
      children: [
        mkChild(0, { p: 1 }, [mkVerdict('pass', 1)]),
        mkChild(1, { p: 2 }, [mkVerdict('pass', 5)]),
      ],
      config: { metric: 'margin' },
    };
    render(<>{sweepTableViewer.render(data, {})}</>);

    const header = screen.getByTestId('header-metric');
    // Default sort: asc by index — descending by metric after two clicks.
    fireEvent.click(header);
    expect(header.getAttribute('aria-sort')).toBe('ascending');
    fireEvent.click(header);
    expect(header.getAttribute('aria-sort')).toBe('descending');
  });

  it('renders the empty-state when no children are provided', () => {
    const data: SweepTableData = { kind: 'sweep-table', children: [] };
    render(<>{sweepTableViewer.render(data, {})}</>);
    expect(screen.getByTestId('sweep-table-empty')).toBeTruthy();
  });
});
