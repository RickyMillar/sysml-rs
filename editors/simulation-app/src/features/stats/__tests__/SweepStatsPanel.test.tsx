/**
 * Tests for <SweepStatsPanel> (R7.2).
 *
 * Covers 1-D and 2-D sweep shapes, the sparse-cell placeholder, and the
 * empty-state fallback. Uses the same `ChildDescriptor` shape as the
 * R5.2/R5.3 sweep helpers.
 */
import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { SweepStatsPanel } from '../SweepStatsPanel';
import type { ChildDescriptor } from '../../../shared/viewers/sweepViewerHelpers';
import { createSeededRng } from '../statsHelpers';

afterEach(() => {
  cleanup();
});

function mk(
  index: number,
  params: Record<string, unknown>,
  metricValue: number,
): ChildDescriptor {
  return {
    session_id: `s-${index}`,
    index,
    params,
    status: 'complete',
    verdicts: [{ verdict: 'pass', margin: metricValue }],
  };
}

describe('<SweepStatsPanel>', () => {
  it('1-D: renders a single row of stats per parameter × metric', () => {
    const kids: ChildDescriptor[] = [];
    for (let i = 0; i < 10; i++) kids.push(mk(i, { gain: i }, i * 0.5));
    render(
      <SweepStatsPanel
        children={kids}
        metrics={[
          {
            id: 'margin',
            label: 'Margin',
            extract: (c) => (typeof c.verdicts[0]?.margin === 'number' ? c.verdicts[0].margin : Number.NaN),
          },
        ]}
        rng={createSeededRng(1)}
      />,
    );
    expect(screen.getByTestId('sweep-stats-panel')).toBeInTheDocument();
    expect(screen.getByTestId('sweep-stats-param-gain')).toBeInTheDocument();
    expect(screen.getByTestId('sweep-stats-overlay-gain-margin')).toBeInTheDocument();
  });

  it('2-D: renders a matrix with one cell per (x, y)', () => {
    const kids: ChildDescriptor[] = [];
    // 3 replicas per cell to clear `minCellSamples=2`.
    const xs = [1, 2];
    const ys = [10, 20];
    let idx = 0;
    for (const x of xs) {
      for (const y of ys) {
        for (let r = 0; r < 3; r++) {
          kids.push(mk(idx++, { A: x, B: y }, x * y + r));
        }
      }
    }
    render(
      <SweepStatsPanel
        children={kids}
        metrics={[
          {
            id: 'margin',
            extract: (c) => (typeof c.verdicts[0]?.margin === 'number' ? c.verdicts[0].margin : Number.NaN),
          },
        ]}
        parameters={['A', 'B']}
        rng={createSeededRng(7)}
      />,
    );
    expect(screen.getByTestId('sweep-stats-matrix')).toBeInTheDocument();
    // One overlay per full cell.
    expect(screen.getByTestId('sweep-stats-matrix-cell-margin-1-10')).toBeInTheDocument();
    expect(screen.getByTestId('sweep-stats-matrix-cell-margin-2-20')).toBeInTheDocument();
  });

  it('2-D: under-occupied cells render a sparse placeholder', () => {
    const kids: ChildDescriptor[] = [
      mk(0, { A: 1, B: 10 }, 1),
      mk(1, { A: 2, B: 20 }, 2),
      mk(2, { A: 2, B: 20 }, 3),
      mk(3, { A: 2, B: 20 }, 4),
    ];
    render(
      <SweepStatsPanel
        children={kids}
        metrics={[
          {
            id: 'margin',
            extract: (c) => (typeof c.verdicts[0]?.margin === 'number' ? c.verdicts[0].margin : Number.NaN),
          },
        ]}
        parameters={['A', 'B']}
        rng={createSeededRng(3)}
      />,
    );
    // (A=1, B=10) has only one sample → sparse placeholder.
    expect(
      screen.getByTestId('sweep-stats-matrix-cell-margin-1-10-sparse'),
    ).toBeInTheDocument();
    // (A=2, B=20) has three samples → full overlay.
    expect(screen.getByTestId('sweep-stats-matrix-cell-margin-2-20')).toBeInTheDocument();
  });

  it('renders empty state with no metrics', () => {
    render(<SweepStatsPanel children={[]} metrics={[]} />);
    expect(screen.getByTestId('sweep-stats-panel-empty')).toBeInTheDocument();
  });
});
