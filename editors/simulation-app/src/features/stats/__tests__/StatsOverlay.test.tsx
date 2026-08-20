/**
 * Tests for the StatsOverlay component (R7.2).
 *
 * These cover:
 *   - Renders the core metrics (mean, σ, CI, skew, kurtosis, SEM).
 *   - Distribution chip reflects the best-fit family.
 *   - Q-Q plot mounts when `showQQ` is true and hides otherwise.
 *   - Empty / NaN-filled samples degrade to dashes gracefully.
 */
import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, render, screen, within } from '@testing-library/react';
import { StatsOverlay } from '../StatsOverlay';
import { createSeededRng } from '../statsHelpers';

afterEach(() => {
  cleanup();
});

function normalBatch(n: number, mu: number, sigma: number, seed = 123): number[] {
  const rng = createSeededRng(seed);
  const out: number[] = [];
  while (out.length < n) {
    const u1 = Math.max(rng(), 1e-12);
    const u2 = rng();
    const r = Math.sqrt(-2 * Math.log(u1));
    const theta = 2 * Math.PI * u2;
    out.push(mu + sigma * r * Math.cos(theta));
    if (out.length < n) out.push(mu + sigma * r * Math.sin(theta));
  }
  return out;
}

describe('<StatsOverlay>', () => {
  it('renders mean, σ, CI, skew, kurtosis, SEM on a normal sample', () => {
    const v = normalBatch(200, 5, 1, 42);
    render(<StatsOverlay values={v} label="trip_time" unit="s" />);
    const metrics = screen.getByTestId('stats-overlay-metrics');
    within(metrics).getByTestId('stats-overlay-mean');
    within(metrics).getByTestId('stats-overlay-ci');
    within(metrics).getByTestId('stats-overlay-sigma');
    within(metrics).getByTestId('stats-overlay-skew');
    within(metrics).getByTestId('stats-overlay-kurtosis');
    within(metrics).getByTestId('stats-overlay-sem');

    const meanCell = within(metrics).getByTestId('stats-overlay-mean');
    expect(meanCell.textContent).toMatch(/[0-9]/);
    expect(meanCell.textContent).toContain('s');
  });

  it('renders the fit chip with family label', () => {
    const v = normalBatch(500, 0, 1, 7);
    render(<StatsOverlay values={v} label="x" />);
    const chip = screen.getByTestId('stats-overlay-fit-chip');
    expect(chip.textContent?.toLowerCase()).toContain('normal');
  });

  it('mounts the Q-Q plot when showQQ is true (default)', () => {
    const v = normalBatch(50, 0, 1, 13);
    render(<StatsOverlay values={v} label="x" />);
    expect(screen.getByTestId('stats-overlay-qq')).toBeInTheDocument();
    expect(screen.getByTestId('stats-overlay-qq-plot')).toBeInTheDocument();
  });

  it('hides the Q-Q plot when showQQ is false', () => {
    const v = normalBatch(50, 0, 1, 14);
    render(<StatsOverlay values={v} label="x" showQQ={false} />);
    expect(screen.queryByTestId('stats-overlay-qq')).toBeNull();
  });

  it('handles empty input with dashes', () => {
    render(<StatsOverlay values={[]} label="x" />);
    const meanCell = screen.getByTestId('stats-overlay-mean');
    expect(meanCell.textContent).toContain('—');
    const ciCell = screen.getByTestId('stats-overlay-ci');
    expect(ciCell.textContent).toContain('—');
  });

  it('filters NaN / infinite input before computing', () => {
    const v = [1, 2, Number.NaN, 3, Number.POSITIVE_INFINITY, 4];
    render(<StatsOverlay values={v} label="x" />);
    const meanCell = screen.getByTestId('stats-overlay-mean');
    // Mean of [1,2,3,4] = 2.5
    expect(meanCell.textContent).toMatch(/2\.5/);
  });

  it('honours caller-supplied RNG for deterministic CI bounds', () => {
    const v = normalBatch(50, 0, 1, 99);
    const rng1 = createSeededRng(42);
    const rng2 = createSeededRng(42);
    const { unmount } = render(
      <StatsOverlay values={v} label="x" rng={rng1} testId="first" />,
    );
    const firstCi = screen.getByTestId('first').querySelector('[data-testid="stats-overlay-ci"]')!.textContent;
    unmount();
    render(<StatsOverlay values={v} label="x" rng={rng2} testId="second" />);
    const secondCi = screen.getByTestId('second').querySelector('[data-testid="stats-overlay-ci"]')!.textContent;
    expect(firstCi).toBe(secondCi);
  });
});
