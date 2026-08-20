/**
 * SessionCounter — status bar P7 chip.
 *
 * Tests the small, self-contained counter/popover in isolation so we
 * don't have to mock the five hooks the parent SessionStatusBar uses.
 */

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { SessionCounter } from '../SessionStatusBar';

afterEach(() => {
  cleanup();
});

describe('SessionCounter — label format', () => {
  it('renders "N active · M free" when cap is positive', () => {
    render(
      <SessionCounter
        used={2}
        cap={80}
        breakdown={{ simulation: { used: 2, cap: 80 }, action: null, orchestrator: null }}
      />,
    );
    const btn = screen.getByTestId('session-counter');
    expect(btn).toHaveTextContent('2 active');
    expect(btn).toHaveTextContent('78 free');
    expect(btn).toHaveAttribute('data-used', '2');
    expect(btn).toHaveAttribute('data-cap', '80');
  });

  it('falls back to "N active" when cap is 0 (quota disabled)', () => {
    render(
      <SessionCounter
        used={3}
        cap={0}
        breakdown={{ simulation: null, action: null, orchestrator: null }}
      />,
    );
    expect(screen.getByTestId('session-counter')).toHaveTextContent('3 active');
    expect(screen.getByTestId('session-counter')).not.toHaveTextContent('free');
  });

  it('clamps negative free count to 0 when used > cap (over-quota)', () => {
    render(
      <SessionCounter
        used={85}
        cap={80}
        breakdown={{ simulation: null, action: null, orchestrator: null }}
      />,
    );
    expect(screen.getByTestId('session-counter')).toHaveTextContent('85 active');
    expect(screen.getByTestId('session-counter')).toHaveTextContent('0 free');
  });
});

describe('SessionCounter — popover', () => {
  it('is closed by default (aria-expanded=false, no popover in DOM)', () => {
    render(
      <SessionCounter
        used={1}
        cap={80}
        breakdown={{ simulation: { used: 1, cap: 80 }, action: null, orchestrator: null }}
      />,
    );
    expect(screen.getByTestId('session-counter')).toHaveAttribute(
      'aria-expanded',
      'false',
    );
    expect(screen.queryByTestId('session-counter-popover')).toBeNull();
  });

  it('opens the popover on click and renders the per-kind breakdown', () => {
    render(
      <SessionCounter
        used={3}
        cap={80}
        breakdown={{
          simulation: { used: 2, cap: 60 },
          action: { used: 1, cap: 10 },
          orchestrator: { used: 0, cap: 10 },
        }}
      />,
    );
    fireEvent.click(screen.getByTestId('session-counter'));
    expect(screen.getByTestId('session-counter-popover')).toBeInTheDocument();
    expect(screen.getByTestId('session-counter-row-simulation')).toHaveTextContent(
      '2 / 60',
    );
    expect(screen.getByTestId('session-counter-row-action')).toHaveTextContent(
      '1 / 10',
    );
    expect(screen.getByTestId('session-counter-row-orchestrator')).toHaveTextContent(
      '0 / 10',
    );
    // Reaper hint is shown so users know the counter self-heals.
    expect(screen.getByTestId('session-counter-popover')).toHaveTextContent(
      /auto-reap/i,
    );
  });

  it('toggles closed on second click (aria-expanded back to false)', () => {
    render(
      <SessionCounter
        used={1}
        cap={80}
        breakdown={{ simulation: { used: 1, cap: 80 }, action: null, orchestrator: null }}
      />,
    );
    const btn = screen.getByTestId('session-counter');
    fireEvent.click(btn);
    expect(btn).toHaveAttribute('aria-expanded', 'true');
    fireEvent.click(btn);
    expect(btn).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByTestId('session-counter-popover')).toBeNull();
  });

  it('skips null buckets in the breakdown (only shows kinds the backend reports)', () => {
    render(
      <SessionCounter
        used={1}
        cap={80}
        breakdown={{
          simulation: { used: 1, cap: 80 },
          action: null,
          orchestrator: null,
        }}
      />,
    );
    fireEvent.click(screen.getByTestId('session-counter'));
    expect(
      screen.getByTestId('session-counter-row-simulation'),
    ).toBeInTheDocument();
    expect(screen.queryByTestId('session-counter-row-action')).toBeNull();
    expect(screen.queryByTestId('session-counter-row-orchestrator')).toBeNull();
  });
});
