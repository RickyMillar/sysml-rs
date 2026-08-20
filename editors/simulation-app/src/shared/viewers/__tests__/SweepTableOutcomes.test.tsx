/**
 * The sweep Table shows what the study measured.
 *
 * The evidence campaign that opened this work ran a five-point `ambientTemp`
 * sweep with `temperature` selected, watched all five children complete, and
 * found a table with columns `#`, `ambientTemp`, `Status`, `Verdict`, `Fails`
 * — and nowhere for the outcome to appear. The run was fine; the table simply
 * had no column for the thing the study was about.
 *
 * The unavailable case matters as much as the value case: a child that could
 * not produce the outcome has to say so, because rendering it as `0` would
 * put a fabricated point in the middle of a temperature column.
 */

import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { sweepTableViewer, type SweepTableData } from '../SweepTableViewer';
import type { ChildDescriptor } from '../sweepViewerHelpers';

// `globals: false` in vitest.config.ts means RTL's auto-cleanup never
// registers, so each render would stack in the same document.
afterEach(cleanup);

function complete(
  index: number,
  ambientTemp: number,
  outcomes?: ChildDescriptor['outcomes'],
): ChildDescriptor {
  return {
    session_id: `s${index}`,
    index,
    params: { ambientTemp },
    status: 'complete',
    verdicts: [],
    ...(outcomes ? { outcomes } : {}),
  };
}

/** The measured five-point sweep, as the backend now returns it. */
const FIVE_POINT: ChildDescriptor[] = [
  complete(0, 250, { temperature: { value: 990.0365673176574, time_ms: 1000 } }),
  complete(1, 275, { temperature: { value: 990.0547044097054, time_ms: 1000 } }),
  complete(2, 300, { temperature: { value: 990.0785237337283, time_ms: 1000 } }),
  complete(3, 325, { temperature: { value: 990.1091038824774, time_ms: 1000 } }),
  complete(4, 350, { temperature: { value: 990.1476172303682, time_ms: 1000 } }),
];

function renderTable(children: ChildDescriptor[]) {
  const data: SweepTableData = { kind: 'sweep-table', children };
  return render(<>{sweepTableViewer.render(data, { height: 400 })}</>);
}

describe('SweepTableViewer — outcome columns', () => {
  it('gives the measured outcome its own column', () => {
    renderTable(FIVE_POINT);
    expect(screen.getByTestId('header-outcome-temperature')).toBeInTheDocument();
  });

  it('shows five attributable temperature values, one per swept point', () => {
    renderTable(FIVE_POINT);
    const shown = FIVE_POINT.map(
      (c) => screen.getByTestId(`sweep-outcome-value-${c.index}-temperature`).textContent,
    );
    expect(shown).toHaveLength(5);
    // Each row carries its own reading — not one value repeated.
    expect(new Set(shown).size).toBe(5);
    expect(shown[0]).toContain('990.03');
    expect(shown[4]).toContain('990.14');
  });

  it('carries the unit into the header when the model declared one', () => {
    renderTable([complete(0, 250, { temperature: { value: 990, unit: 'K' } })]);
    expect(screen.getByTestId('header-outcome-temperature').textContent).toContain('(K)');
  });

  it('says "unavailable" instead of inventing a number', () => {
    renderTable([
      complete(0, 250, { temperature: { value: 990.03, time_ms: 1000 } }),
      complete(1, 275, {
        temperature: { error: "'temperature' was not recorded by this run" },
      }),
    ]);
    const cell = screen.getByTestId('sweep-outcome-unavailable-1-temperature');
    expect(cell.textContent).toBe('unavailable');
    expect(cell.textContent).not.toContain('0');
    // The reason survives to the user, on hover.
    expect(cell.getAttribute('title')).toContain('not recorded');
    // ...and the readable sibling is unaffected.
    expect(screen.getByTestId('sweep-outcome-value-0-temperature')).toBeInTheDocument();
  });

  it('renders a genuine zero as a value, not as unavailable', () => {
    renderTable([complete(0, 250, { temperature: { value: 0, time_ms: 1000 } })]);
    expect(screen.getByTestId('sweep-outcome-value-0-temperature').textContent).toBe('0');
    expect(screen.queryByTestId('sweep-outcome-unavailable-0-temperature')).toBeNull();
  });

  it('adds no outcome column when the study measured nothing', () => {
    renderTable([complete(0, 250)]);
    expect(screen.queryByTestId('header-outcome-temperature')).toBeNull();
    // The pre-existing columns are untouched.
    expect(screen.getByTestId('header-status')).toBeInTheDocument();
    expect(screen.getByTestId('header-verdict')).toBeInTheDocument();
    expect(screen.getByTestId('header-metric')).toBeInTheDocument();
  });

  it('keeps Status, Verdict and the metric column alongside the outcome', () => {
    renderTable(FIVE_POINT);
    expect(screen.getByTestId('header-param-ambientTemp')).toBeInTheDocument();
    expect(screen.getByTestId('header-outcome-temperature')).toBeInTheDocument();
    expect(screen.getByTestId('header-status')).toBeInTheDocument();
    expect(screen.getByTestId('header-verdict')).toBeInTheDocument();
    expect(screen.getByTestId('header-metric')).toBeInTheDocument();
  });
});

describe('SweepTableViewer — the shape beside the number', () => {
  /** Two rows: one that genuinely cooled, one that never moved. */
  const MOVED: [number, number][] = Array.from({ length: 12 }, (_, i) => [
    i * 100_000,
    300 + 700 * Math.exp(-i / 3),
  ]);
  const FLAT: [number, number][] = Array.from({ length: 12 }, (_, i) => [i * 100, 990.0367]);

  it('draws a trace alongside the value', () => {
    renderTable([
      complete(0, 250, { temperature: { value: 314.2, time_ms: 2e6, series: MOVED } }),
    ]);
    expect(screen.getByTestId('sweep-outcome-spark-0-temperature')).toBeInTheDocument();
    expect(screen.getByTestId('sweep-outcome-value-0-temperature').textContent).toContain('314.2');
  });

  it('distinguishes a run that settled from one that never moved', () => {
    // Same column, same status, near-identical numbers — the shape is the
    // only thing that separates them, which is the whole point.
    renderTable([
      complete(0, 250, { temperature: { value: 314.2, time_ms: 2e6, series: MOVED } }),
      complete(1, 275, { temperature: { value: 990.0367, time_ms: 1000, series: FLAT } }),
    ]);
    const settled = screen.getByTestId('sweep-outcome-value-0-temperature');
    const stalled = screen.getByTestId('sweep-outcome-value-1-temperature');
    expect(settled.getAttribute('title')).toContain('→');
    expect(stalled.getAttribute('title')).toMatch(/^flat at/);
  });

  it('still shows the number when no trace came back', () => {
    // Older records and short runs carry no series; the cell must not
    // degrade because the decoration is missing.
    renderTable([complete(0, 250, { temperature: { value: 990.03, time_ms: 1000 } })]);
    expect(screen.getByTestId('sweep-outcome-value-0-temperature').textContent).toContain('990.03');
    expect(screen.queryByTestId('sweep-outcome-spark-0-temperature')).toBeNull();
  });

  it('draws no trace on an unavailable outcome', () => {
    renderTable([complete(0, 250, { temperature: { error: 'not recorded' } })]);
    expect(screen.queryByTestId('sweep-outcome-spark-0-temperature')).toBeNull();
    expect(screen.getByTestId('sweep-outcome-unavailable-0-temperature')).toBeInTheDocument();
  });
});
