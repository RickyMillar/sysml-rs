/**
 * Tests for <MonteCarloStatsPanel> (R7.2).
 *
 * The panel is a thin composition of per-outcome <StatsOverlay>
 * instances — these tests pin its behaviour over the MC viewer's
 * `ChildDescriptor[]` shape and confirm:
 *
 *   - Per-outcome overlay mounts with the expected test id.
 *   - Non-finite extractor outputs are filtered before stats computation.
 *   - Empty outcome list renders the empty state.
 */
import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { MonteCarloStatsPanel } from '../MonteCarloStatsPanel';
import type { ChildDescriptor } from '../../../workflows/analyze/montecarlo/passRateHelpers';
import { createSeededRng } from '../statsHelpers';

afterEach(() => {
  cleanup();
});

function mkChild(index: number, tripTime: number): ChildDescriptor {
  return {
    index,
    session_id: `s-${index}`,
    status: 'complete',
    params: {},
    metrics: { trip_time: tripTime },
  };
}

describe('<MonteCarloStatsPanel>', () => {
  it('renders one overlay per outcome', () => {
    const rng = createSeededRng(42);
    const kids: ChildDescriptor[] = [];
    for (let i = 0; i < 50; i++) kids.push(mkChild(i, 1 + rng()));
    render(
      <MonteCarloStatsPanel
        children={kids}
        outcomes={[
          {
            id: 'trip_time',
            label: 'Trip time',
            unit: 's',
            extract: (c) => {
              const m = c.metrics?.trip_time;
              return typeof m === 'number' ? m : Number.NaN;
            },
          },
        ]}
        rng={createSeededRng(1)}
      />,
    );
    expect(screen.getByTestId('mc-stats-panel')).toBeInTheDocument();
    expect(screen.getByTestId('mc-stats-overlay-trip_time')).toBeInTheDocument();
  });

  it('filters non-finite extractor outputs', () => {
    const kids: ChildDescriptor[] = [
      mkChild(0, 1),
      mkChild(1, Number.NaN),
      { index: 2, status: 'pending', params: {} },
      mkChild(3, 3),
      mkChild(4, 5),
    ];
    render(
      <MonteCarloStatsPanel
        children={kids}
        outcomes={[
          {
            id: 'trip_time',
            extract: (c) => {
              const m = c.metrics?.trip_time;
              return typeof m === 'number' ? m : Number.NaN;
            },
          },
        ]}
        rng={createSeededRng(2)}
      />,
    );
    // Mean of [1, 3, 5] = 3.
    const overlay = screen.getByTestId('mc-stats-overlay-trip_time');
    expect(overlay.textContent).toMatch(/3\./);
  });

  it('renders empty state when no outcomes configured', () => {
    render(<MonteCarloStatsPanel children={[]} outcomes={[]} />);
    expect(screen.getByTestId('mc-stats-panel-empty')).toBeInTheDocument();
  });
});
