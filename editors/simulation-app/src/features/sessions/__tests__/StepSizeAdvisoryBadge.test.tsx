/**
 * StepSizeAdvisoryBadge — surfaces the runtime step-size under-resolution
 * advisory (P1 dt-under-resolution arc).
 *
 * Pins the pure `extractUnderResolved` parser (which reads the backend's
 * tagged `step_size_health` array off a raw snapshot) plus the render
 * show/hide behaviour.
 */

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import {
  StepSizeAdvisoryBadge,
  extractUnderResolved,
} from '../StepSizeAdvisoryBadge';

afterEach(() => {
  cleanup();
});

describe('extractUnderResolved', () => {
  it('returns [] for a null / shapeless snapshot', () => {
    expect(extractUnderResolved(null)).toEqual([]);
    expect(extractUnderResolved(undefined)).toEqual([]);
    expect(extractUnderResolved({})).toEqual([]);
    expect(extractUnderResolved({ step_size_health: 'nope' })).toEqual([]);
  });

  it('ignores ok and not_applicable advisories', () => {
    const snap = {
      step_size_health: [
        { subsystem: 'A', advisory: { kind: 'ok', ticks_per_cycle: 40 } },
        { subsystem: 'B', advisory: { kind: 'not_applicable' } },
      ],
    };
    expect(extractUnderResolved(snap)).toEqual([]);
  });

  it('extracts under_resolved entries with ticks/cycle and suggested dt', () => {
    const snap = {
      step_size_health: [
        {
          subsystem: 'hybrid',
          advisory: {
            kind: 'under_resolved',
            ticks_per_cycle: 6,
            suggested_dt_ms: 0.3,
          },
        },
        { subsystem: 'Other', advisory: { kind: 'ok', ticks_per_cycle: 25 } },
      ],
    };
    expect(extractUnderResolved(snap)).toEqual([
      { subsystem: 'hybrid', ticksPerCycle: 6, suggestedDtMs: 0.3 },
    ]);
  });
});

describe('StepSizeAdvisoryBadge — render', () => {
  it('renders nothing when no subsystem is under-resolved', () => {
    const snap = {
      step_size_health: [
        { subsystem: 'A', advisory: { kind: 'ok', ticks_per_cycle: 40 } },
      ],
    };
    render(<StepSizeAdvisoryBadge snapshot={snap} />);
    expect(screen.queryByTestId('step-size-advisory')).toBeNull();
  });

  it('renders the worst offender and its suggested dt when under-resolved', () => {
    const snap = {
      step_size_health: [
        {
          subsystem: 'hybrid',
          advisory: {
            kind: 'under_resolved',
            ticks_per_cycle: 6,
            suggested_dt_ms: 0.3,
          },
        },
        {
          subsystem: 'Faster',
          advisory: {
            kind: 'under_resolved',
            ticks_per_cycle: 3,
            suggested_dt_ms: 0.15,
          },
        },
      ],
    };
    render(<StepSizeAdvisoryBadge snapshot={snap} />);
    const badge = screen.getByTestId('step-size-advisory');
    // Worst offender = smallest ticks/cycle (Faster, 3 ticks).
    expect(badge.textContent).toContain('Faster');
    expect(badge.textContent).toContain('3 ticks/cycle');
    // Advisory wording, never alarm vocabulary.
    expect(badge.title).toContain('numerical step-size advisory');
    expect(badge.title.toLowerCase()).not.toContain('error');
    expect(badge.title.toLowerCase()).not.toContain('bug');
  });
});
