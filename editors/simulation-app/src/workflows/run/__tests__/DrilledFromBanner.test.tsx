/**
 * Tests for DrilledFromBanner (R3.5; migrated to the multi-hop
 * `useInvestigationTrail` store in ninebar Phase 1, audit F15).
 *
 * Verifies the banner renders only when the trail has a hop in view,
 * shows the tick / element / session context for the current hop, shows
 * a clickable breadcrumb for earlier hops, and "Back" steps the cursor
 * back one hop (hiding the banner entirely once the trail is exhausted).
 */

import { afterEach, describe, expect, it } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';
import { DrilledFromBanner } from '../DrilledFromBanner';
import { useInvestigationTrail } from '@/features/investigation/useInvestigationTrail';

afterEach(() => {
  cleanup();
  useInvestigationTrail.getState().clear();
});

describe('DrilledFromBanner', () => {
  it('renders nothing when the trail is empty', () => {
    const { container } = render(<DrilledFromBanner />);
    expect(container.firstChild).toBeNull();
  });

  it('renders tick + element + session when a hop is in view', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'abcd1234-efgh-5678',
      tick: 482,
      elementId: 'groupHead.trip_time',
      label: 'Verify · tick 482',
    });
    render(<DrilledFromBanner />);
    expect(screen.getByTestId('drilled-from-banner')).toBeInTheDocument();
    expect(screen.getByTestId('drilled-from-label')).toHaveTextContent(
      'Drilled from Verify',
    );
    expect(screen.getByTestId('drilled-from-tick')).toHaveTextContent('tick 482');
    expect(screen.getByTestId('drilled-from-element')).toHaveTextContent(
      'groupHead.trip_time',
    );
    // Session shown truncated to first 8 chars.
    expect(screen.getByTestId('drilled-from-session')).toHaveTextContent(
      'session abcd1234',
    );
  });

  it('omits the tick chunk when tick is absent', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'sess-x',
      elementId: 'x.y',
      label: 'Verify',
    });
    render(<DrilledFromBanner />);
    expect(screen.queryByTestId('drilled-from-tick')).toBeNull();
  });

  it('omits the element chunk when elementId is absent', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'sess-x',
      tick: 10,
      label: 'Verify · tick 10',
    });
    render(<DrilledFromBanner />);
    expect(screen.queryByTestId('drilled-from-element')).toBeNull();
    expect(screen.getByTestId('drilled-from-tick')).toHaveTextContent('tick 10');
  });

  it('omits the session chunk when fromSessionId is empty', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: '',
      tick: 3,
      elementId: 'x.y',
      label: 'Verify · tick 3',
    });
    render(<DrilledFromBanner />);
    expect(screen.queryByTestId('drilled-from-session')).toBeNull();
  });

  it('omits the breadcrumb strip when there is only one hop', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'sess-x',
      tick: 42,
      elementId: 'pkg.x',
      label: 'Verify · tick 42',
    });
    render(<DrilledFromBanner />);
    expect(screen.queryByTestId('drilled-from-breadcrumb')).toBeNull();
  });

  it('renders earlier hops as a clickable breadcrumb ahead of the current one', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'sess-x',
      tick: 10,
      label: 'Verify · tick 10',
    });
    useInvestigationTrail.getState().push({
      origin: 'run',
      fromSessionId: 'sess-x',
      tick: 20,
      label: 'Run · tick 20',
    });
    render(<DrilledFromBanner />);
    // Current hop (index 1) is the prominent one.
    expect(screen.getByTestId('drilled-from-tick')).toHaveTextContent('tick 20');
    // Earlier hop (index 0) renders as a breadcrumb crumb.
    expect(screen.getByTestId('drilled-from-breadcrumb')).toBeInTheDocument();
    expect(screen.getByTestId('drilled-from-crumb-0')).toHaveTextContent(
      'Verify · tick 10',
    );
  });

  it('clicking an earlier breadcrumb crumb jumps the cursor to it', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'sess-x',
      tick: 10,
      label: 'Verify · tick 10',
    });
    useInvestigationTrail.getState().push({
      origin: 'run',
      fromSessionId: 'sess-x',
      tick: 20,
      label: 'Run · tick 20',
    });
    render(<DrilledFromBanner />);
    fireEvent.click(screen.getByTestId('drilled-from-crumb-0'));
    expect(useInvestigationTrail.getState().cursor).toBe(0);
  });

  it('"Back" steps to the previous hop without discarding it', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'sess-x',
      tick: 10,
      label: 'Verify · tick 10',
    });
    useInvestigationTrail.getState().push({
      origin: 'run',
      fromSessionId: 'sess-x',
      tick: 20,
      label: 'Run · tick 20',
    });
    render(<DrilledFromBanner />);
    fireEvent.click(screen.getByTestId('drilled-from-back'));
    const trail = useInvestigationTrail.getState();
    expect(trail.cursor).toBe(0);
    expect(trail.hops).toHaveLength(2); // forward hop is retained, not discarded
  });

  it('"Back" on a single-hop trail hides the banner (old dismiss behaviour)', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'sess-x',
      tick: 42,
      elementId: 'pkg.x',
      label: 'Verify · tick 42',
    });
    render(<DrilledFromBanner />);
    fireEvent.click(screen.getByTestId('drilled-from-back'));
    expect(useInvestigationTrail.getState().cursor).toBe(-1);
  });
});
