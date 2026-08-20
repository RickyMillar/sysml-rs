/**
 * TrailCrumb — renders only when the investigation trail has a hop in
 * view, shows the current hop's label, and "back" steps the cursor to
 * the previous hop without discarding it (ninebar Phase 1 frame chips,
 * mirrors DrilledFromBanner's back semantics).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useInvestigationTrail } from '@/features/investigation/useInvestigationTrail';
import { TrailCrumb } from '../TrailCrumb';

afterEach(() => {
  cleanup();
  useInvestigationTrail.getState().clear();
});

describe('TrailCrumb', () => {
  it('renders nothing when the trail is empty', () => {
    const { container } = render(<TrailCrumb />);
    expect(container.firstChild).toBeNull();
  });

  it('shows the current hop label when the trail has a hop in view', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'sess-x',
      tick: 42,
      label: 'Verify · tick 42',
    });
    render(<TrailCrumb />);
    expect(screen.getByTestId('trail-crumb-label')).toHaveTextContent('Verify · tick 42');
  });

  it('"back" steps the cursor to the previous hop, retaining it (not discarding)', () => {
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
    render(<TrailCrumb />);
    expect(screen.getByTestId('trail-crumb-label')).toHaveTextContent('Run · tick 20');

    fireEvent.click(screen.getByTestId('trail-crumb-back'));

    const trail = useInvestigationTrail.getState();
    expect(trail.cursor).toBe(0);
    expect(trail.hops).toHaveLength(2);
  });

  it('hides again once "back" walks past the first hop', () => {
    useInvestigationTrail.getState().push({
      origin: 'verify',
      fromSessionId: 'sess-x',
      tick: 5,
      label: 'Verify · tick 5',
    });
    render(<TrailCrumb />);
    fireEvent.click(screen.getByTestId('trail-crumb-back'));
    expect(useInvestigationTrail.getState().cursor).toBe(-1);
  });
});
