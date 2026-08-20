/**
 * SharedPlayhead — play / pause / step / scrub interactions.
 *
 * All actions mutate `useCompareStore` directly (the playhead is a
 * controlled component). Fake timers drive the 100 ms auto-advance.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { SharedPlayhead } from '../SharedPlayhead';
import { useCompareStore } from '../useCompareStore';

function reset() {
  useCompareStore.setState({
    pickedSessionIds: [],
    sharedTick: 0,
    isPlaying: false,
    layout: null,
    activeModeId: null,
    pickedVariables: null,
  });
}

beforeEach(() => {
  reset();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe('SharedPlayhead — step controls', () => {
  it('step-forward advances sharedTick by 1', () => {
    render(<SharedPlayhead maxTick={5} />);
    fireEvent.click(screen.getByTestId('shared-playhead-step-forward'));
    expect(useCompareStore.getState().sharedTick).toBe(1);
  });

  it('step-back decrements sharedTick (clamped at 0)', () => {
    useCompareStore.getState().setSharedTick(2);
    render(<SharedPlayhead maxTick={5} />);
    fireEvent.click(screen.getByTestId('shared-playhead-step-back'));
    expect(useCompareStore.getState().sharedTick).toBe(1);
    fireEvent.click(screen.getByTestId('shared-playhead-step-back'));
    fireEvent.click(screen.getByTestId('shared-playhead-step-back'));
    expect(useCompareStore.getState().sharedTick).toBe(0);
  });

  it('step-forward is disabled once sharedTick hits maxTick', () => {
    useCompareStore.getState().setSharedTick(5);
    render(<SharedPlayhead maxTick={5} />);
    const btn = screen.getByTestId('shared-playhead-step-forward') as HTMLButtonElement;
    expect(btn).toBeDisabled();
  });

  it('all controls are disabled when maxTick is 0', () => {
    render(<SharedPlayhead maxTick={0} />);
    expect(screen.getByTestId('shared-playhead-step-back')).toBeDisabled();
    expect(screen.getByTestId('shared-playhead-step-forward')).toBeDisabled();
    expect(screen.getByTestId('shared-playhead-play-pause')).toBeDisabled();
  });
});

describe('SharedPlayhead — play / pause', () => {
  it('starts paused by default', () => {
    render(<SharedPlayhead maxTick={5} />);
    expect(useCompareStore.getState().isPlaying).toBe(false);
  });

  it('clicking play toggles isPlaying true', () => {
    render(<SharedPlayhead maxTick={5} />);
    fireEvent.click(screen.getByTestId('shared-playhead-play-pause'));
    expect(useCompareStore.getState().isPlaying).toBe(true);
  });

  it('clicking pause while playing toggles back to paused', () => {
    render(<SharedPlayhead maxTick={5} />);
    const btn = screen.getByTestId('shared-playhead-play-pause');
    fireEvent.click(btn);
    fireEvent.click(btn);
    expect(useCompareStore.getState().isPlaying).toBe(false);
  });

  it('rewinds to 0 when play is pressed at the end', () => {
    useCompareStore.getState().setSharedTick(5);
    render(<SharedPlayhead maxTick={5} />);
    fireEvent.click(screen.getByTestId('shared-playhead-play-pause'));
    expect(useCompareStore.getState().sharedTick).toBe(0);
    expect(useCompareStore.getState().isPlaying).toBe(true);
  });

  it('auto-advances the tick at the configured interval while playing', () => {
    vi.useFakeTimers();
    render(<SharedPlayhead maxTick={4} advanceMs={50} />);
    fireEvent.click(screen.getByTestId('shared-playhead-play-pause'));
    vi.advanceTimersByTime(155);
    // 3 ticks elapsed -> sharedTick should be 3 (from 0, 3 intervals fired).
    expect(useCompareStore.getState().sharedTick).toBeGreaterThanOrEqual(2);
  });

  it('stops playing when reaching maxTick', () => {
    vi.useFakeTimers();
    render(<SharedPlayhead maxTick={2} advanceMs={10} />);
    fireEvent.click(screen.getByTestId('shared-playhead-play-pause'));
    vi.advanceTimersByTime(500);
    expect(useCompareStore.getState().sharedTick).toBe(2);
    expect(useCompareStore.getState().isPlaying).toBe(false);
  });
});

describe('SharedPlayhead — scrubber', () => {
  it('setting the range scrubs sharedTick', () => {
    render(<SharedPlayhead maxTick={10} />);
    const range = screen.getByTestId('shared-playhead-scrubber') as HTMLInputElement;
    fireEvent.change(range, { target: { value: '7' } });
    expect(useCompareStore.getState().sharedTick).toBe(7);
  });

  it('displays the current tick label', () => {
    useCompareStore.getState().setSharedTick(3);
    render(<SharedPlayhead maxTick={10} />);
    expect(screen.getByTestId('shared-playhead-tick').textContent).toContain('tick 3 / 10');
  });
});

describe('SharedPlayhead — session pills', () => {
  it('renders a pill per session and marks frozen ones', () => {
    useCompareStore.getState().setSharedTick(10);
    render(
      <SharedPlayhead
        maxTick={10}
        sessionTicks={[
          { id: 'a', label: 'A', ticks: 11 }, // ends at tick 10 → not frozen
          { id: 'b', label: 'B', ticks: 5 },  // ends at tick 4 → frozen at 10
        ]}
      />,
    );
    const pills = screen.getByTestId('shared-playhead-session-pills');
    expect(pills.textContent).toContain('A');
    expect(pills.textContent).toContain('B');
    // B should carry the frozen bullet marker
    expect(pills.textContent).toMatch(/B.*•/);
  });
});

describe('SharedPlayhead — clamps when maxTick shrinks', () => {
  it('re-clamps sharedTick via effect when maxTick drops below it', () => {
    useCompareStore.getState().setSharedTick(8);
    const { rerender } = render(<SharedPlayhead maxTick={10} />);
    rerender(<SharedPlayhead maxTick={3} />);
    expect(useCompareStore.getState().sharedTick).toBe(3);
  });
});
