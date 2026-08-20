/**
 * SwimlaneTimeline — the axis is REAL TICKS, not entry indices.
 *
 * The J3 timeline finding: a severe pump run whose relief the runtime records
 * at tick 3819 was labelled "tick 7" in the UI, because 7 was the index of the
 * sampled entry in which the client first observed `relieved`. Entries come
 * from `useStateTimelineIngest`, which appends one per poll where the state map
 * changed — so index equals tick only if the run is stepped exactly one tick
 * per poll, which a bulk step never is.
 *
 * Two defects, one cause. The tooltip printed an index under the word "tick",
 * and the axis was index-scaled (`plotW / entries.length`) while triggers,
 * fragments, successions and the playhead were positioned with real ticks
 * against it — so the annotations and the blocks were in different coordinate
 * systems on the same axis.
 *
 * These entries mirror a real severe run's shape: sparse samples with large,
 * uneven tick gaps.
 */
import { describe, expect, it } from 'vitest';
import { render } from '@testing-library/react';
import { SwimlaneTimeline } from '../SwimlaneTimeline';

/** Sampled like a real bulk-stepped run: 8 entries spanning ticks 1..4001. */
const ENTRIES = [
  { tick: 1, timeMs: 1, subsystems: { PumpCycle: 'intake' } },
  { tick: 1002, timeMs: 1002, subsystems: { PumpCycle: 'compress' } },
  { tick: 1750, timeMs: 1750, subsystems: { PumpCycle: 'discharge' } },
  { tick: 2500, timeMs: 2500, subsystems: { PumpCycle: 'recover' } },
  { tick: 3002, timeMs: 3002, subsystems: { PumpCycle: 'intake' } },
  { tick: 3250, timeMs: 3250, subsystems: { PumpCycle: 'compress' } },
  { tick: 3500, timeMs: 3500, subsystems: { PumpCycle: 'discharge' } },
  { tick: 3819, timeMs: 3819, subsystems: { PumpCycle: 'relieved' } },
];

const WIDTH = 600;
const PAD_LEFT = 80;
const PAD_RIGHT = 8;
const PLOT_W = WIDTH - PAD_LEFT - PAD_RIGHT;

/** The x the component should compute for a tick, given this entry set. */
function expectedX(tick: number): number {
  const first = ENTRIES[0].tick;
  const last = ENTRIES[ENTRIES.length - 1].tick;
  return PAD_LEFT + ((tick - first) * PLOT_W) / (last - first);
}

function titles(container: HTMLElement): string[] {
  return [...container.querySelectorAll('title')].map((t) => t.textContent ?? '');
}

describe('SwimlaneTimeline — tick-space axis', () => {
  it('labels blocks with the runtime tick, not the entry index', () => {
    const { container } = render(<SwimlaneTimeline entries={ENTRIES} width={WIDTH} />);
    const relieved = titles(container).find((t) => t.includes('relieved'));
    expect(relieved).toBeDefined();

    // The regression in one assertion: `relieved` is entry index 7 and runtime
    // tick 3819. It must read as the tick.
    expect(relieved).toContain('tick 3819');
    expect(relieved).not.toMatch(/tick 7\b/);
  });

  it('spans a state until the sample that changed it, never reporting 0ms', () => {
    const { container } = render(<SwimlaneTimeline entries={ENTRIES} width={WIDTH} />);
    // Every one of these states is observed in exactly ONE sample. Closing a
    // block on its own last sample gave all of them a zero duration — the
    // "0 ms" in the J3 report. A sampled state holds until the sample that
    // changed it.
    for (const t of titles(container).filter((x) => x.includes('PumpCycle'))) {
      expect(t).not.toMatch(/\b0ms\b/);
    }
    // The state the run ended in has no closing sample, so its duration is
    // UNKNOWN. Reporting 0ms there is the same lie in the other direction.
    const relieved = titles(container).find((t) => t.includes('relieved'));
    expect(relieved).toContain('from tick 3819');
    expect(relieved).toContain('still current');
    // `discharge` first opens at tick 1750 and is gone by the 2500 sample.
    const discharge = titles(container).find((t) => t.includes('discharge'));
    expect(discharge).toContain('tick 1750');
    expect(discharge).toContain('750ms');
    // And the bounds are labelled as observations, not as the exact transition.
    expect(discharge).toContain('sampled');
  });

  it('positions a block by tick, so uneven sample gaps render at their real spacing', () => {
    const { container } = render(<SwimlaneTimeline entries={ENTRIES} width={WIDTH} />);
    const rects = [...container.querySelectorAll('rect')].filter((r) =>
      (r.querySelector('title')?.textContent ?? '').includes('PumpCycle'),
    );
    const first = rects[0];
    expect(first).toBeDefined();
    // The first block starts at the first sampled tick, i.e. the axis origin.
    expect(Number(first.getAttribute('x'))).toBeCloseTo(expectedX(1), 1);

    // The last block (relieved, tick 3819) sits at the far end. Under the old
    // index axis it would have been at index 7 of 8 — a completely different x.
    const last = rects[rects.length - 1];
    expect(Number(last.getAttribute('x'))).toBeCloseTo(expectedX(3819), 1);
  });

  it('draws the playhead in the same coordinate system as the blocks', () => {
    // The clearest symptom of the mixed axis: `currentTick` is a real tick, so
    // against an index axis (plotW / 8 per unit) tick 3819 landed ~3800 lanes
    // off the right edge of a 600px chart.
    const { container } = render(
      <SwimlaneTimeline entries={ENTRIES} width={WIDTH} currentTick={3819} />,
    );
    const playhead = [...container.querySelectorAll('line')].find(
      (l) => l.getAttribute('stroke-width') === '1.5',
    );
    expect(playhead).toBeDefined();
    const x = Number(playhead!.getAttribute('x1'));
    expect(x).toBeCloseTo(expectedX(3819), 1);
    expect(x).toBeLessThanOrEqual(WIDTH - PAD_RIGHT);
  });

  it('places a trigger annotation at its own tick', () => {
    const { container } = render(
      <SwimlaneTimeline
        entries={ENTRIES}
        width={WIDTH}
        triggers={[
          {
            subsystem: 'PumpCycle',
            tick: 3819,
            event: 'exposure > exposureTrip',
            fromState: 'discharge',
            toState: 'relieved',
          },
        ]}
      />,
    );
    const marker = [...container.querySelectorAll('polygon')].find((p) =>
      (p.querySelector('title')?.textContent ?? '').includes('exposureTrip'),
    );
    expect(marker).toBeDefined();
    // Diamond is centred on the tick.
    const cx = Number(marker!.getAttribute('points')!.split(' ')[0].split(',')[0]);
    expect(cx).toBeCloseTo(expectedX(3819), 1);
  });

  it('does not divide by zero when every sample shares one tick', () => {
    const flat = [
      { tick: 5, timeMs: 5, subsystems: { PumpCycle: 'intake' } },
      { tick: 5, timeMs: 5, subsystems: { PumpCycle: 'compress' } },
    ];
    expect(() => render(<SwimlaneTimeline entries={flat} width={WIDTH} />)).not.toThrow();
  });
});
